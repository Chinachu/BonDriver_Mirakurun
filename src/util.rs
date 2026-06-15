//! 共通の定数・ヘルパ。

/// WaitForSingleObject 互換の戻り値（WaitTsStream で使用）。
pub const WAIT_OBJECT_0: u32 = 0x0000_0000;
pub const WAIT_ABANDONED: u32 = 0x0000_0080;
pub const WAIT_TIMEOUT: u32 = 0x0000_0102;

/// Win32 BOOL 互換。
pub const TRUE: i32 = 1;
pub const FALSE: i32 = 0;

/// チューナ名。
pub const TUNER_NAME: &str = "BonDriver_Mirakurun";

/// TSデータのサイズ (188 * 256)。
pub const TSDATASIZE: usize = 48128;

/// チューナ空間の最大数。
pub const SPACE_NUM: usize = 8;

/// 受信バッファに保持するTSチャンク数の上限。
/// 元実装の ASYNCBUFFSIZE = 0x200000 / TSDATASIZE * 2 と同等。
pub const ASYNCBUFFSIZE: usize = (0x0020_0000 / TSDATASIZE) * 2;

/// ビットレート計算間隔(ms)。
pub const BITRATE_CALC_TIME_MS: u128 = 500;

/// MagicPacket送出後にサーバ起動を待つ秒数。
pub const MAGICPACKET_WAIT_SECONDS: u64 = 20;

/// UTF-8文字列を NUL 終端の UTF-16(ワイド文字列) に変換する。
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
