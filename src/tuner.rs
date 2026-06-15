//! チューナ本体(元実装の CBonTuner 相当)。
//!
//! TVTest から呼ばれる各APIの実体。チャンネル選局時に Mirakurun の stream API へ
//! HTTP GET を発行し、バックグラウンドスレッドでTSデータを受信してキューに積む。
//!
//! 元実装は Push/Pop の2スレッド + オーバーラップドI/O だったが、ここでは
//! 1本の受信スレッド + Mutex/Condvar のキューという等価でより素直な構成にしている。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::util::{
    to_wide, ASYNCBUFFSIZE, BITRATE_CALC_TIME_MS, FALSE, TRUE, TSDATASIZE, TUNER_NAME,
    WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use crate::{with_channels, CONFIG};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenMutexW, ReleaseMutex,
};

const MUTEX_ALL_ACCESS: u32 = 0x001F_0001;

/// 受信スレッドとTVTest側スレッドで共有する状態。
struct Shared {
    queue: Mutex<VecDeque<Vec<u8>>>,
    cond: Condvar,
    recv_bytes: AtomicU64,
    running: AtomicBool,
}

/// 1つのストリーム受信セッション。
struct StreamCtx {
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
    /// 受信スレッドの read をブロック解除するための複製ソケット。
    shutdown: TcpStream,
}

/// チューナの状態。
pub struct Tuner {
    open: bool,
    cur_space: u32,
    cur_channel: u32,

    stream: Option<StreamCtx>,
    /// GetTsStream(ポインタ版)で返すバッファの保持先。
    last_buf: Vec<u8>,
    /// EnumTuningSpace / EnumChannelName が返すワイド文字列の保持先。
    wbuf: Vec<u16>,
    /// チューナ名のワイド文字列。
    name_wide: Vec<u16>,

    /// 多重オープン検出用の名前付きミューテックス。
    mutex_handle: HANDLE,

    bitrate: f32,
    last_calc: Instant,
}

// HANDLE(生ポインタ)を含むが、Tuner は TVTest 側スレッドからのみ操作され、
// 受信スレッドへ渡すのは Arc<Shared> のみであるため安全に扱える。
unsafe impl Send for Tuner {}

impl Tuner {
    pub fn new() -> Tuner {
        Tuner {
            open: false,
            cur_space: 0,
            cur_channel: 0xFFFF_FFFF,
            stream: None,
            last_buf: Vec::new(),
            wbuf: Vec::new(),
            name_wide: to_wide(TUNER_NAME),
            mutex_handle: std::ptr::null_mut(),
            bitrate: 0.0,
            last_calc: Instant::now(),
        }
    }

    // ---- IBonDriver ----------------------------------------------------

    pub fn open_tuner(&mut self) -> i32 {
        // チャンネル情報が取得できていなければ失敗。
        if with_channels(|c| c.is_empty()) {
            return FALSE;
        }

        self.open = true;
        TRUE
    }

    pub fn close_tuner(&mut self) {
        // ストリーム停止。
        if let Some(mut ctx) = self.stream.take() {
            ctx.shared.running.store(false, Ordering::SeqCst);
            let _ = ctx.shutdown.shutdown(Shutdown::Both);
            ctx.shared.cond.notify_all();
            if let Some(h) = ctx.handle.take() {
                let _ = h.join();
            }
        }

        // チャンネル初期化。
        self.cur_space = 0;
        self.cur_channel = 0xFFFF_FFFF;

        // ミューテックス開放。
        if !self.mutex_handle.is_null() {
            unsafe {
                ReleaseMutex(self.mutex_handle);
                CloseHandle(self.mutex_handle);
            }
            self.mutex_handle = std::ptr::null_mut();
        }

        self.bitrate = 0.0;
    }

    pub fn set_channel_byte(&mut self, ch: u8) -> i32 {
        // 元実装: SetChannel((DWORD)0, (DWORD)bCh - 13)
        self.set_channel(0, (ch as u32).wrapping_sub(13))
    }

    pub fn get_signal_level(&mut self) -> f32 {
        self.calc_bitrate();
        self.bitrate
    }

