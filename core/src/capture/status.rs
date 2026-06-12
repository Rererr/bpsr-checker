//! パケット観測の状態を UI へ伝える共有カウンタ群。
//! windivert.rs（観測スレッド）が書き、compute::get_capture_status が読む。
//! フィルタは全 TCP を観測するため、「ゲーム通信あり」の判定は
//! ゲームサーバ処理経路を通った mark_game_packet のみで行う。

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const STATE_INIT: u8 = 0;
pub const STATE_RUNNING: u8 = 1;
pub const STATE_FAILED: u8 = 2;

pub static CAPTURE_STATE: AtomicU8 = AtomicU8::new(STATE_INIT);
pub static PACKETS_TOTAL: AtomicU64 = AtomicU64::new(0);
/// 任意の TCP パケットを最後に観測した時刻（unix ms。0=未観測）
pub static LAST_PACKET_UNIX_MS: AtomicU64 = AtomicU64::new(0);
/// ゲームサーバのパケットを最後に処理した時刻（unix ms。0=未観測）
pub static LAST_GAME_PACKET_UNIX_MS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn set_state(state: u8) {
    CAPTURE_STATE.store(state, Ordering::Relaxed);
}

pub fn state() -> u8 {
    CAPTURE_STATE.load(Ordering::Relaxed)
}

/// recv ループで TCP パケットを観測するたびに呼ぶ（Relaxed 2 命令＋時刻取得のみ）。
pub fn mark_packet() {
    PACKETS_TOTAL.fetch_add(1, Ordering::Relaxed);
    LAST_PACKET_UNIX_MS.store(unix_ms(), Ordering::Relaxed);
}

/// ゲームサーバ由来のパケットを処理経路へ渡す直前に呼ぶ。
pub fn mark_game_packet() {
    LAST_GAME_PACKET_UNIX_MS.store(unix_ms(), Ordering::Relaxed);
}

/// 指定時刻からの経過 ms。未観測（0）は None。
pub fn ms_since(unix_ms_value: u64) -> Option<u64> {
    if unix_ms_value == 0 {
        return None;
    }
    Some(unix_ms().saturating_sub(unix_ms_value))
}
