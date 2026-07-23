//! BonDriver_Mirakurun (Rust 実装)
//!
//! TVTest 等から利用される BonDriver (DLLプラグイン)。`CreateBonDriver` を
//! エクスポートし、C++ の IBonDriver2 と同一の vtable レイアウトを持つオブジェクトを
//! 返すことで、既存のホストとABI互換を保つ。
//!
//! 対応アーキテクチャは x64。x86 はメンバ関数が __thiscall 呼び出し規約となり、
//! ここで用いる extern "C" の vtable とは非互換のためビルドを禁止する。

// クレート名(BonDriver_Mirakurun)は意図的に CamelCase のため警告を抑制する。
#![allow(non_snake_case)]

#[cfg(target_arch = "x86")]
compile_error!(
    "32bit(x86)ビルドは __thiscall ABI が必要なため未対応です。x64 でビルドしてください。"
);

mod channels;
mod config;
mod http;
mod rtti;
mod tuner;
mod util;

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, AtomicPtr, Ordering};
use std::sync::{OnceLock, RwLock};

use channels::Channels;
use config::Config;
use tuner::Tuner;

use windows_sys::Win32::Foundation::{BOOL, HINSTANCE, MAX_PATH};
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;

// ---- グローバル状態 ----------------------------------------------------

/// DllMain で読み込んだ設定。
static CONFIG: OnceLock<Config> = OnceLock::new();
/// CreateBonDriver 時に取得するチャンネル情報。
static CHANNELS: RwLock<Option<Channels>> = RwLock::new(None);
/// 多重生成を防ぐためのシングルトン(元実装の m_pThis 相当)。
static INSTANCE: AtomicPtr<BonObject> = AtomicPtr::new(std::ptr::null_mut());
/// DLLのモジュールハンドル(INIパス解決用)。
static HMODULE_VALUE: AtomicIsize = AtomicIsize::new(0);

/// グローバルなチャンネル情報へ読み取りアクセスする。
pub(crate) fn with_channels<R>(f: impl FnOnce(&Channels) -> R) -> R {
    let guard = CHANNELS.read().unwrap();
    match guard.as_ref() {
        Some(c) => f(c),
        None => {
            // 未取得時は空とみなす。
            let empty = Channels::default();
            f(&empty)
        }
    }
}

// ---- IBonDriver2 vtable レイアウト -------------------------------------

/// IBonDriver2 の vtable。並び順は MSVC が実際に生成するスロット順
/// (cl.exe x64 での実測)と完全に一致させること(ABI互換のため)。
///
/// 注意: MSVC は同名オーバーロードの仮想関数を「宣言と逆順」で vtable に
/// 配置する。IBonDriver の GetTsStream は宣言順が (コピー版, ポインタ版) だが、
/// vtable 上は slot6=ポインタ版, slot7=コピー版 になる。
/// また IBonDriver2 末尾で再宣言される Release は基底と同一スロット
/// (slot9)を共有し、追加スロットは作られない(全17スロット)。
#[repr(C)]
struct IBonDriver2Vtbl {
    // IBonDriver
    open_tuner: extern "C" fn(*mut BonObject) -> BOOL,
    close_tuner: extern "C" fn(*mut BonObject),
    set_channel_byte: extern "C" fn(*mut BonObject, u8) -> BOOL,
    get_signal_level: extern "C" fn(*mut BonObject) -> f32,
    wait_ts_stream: extern "C" fn(*mut BonObject, u32) -> u32,
    get_ready_count: extern "C" fn(*mut BonObject) -> u32,
    // slot6: GetTsStream(BYTE**,...) ポインタ版(オーバーロード逆順配置)
    get_ts_stream_ptr:
        extern "C" fn(*mut BonObject, *mut *mut u8, *mut u32, *mut u32) -> BOOL,
    // slot7: GetTsStream(BYTE*,...) コピー版
    get_ts_stream: extern "C" fn(*mut BonObject, *mut u8, *mut u32, *mut u32) -> BOOL,
    purge_ts_stream: extern "C" fn(*mut BonObject),
    release: extern "C" fn(*mut BonObject),
    // IBonDriver2
    get_tuner_name: extern "C" fn(*mut BonObject) -> *const u16,
    is_tuner_opening: extern "C" fn(*mut BonObject) -> BOOL,
    enum_tuning_space: extern "C" fn(*mut BonObject, u32) -> *const u16,
    enum_channel_name: extern "C" fn(*mut BonObject, u32, u32) -> *const u16,
    set_channel_2: extern "C" fn(*mut BonObject, u32, u32) -> BOOL,
    get_cur_space: extern "C" fn(*mut BonObject) -> u32,
    get_cur_channel: extern "C" fn(*mut BonObject) -> u32,
}