    pub fn wait_ts_stream(&mut self, timeout: u32) -> u32 {
        let ctx = match &self.stream {
            Some(c) => c,
            None => return WAIT_ABANDONED,
        };
        if !ctx.shared.running.load(Ordering::SeqCst) {
            return WAIT_ABANDONED;
        }

        let mut guard = ctx.shared.queue.lock().unwrap();
        if guard.is_empty() {
            if timeout == 0 {
                // INFINITE 相当: データが来るかクローズされるまで待つ。
                while guard.is_empty() && ctx.shared.running.load(Ordering::SeqCst) {
                    let (g, _) = ctx
                        .shared
                        .cond
                        .wait_timeout(guard, Duration::from_millis(1000))
                        .unwrap();
                    guard = g;
                }
            } else {
                let (g, _) = ctx
                    .shared
                    .cond
                    .wait_timeout(guard, Duration::from_millis(timeout as u64))
                    .unwrap();
                guard = g;
            }
        }

        if !ctx.shared.running.load(Ordering::SeqCst) {
            return WAIT_ABANDONED;
        }
        if guard.is_empty() {
            WAIT_TIMEOUT
        } else {
            WAIT_OBJECT_0
        }
    }

    pub fn get_ready_count(&self) -> u32 {
        match &self.stream {
            Some(ctx) => ctx.shared.queue.lock().unwrap().len() as u32,
            None => 0,
        }
    }

    /// GetTsStream(ポインタ版)。返したポインタは次回呼び出しまで有効。
    pub fn get_ts_stream_ptr(
        &mut self,
        ppdst: *mut *mut u8,
        psize: *mut u32,
        premain: *mut u32,
    ) -> i32 {
        let ctx = match &self.stream {
            Some(c) => c,
            None => return FALSE,
        };

        let popped = {
            let mut guard = ctx.shared.queue.lock().unwrap();
            let buf = guard.pop_front();
            let remain = guard.len() as u32;
            buf.map(|b| (b, remain))
        };

        unsafe {
            match popped {
                Some((buf, remain)) => {
                    self.last_buf = buf;
                    *ppdst = self.last_buf.as_mut_ptr();
                    *psize = self.last_buf.len() as u32;
                    *premain = remain;
                }
                None => {
                    // データ無し。ホストが *ppdst を参照しても安全なよう null を返す。
                    *ppdst = std::ptr::null_mut();
                    *psize = 0;
                    *premain = 0;
                }
            }
        }
        TRUE
    }

    /// GetTsStream(コピー版)。
    pub fn get_ts_stream_copy(
        &mut self,
        pdst: *mut u8,
        psize: *mut u32,
        premain: *mut u32,
    ) -> i32 {
        let mut src: *mut u8 = std::ptr::null_mut();
        let ret = self.get_ts_stream_ptr(&mut src, psize, premain);
        if ret == TRUE {
            let size = unsafe { *psize } as usize;
            if size > 0 && !src.is_null() && !pdst.is_null() {
                unsafe {
                    std::ptr::copy_nonoverlapping(src, pdst, size);
                }
            }
        }
        ret
    }

    pub fn purge_ts_stream(&mut self) {
        if let Some(ctx) = &self.stream {
            ctx.shared.queue.lock().unwrap().clear();
        }
    }

    // ---- IBonDriver2 ---------------------------------------------------

    pub fn tuner_name_ptr(&self) -> *const u16 {
        self.name_wide.as_ptr()
    }

    pub fn is_tuner_opening(&self) -> i32 {
        let name = to_wide(TUNER_NAME);
        let handle = unsafe { OpenMutexW(MUTEX_ALL_ACCESS, 0, name.as_ptr()) };
        if !handle.is_null() {
            unsafe {
                CloseHandle(handle);
            }
            TRUE
        } else {
            FALSE
        }
    }

    pub fn enum_tuning_space(&mut self, space: u32) -> *const u16 {
        match with_channels(|c| c.enum_tuning_space(space).map(to_wide)) {
            Some(w) => {
                self.wbuf = w;
                self.wbuf.as_ptr()
            }
            None => std::ptr::null(),
        }
    }

