//! Mirakurun API 取得用の最小限のHTTPクライアント。
//!
//! 元実装の binzume/http.h(HttpClient)の置き換え。チャンネル一覧取得など、
//! レスポンス全体を読み切ってボディだけを取り出す用途に使う。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// `http://host:port{path}` に GET し、レスポンスボディを返す。
/// 失敗した場合は None。
pub fn get_body(host: &str, port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect((host, port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(30))).ok();

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n",
        path = path,
        host = host,
    );
    stream.write_all(request.as_bytes()).ok()?;

    // Connection: close なのでサーバ側がクローズするまで読み切る。
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;

    // ヘッダとボディの境界 "\r\n\r\n" で分割する。
    let sep = b"\r\n\r\n";
    let pos = find_subslice(&buf, sep)?;
    let body = &buf[pos + sep.len()..];

    Some(String::from_utf8_lossy(body).into_owned())
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