/// CreateBonDriver が返すオブジェクト。
/// 先頭に vtable ポインタを置くことで C++ オブジェクトと同一レイアウトになる。
#[repr(C)]
struct BonObject {
    vtable: *const IBonDriver2Vtbl,
    tuner: *mut Tuner,
}

static VTABLE: IBonDriver2Vtbl = IBonDriver2Vtbl {
    open_tuner: thunk_open_tuner,
    close_tuner: thunk_close_tuner,
    set_channel_byte: thunk_set_channel_byte,
    get_signal_level: thunk_get_signal_level,
    wait_ts_stream: thunk_wait_ts_stream,
    get_ready_count: thunk_get_ready_count,
    get_ts_stream_ptr: thunk_get_ts_stream_ptr,
    get_ts_stream: thunk_get_ts_stream,
    purge_ts_stream: thunk_purge_ts_stream,
    release: thunk_release,
    get_tuner_name: thunk_get_tuner_name,
    is_tuner_opening: thunk_is_tuner_opening,
    enum_tuning_space: thunk_enum_tuning_space,
    enum_channel_name: thunk_enum_channel_name,
    set_channel_2: thunk_set_channel_2,
    get_cur_space: thunk_get_cur_space,
    get_cur_channel: thunk_get_cur_channel,
};

/// RTTI 付きブロックへ複製した vtable(TVTest の dynamic_cast 用)。
/// 一度構築したら DLL の寿命中使い回す。
static VTABLE_WITH_RTTI: OnceLock<VtblPtr> = OnceLock::new();

/// 生ポインタを OnceLock に入れるためのラッパ。
/// 参照先は不変データのためスレッド間で共有しても安全。
struct VtblPtr(*const IBonDriver2Vtbl);
unsafe impl Send for VtblPtr {}
unsafe impl Sync for VtblPtr {}

/// dynamic_cast 可能な vtable ポインタを取得する。
fn vtable_ptr() -> *const IBonDriver2Vtbl {
    VTABLE_WITH_RTTI
        .get_or_init(|| {
            let p = unsafe {
                rtti::build_vtable_with_rtti(
                    &VTABLE as *const IBonDriver2Vtbl as *const u8,
                    std::mem::size_of::<IBonDriver2Vtbl>(),
                )
            };
            VtblPtr(p as *const IBonDriver2Vtbl)
        })
        .0
}

// ---- thunk: vtable から Rust メソッドへの橋渡し ------------------------

/// `this` から Tuner の可変参照を得る。
#[inline]
unsafe fn tuner<'a>(this: *mut BonObject) -> &'a mut Tuner {
    unsafe { &mut *(*this).tuner }
}

/// FFI境界をまたぐ panic を遮断するヘルパ。
#[inline]
fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(default)
}

extern "C" fn thunk_open_tuner(this: *mut BonObject) -> BOOL {
    guard(util::FALSE, || unsafe { tuner(this).open_tuner() })
}

extern "C" fn thunk_close_tuner(this: *mut BonObject) {
    guard((), || unsafe { tuner(this).close_tuner() })
}

extern "C" fn thunk_set_channel_byte(this: *mut BonObject, ch: u8) -> BOOL {
    guard(util::FALSE, || unsafe { tuner(this).set_channel_byte(ch) })
}

extern "C" fn thunk_get_signal_level(this: *mut BonObject) -> f32 {
    guard(0.0, || unsafe { tuner(this).get_signal_level() })
}

extern "C" fn thunk_wait_ts_stream(this: *mut BonObject, timeout: u32) -> u32 {
    guard(util::WAIT_ABANDONED, || unsafe {
        tuner(this).wait_ts_stream(timeout)
    })
}

extern "C" fn thunk_get_ready_count(this: *mut BonObject) -> u32 {
    guard(0, || unsafe { tuner(this).get_ready_count() })
}

extern "C" fn thunk_get_ts_stream(
    this: *mut BonObject,
    pdst: *mut u8,
    psize: *mut u32,
    premain: *mut u32,
) -> BOOL {
    guard(util::FALSE, || unsafe {
        tuner(this).get_ts_stream_copy(pdst, psize, premain)
    })
}

extern "C" fn thunk_get_ts_stream_ptr(
    this: *mut BonObject,
    ppdst: *mut *mut u8,
    psize: *mut u32,
    premain: *mut u32,
) -> BOOL {
    guard(util::FALSE, || unsafe {
        tuner(this).get_ts_stream_ptr(ppdst, psize, premain)
    })
}