    pub fn enum_channel_name(&mut self, space: u32, channel: u32) -> *const u16 {
        match with_channels(|c| c.enum_channel_name(space, channel).map(to_wide)) {
            Some(w) => {
                self.wbuf = w;
                self.wbuf.as_ptr()
            }
            None => std::ptr::null(),
        }
    }

    pub fn set_channel(&mut self, space: u32, channel: u32) -> i32 {
        // 対象チャンネルの情報を解決する。
        let entry = match with_channels(|c| c.entry(space, channel).cloned()) {
            Some(e) => e,
            None => return FALSE,
        };

        let cfg = match CONFIG.get() {
            Some(c) => c,
            None => return FALSE,
        };

        // 一旦クローズ。
        self.close_tuner();

        // 接続先URLとリクエストを構築する。
        let path = if cfg.service_split == 1 {
            let sid = entry.service_id.unwrap_or(0);
            format!(
                "/api/channels/{}/{}/services/{}/stream?decode={}",
                entry.type_, entry.channel, sid, cfg.decode_b25
            )
        } else {
            format!(
                "/api/channels/{}/{}/stream?decode={}",
                entry.type_, entry.channel, cfg.decode_b25
            )
        };
        let request = format!(
            "GET {} HTTP/1.0\r\nX-Mirakurun-Priority: {}\r\n\r\n",
            path, cfg.priority
        );

        // 接続。
        let mut stream = match TcpStream::connect((cfg.server_host.as_str(), cfg.port_num())) {
            Ok(s) => s,
            Err(_) => return FALSE,
        };
        let _ = stream.set_nodelay(true);

        if stream.write_all(request.as_bytes()).is_err() {
            return FALSE;
        }

        // ブロック解除用の複製ソケット。
        let shutdown = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => return FALSE,
        };

        // 受信スレッド起動。
        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            cond: Condvar::new(),
            recv_bytes: AtomicU64::new(0),
            running: AtomicBool::new(true),
        });
        let shared_thread = Arc::clone(&shared);
        let handle = std::thread::spawn(move || reader_loop(stream, shared_thread));

        self.stream = Some(StreamCtx {
            shared,
            handle: Some(handle),
            shutdown,
        });

        // 多重オープン検出用ミューテックス作成。
        let name = to_wide(TUNER_NAME);
        self.mutex_handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };

        // チャンネル情報更新。
        self.cur_space = space;
        self.cur_channel = channel;

        self.purge_ts_stream();
        TRUE
    }

    pub fn get_cur_space(&self) -> u32 {
        self.cur_space
    }

    pub fn get_cur_channel(&self) -> u32 {
        self.cur_channel
    }

    // ---- 内部処理 ------------------------------------------------------

    fn calc_bitrate(&mut self) {
        let span = self.last_calc.elapsed().as_millis();
        if span >= BITRATE_CALC_TIME_MS {
            let bytes = match &self.stream {
                Some(ctx) => ctx.shared.recv_bytes.swap(0, Ordering::SeqCst),
                None => 0,
            };
            // Mbps 換算。
            self.bitrate =
                ((bytes as f64 * 8.0 * 1000.0) / (span as f64 * 1024.0 * 1024.0)) as f32;
            self.last_calc = Instant::now();
        }
    }
}

impl Drop for Tuner {
    fn drop(&mut self) {
        self.close_tuner();
    }
}

/// 受信スレッド本体。ソケットからTSデータを読み続けてキューへ積む。
fn reader_loop(mut stream: TcpStream, shared: Arc<Shared>) {
    let mut buf = vec![0u8; TSDATASIZE];
    while shared.running.load(Ordering::SeqCst) {
        match stream.read(&mut buf) {
            Ok(0) => break, // 切断
            Ok(n) => {
                shared.recv_bytes.fetch_add(n as u64, Ordering::SeqCst);
                let mut guard = shared.queue.lock().unwrap();
                // バッファ上限を超えたら古いものから捨てる。
                while guard.len() >= ASYNCBUFFSIZE {
                    guard.pop_front();
                }
                guard.push_back(buf[..n].to_vec());
                drop(guard);
                shared.cond.notify_one();
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => break,
        }
    }
    shared.running.store(false, Ordering::SeqCst);
    shared.cond.notify_all();
}
