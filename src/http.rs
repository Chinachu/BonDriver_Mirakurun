//! Mirakurun API 取得用の最小限のHTTPクライアント。
//!
//! 元実装の binzume/http.h(HttpClient)の置き換え。チャンネル一覧取得など、
//! レスポンス全体を読み切ってボディだけを取り出す用途に使う。

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// `http://host:port{path}` に GET し、レスポンスボディを返す。
/// 失敗した場合は None。
pub fn get_body(host: &str, port: u16, path: &str) -> Option<String> {
    // 接続タイムアウトを設けて、到達不能ホストでの長時間ブロックを防ぐ。
    let addr = (host, port).to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(30))).ok();

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n",
        path = path,
        host = host,
    );
    stream.write_all(request.as_bytes()).ok()?;

    // ヘッダ境界 "\r\n\r\n" まで読み進める。
    let sep = b"\r\n\r\n";
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, sep) {
            break pos;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return None, // ヘッダ完結前に切断
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    };

    let header = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let body_start = header_end + sep.len();

    // chunked 転送の場合はチャンク制御を取り除いてボディだけを取り出す。
    if is_chunked(&header) {
        // 残りを読み切ってからデコードする。
        stream.read_to_end(&mut buf).ok()?;
        return Some(decode_chunked(&buf[body_start..]));
    }

    // ボディ長を Content-Length で確定できれば、keep-alive でも待たずに済む。
    if let Some(len) = content_length(&header) {
        while buf.len() - body_start < len {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
        let end = (body_start + len).min(buf.len());
        return Some(String::from_utf8_lossy(&buf[body_start..end]).into_owned());
    }

    // Content-Length が無い場合は Connection: close を期待して読み切る。
    stream.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf[body_start..]).into_owned())
}

/// Transfer-Encoding: chunked が指定されているか(大文字小文字無視)。
fn is_chunked(header: &str) -> bool {
    for line in header.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("transfer-encoding")
                && v.to_ascii_lowercase().contains("chunked")
            {
                return true;
            }
        }
    }
    false
}

/// chunked ボディをデコードして連結したバイト列を文字列で返す。
/// 各チャンクは `<16進長>CRLF<データ>CRLF` の形式で、長さ 0 で終端する。
fn decode_chunked(mut data: &[u8]) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(data.len());
    let crlf = b"\r\n";
    loop {
        let line_end = match find_subslice(data, crlf) {
            Some(p) => p,
            None => break,
        };
        // チャンクサイズ行(拡張子 ";..." は無視する)。
        let size_str = String::from_utf8_lossy(&data[..line_end]);
        let size_hex = size_str.split(';').next().unwrap_or("").trim();
        let size = match usize::from_str_radix(size_hex, 16) {
            Ok(s) => s,
            Err(_) => break,
        };
        let body_pos = line_end + crlf.len();
        if size == 0 {
            break; // 終端チャンク
        }
        let end = body_pos + size;
        if end > data.len() {
            out.extend_from_slice(&data[body_pos..]);
            break;
        }
        out.extend_from_slice(&data[body_pos..end]);
        // データ直後の CRLF を読み飛ばす。
        data = &data[(end + crlf.len()).min(data.len())..];
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// レスポンスヘッダから Content-Length を取り出す(大文字小文字無視)。
fn content_length(header: &str) -> Option<usize> {
    for line in header.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                return v.trim().parse::<usize>().ok();
            }
        }
    }
    None
}

/// haystack 中の needle の開始位置を探す。
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