extern "C" fn thunk_purge_ts_stream(this: *mut BonObject) {
    guard((), || unsafe { tuner(this).purge_ts_stream() })
}

extern "C" fn thunk_release(this: *mut BonObject) {
    guard((), || release_instance(this))
}

extern "C" fn thunk_get_tuner_name(this: *mut BonObject) -> *const u16 {
    guard(std::ptr::null(), || unsafe { tuner(this).tuner_name_ptr() })
}

extern "C" fn thunk_is_tuner_opening(this: *mut BonObject) -> BOOL {
    guard(util::FALSE, || unsafe { tuner(this).is_tuner_opening() })
}

extern "C" fn thunk_enum_tuning_space(this: *mut BonObject, space: u32) -> *const u16 {
    guard(std::ptr::null(), || unsafe {
        tuner(this).enum_tuning_space(space)
    })
}

extern "C" fn thunk_enum_channel_name(
    this: *mut BonObject,
    space: u32,
    channel: u32,
) -> *const u16 {
    guard(std::ptr::null(), || unsafe {
        tuner(this).enum_channel_name(space, channel)
    })
}

extern "C" fn thunk_set_channel_2(this: *mut BonObject, space: u32, channel: u32) -> BOOL {
    guard(util::FALSE, || unsafe {
        tuner(this).set_channel(space, channel)
    })
}

extern "C" fn thunk_get_cur_space(this: *mut BonObject) -> u32 {
    guard(0, || unsafe { tuner(this).get_cur_space() })
}

extern "C" fn thunk_get_cur_channel(this: *mut BonObject) -> u32 {
    guard(0, || unsafe { tuner(this).get_cur_channel() })
}

/// インスタンス開放。Box を破棄し、シングルトンをクリアする。
fn release_instance(this: *mut BonObject) {
    if this.is_null() {
        return;
    }
    INSTANCE
        .compare_exchange(this, std::ptr::null_mut(), Ordering::SeqCst, Ordering::SeqCst)
        .ok();
    unsafe {
        let obj = Box::from_raw(this);
        // Tuner の Drop で close_tuner が呼ばれる。
        drop(Box::from_raw(obj.tuner));
    }
}

// ---- エクスポート関数 --------------------------------------------------

/// BonDriver インスタンスを生成する。既に存在する場合は同じものを返す。
#[unsafe(no_mangle)]
pub extern "C" fn CreateBonDriver() -> *mut c_void {
    guard(std::ptr::null_mut(), || {
        let existing = INSTANCE.load(Ordering::SeqCst);
        if !existing.is_null() {
            return existing as *mut c_void;
        }

        // チャンネル情報を取得する(元実装の InitChannel 相当)。
        if let Some(cfg) = CONFIG.get() {
            let ch = Channels::fetch(cfg);
            if let Ok(mut guard) = CHANNELS.write() {
                *guard = Some(ch);
            }
        }

        let tuner = Box::into_raw(Box::new(Tuner::new()));
        let obj = Box::into_raw(Box::new(BonObject {
            vtable: vtable_ptr(),
            tuner,
        }));
        INSTANCE.store(obj, Ordering::SeqCst);
        obj as *mut c_void
    })
}

// ---- DLL エントリポイント ----------------------------------------------

const DLL_PROCESS_ATTACH: u32 = 1;
const DLL_PROCESS_DETACH: u32 = 0;

#[unsafe(no_mangle)]
pub extern "system" fn DllMain(
    hinst: HINSTANCE,
    reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    match reason {
        DLL_PROCESS_ATTACH => {
            HMODULE_VALUE.store(hinst as isize, Ordering::SeqCst);
            // INIを読み込む。失敗したらロードを拒否する(元実装と同じ)。
            match load_config(hinst) {
                Some(cfg) => {
                    let _ = CONFIG.set(cfg);
                    util::TRUE
                }
                None => util::FALSE,
            }
        }
        DLL_PROCESS_DETACH => {
            // 未開放インスタンスがあれば開放する。
            let inst = INSTANCE.load(Ordering::SeqCst);
            if !inst.is_null() {
                release_instance(inst);
            }
            util::TRUE
        }
        _ => util::TRUE,
    }
}

/// DLLと同じ場所にある `<dll名>.ini` を読み込む。
fn load_config(hinst: HINSTANCE) -> Option<Config> {
    let mut buf = vec![0u16; MAX_PATH as usize];
    let len = unsafe { GetModuleFileNameW(hinst as _, buf.as_mut_ptr(), buf.len() as u32) };
    if len == 0 {
        return None;
    }
    let dll_path = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
    let ini_path = dll_path.with_extension("ini");
    Config::load(&ini_path)
}
