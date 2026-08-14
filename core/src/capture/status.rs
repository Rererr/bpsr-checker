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

// ─── DPS過小評価の切り分け用: キャプチャ欠落カウンタ（常時計上・probe非限定）───
// probe とは異なりビルド常時カウントする（Atomic加算のみで実測コスト無視できるため）。
// UI表示は未実装。値は compute::get_capture_status() 経由、または probe::log_and_reset_encounter_summary()
// のサマリーログで読む。

/// MAX_SUBNET_CONNECTIONS 到達で追跡を諦めた**接続の数**（パケット数ではない）。
/// 同一接続からの以後のパケットは重複計上しない（windivert.rs の subnet_cap_rejected 参照）。
/// 上限（MAX_SUBNET_CAP_REJECTED_TRACKED=256接続）を超えた分は計上されない。
pub static SUBNET_CAP_HITS: AtomicU64 = AtomicU64::new(0);
/// TCP 再組立でギャップ検知→再同期が起きた回数（戦闘データ一部欠落を伴う）。
pub static REASSEMBLY_GAPS: AtomicU64 = AtomicU64::new(0);
/// ディスパッチチャネル満杯で処理側へ渡せず破棄したフレーム数。
pub static DROPPED_FRAMES: AtomicU64 = AtomicU64::new(0);

pub fn mark_subnet_cap_hit() {
    SUBNET_CAP_HITS.fetch_add(1, Ordering::Relaxed);
}

pub fn mark_reassembly_gap() {
    REASSEMBLY_GAPS.fetch_add(1, Ordering::Relaxed);
}

pub fn mark_dropped_frame() {
    DROPPED_FRAMES.fetch_add(1, Ordering::Relaxed);
}

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
