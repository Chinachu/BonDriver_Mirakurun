//! INIファイルの読み込み。
//!
//! 元実装では GetPrivateProfileString / GetPrivateProfileInt を使用していたが、
//! ここでは同等の振る舞い(セクション・キーの大文字小文字無視、値の前後の
//! ダブルクォート除去)を持つ簡易パーサで再現する。

use std::collections::HashMap;
use std::path::Path;

/// BonDriver_Mirakurun.ini の設定値。
#[derive(Debug, Clone)]
pub struct Config {
    pub server_host: String,
    pub server_port: String,
    pub decode_b25: i32,
    pub priority: i32,
    pub service_split: i32,
}

impl Config {
    /// 接続に使用するポート番号。パースできない場合は 8888 を返す。
    pub fn port_num(&self) -> u16 {
        self.server_port.parse::<u16>().unwrap_or(8888)
    }

    /// INIファイルを読み込んで Config を生成する。
    /// 読み込みに失敗した場合は None(元実装の Init が -2 を返すケース)。
    pub fn load(ini_path: &Path) -> Option<Config> {
        let text = std::fs::read(ini_path).ok()?;
        // INIはASCII想定。非ASCIIはロッシーに変換する。
        let text = String::from_utf8_lossy(&text);
        let ini = Ini::parse(&text);

        Some(Config {
            server_host: ini.get_string("GLOBAL", "SERVER_HOST", "localhost"),
            server_port: ini.get_string("GLOBAL", "SERVER_PORT", "8888"),
            decode_b25: ini.get_int("GLOBAL", "DECODE_B25", 0),
            priority: ini.get_int("GLOBAL", "PRIORITY", 0),
            service_split: ini.get_int("GLOBAL", "SERVICE_SPLIT", 0),
        })
    }
}

/// 大文字小文字を区別しない簡易INIパーサ。
struct Ini {
    // section(大文字) -> (key(大文字) -> value)
    sections: HashMap<String, HashMap<String, String>>,
}

impl Ini {
    fn parse(text: &str) -> Ini {
        let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut current = String::new();

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                current = line[1..line.len() - 1].trim().to_ascii_uppercase();
                continue;
            }
            if let Some(eq) = line.find('=') {
                let key = line[..eq].trim().to_ascii_uppercase();
                let value = strip_quotes(line[eq + 1..].trim());
                sections
                    .entry(current.clone())
                    .or_default()
                    .insert(key, value.to_string());
            }
        }

        Ini { sections }
    }

    fn get_string(&self, section: &str, key: &str, default: &str) -> String {
        self.sections
            .get(&section.to_ascii_uppercase())
            .and_then(|s| s.get(&key.to_ascii_uppercase()))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }

    fn get_int(&self, section: &str, key: &str, default: i32) -> i32 {
        match self
            .sections
            .get(&section.to_ascii_uppercase())
            .and_then(|s| s.get(&key.to_ascii_uppercase()))
        {
            // 先頭の整数部分のみを解釈する(GetPrivateProfileInt 互換)。
            Some(v) => parse_leading_int(v).unwrap_or(default),
            None => default,
        }
    }
}

/// 前後を囲むダブルクォートを1組だけ除去する。
fn strip_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// 文字列先頭の整数(符号付き)を解釈する。
fn parse_leading_int(s: &str) -> Option<i32> {
    let s = s.trim();
    let mut end = 0;
    let bytes = s.as_bytes();
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    s[..end].parse::<i32>().ok()
}
