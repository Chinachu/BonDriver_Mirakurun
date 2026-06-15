//! Mirakurun のチャンネル一覧の取得とチューニング空間の構築。
//!
//! 元実装の InitChannel / GetApiChannels / EnumTuningSpace / EnumChannelName /
//! SetChannel のチャンネル解決部分に相当する。

use crate::config::Config;
use crate::http;
use crate::util::SPACE_NUM;
use serde_json::Value;

/// フラットなチャンネル(またはサービス)1件分。
#[derive(Debug, Clone)]
pub struct ChannelEntry {
    pub type_: String,
    pub channel: String,
    pub name: String,
    pub service_id: Option<u64>,
}

/// チャンネル一覧と、そこから導出したチューニング空間情報。
#[derive(Debug, Default, Clone)]
pub struct Channels {
    /// API から取得したフラットなチャンネル列。
    entries: Vec<ChannelEntry>,
    /// チューニング空間ごとの type 名(g_pType 相当)。
    types: Vec<String>,
    /// 各チューニング空間の先頭インデックス(g_Channel_Base 相当)。
    bases: Vec<usize>,
}

impl Channels {
    /// チャンネル情報が空(取得失敗)かどうか。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 最大のチューニング空間インデックス(g_Max_Type 相当)。
    fn max_type(&self) -> usize {
        self.types.len().saturating_sub(1)
    }

    /// Mirakurun API からチャンネル一覧を取得して構築する。
    pub fn fetch(cfg: &Config) -> Channels {
        let path = if cfg.service_split == 1 {
            "/api/services"
        } else {
            "/api/channels"
        };

        let body = match http::get_body(&cfg.server_host, cfg.port_num(), path) {
            Some(b) => b,
            None => return Channels::default(),
        };

        let value: Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(_) => return Channels::default(),
        };

        let array = match value.as_array() {
            Some(a) if !a.is_empty() => a,
            _ => return Channels::default(),
        };

        let mut entries = Vec::with_capacity(array.len());
        for item in array {
            let entry = if cfg.service_split == 1 {
                let ch = &item["channel"];
                ChannelEntry {
                    type_: ch["type"].as_str().unwrap_or("").to_string(),
                    channel: ch["channel"].as_str().unwrap_or("").to_string(),
                    name: item["name"].as_str().unwrap_or("").to_string(),
                    service_id: item["serviceId"].as_u64(),
                }
            } else {
                ChannelEntry {
                    type_: item["type"].as_str().unwrap_or("").to_string(),
                    channel: item["channel"].as_str().unwrap_or("").to_string(),
                    name: item["name"].as_str().unwrap_or("").to_string(),
                    service_id: None,
                }
            };
            entries.push(entry);
        }

        let (types, bases) = build_spaces(&entries);
        Channels {
            entries,
            types,
            bases,
        }
    }

    /// 使用可能なチューニング空間名を返す(EnumTuningSpace 相当)。
    pub fn enum_tuning_space(&self, space: u32) -> Option<&str> {
        self.types.get(space as usize).map(|s| s.as_str())
    }

    /// 指定空間・チャンネルの名前を返す(EnumChannelName 相当)。
    pub fn enum_channel_name(&self, space: u32, channel: u32) -> Option<&str> {
        let idx = self.resolve_index(space, channel)?;
        self.entries.get(idx).map(|e| e.name.as_str())
    }

    /// 指定空間・チャンネルのエントリを返す(SetChannel 用)。
    pub fn entry(&self, space: u32, channel: u32) -> Option<&ChannelEntry> {
        let idx = self.resolve_index(space, channel)?;
        self.entries.get(idx)
    }

    /// (空間, 相対チャンネル) からフラットインデックスを解決する。
    fn resolve_index(&self, space: u32, channel: u32) -> Option<usize> {
        let space = space as usize;
        if space >= self.types.len() {
            return None;
        }
        // 最後の空間以外は、相対チャンネルが空間の幅を超えていないか確認する。
        if space < self.max_type() {
            let width = self.bases[space + 1] - self.bases[space];
            if channel as usize >= width {
                return None;
            }
        }
        let idx = channel as usize + self.bases[space];
        if idx >= self.entries.len() {
            None
        } else {
            Some(idx)
        }
    }
}

/// フラットなチャンネル列を type ごとにグループ化し、
/// チューニング空間の type 名と先頭インデックスを構築する。
///
/// 元実装と同様、エントリは type ごとにまとまっている前提で、
/// type が切り替わった位置を空間の境界とみなす。空間数は SPACE_NUM-1 まで。
fn build_spaces(entries: &[ChannelEntry]) -> (Vec<String>, Vec<usize>) {
    let mut types: Vec<String> = Vec::new();
    let mut bases: Vec<usize> = Vec::new();

    for (i, e) in entries.iter().enumerate() {
        match types.last() {
            None => {
                types.push(e.type_.clone());
                bases.push(0);
            }
            Some(last) if *last != e.type_ => {
                if types.len() >= SPACE_NUM - 1 {
                    break;
                }
                types.push(e.type_.clone());
                bases.push(i);
            }
            _ => {}
        }
    }

    (types, bases)
}
