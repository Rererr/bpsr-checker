//! プロトコル棚卸し用の調査ログ（`BPSR_PROBE=1` で有効化。通常運用ではゼロコスト）。
//!
//! マップ移動・ログイン・ダンジョン読込などの際に「実際に何が届いているか」を全て
//! `PROBE` プレフィックス付きで通常ログへ記録する。目的はプロトコルの網羅的な実態調査で、
//! 集計ロジックには一切影響しない。結果の分析・知見は docs-private/protocol/ に永続化する。
//!
//! 記録内容:
//! - 全 notify メソッド（既知/未知を問わず。service, method, ペイロード長）
//! - パケットの protobuf トップレベルフィールド構造（field 番号・wire type・長さ）
//! - エンティティ attr の全量ダンプ（attr id と値。既知アームで decode しない id も含む）

use crate::protocol::pb;
use log::info;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

static ENABLED: LazyLock<bool> =
    LazyLock::new(|| std::env::var("BPSR_PROBE").is_ok_and(|v| v == "1"));

pub fn enabled() -> bool {
    *ENABLED
}

pub fn log_client_tcp(conn: &crate::capture::server::Server, seq: u32, data: &[u8]) {
    if !enabled() || data.is_empty() {
        return;
    }
    info!(
        "PROBE client-tcp: conn={conn} seq={seq} len={} raw={}",
        data.len(),
        full_hex(data)
    );
}

pub fn log_client_frame(conn: &crate::capture::server::Server, data: &[u8]) {
    if !enabled() {
        return;
    }
    info!(
        "PROBE client-frame: conn={conn} len={} raw={}",
        data.len(),
        full_hex(data)
    );
}

pub fn log_server_tcp(conn: &crate::capture::server::Server, seq: u32, data: &[u8]) {
    if !enabled() || data.is_empty() {
        return;
    }
    info!(
        "PROBE server-tcp: conn={conn} seq={seq} len={} raw={}",
        data.len(),
        full_hex(data)
    );
}

pub fn log_server_frame(conn: &crate::capture::server::Server, data: &[u8]) {
    if !enabled() {
        return;
    }
    info!(
        "PROBE server-frame: conn={conn} len={} raw={}",
        data.len(),
        full_hex(data)
    );
}

pub fn log_udp(src: &str, src_port: u16, dst: &str, dst_port: u16, data: &[u8]) {
    if !enabled() || data.is_empty() {
        return;
    }
    info!(
        "PROBE game-udp: conn={src}:{src_port} -> {dst}:{dst_port} len={} raw={}",
        data.len(),
        full_hex(data)
    );
}

/// 全 notify メソッドの到達記録（packet_parser から呼ぶ）。mapped=既知 opcode 名（未知は None）。
pub fn log_method(service: u64, method: u32, mapped: Option<&str>, payload_len: usize) {
    if !enabled() {
        return;
    }
    match mapped {
        Some(name) => info!("PROBE method: 0x{method:08x} {name} len={payload_len}"),
        None => info!(
            "PROBE method: 0x{method:08x} UNMAPPED service=0x{service:016x} len={payload_len}"
        ),
    }
}

/// protobuf メッセージのトップレベルフィールド構造をスキャンして記録する（decode 型に
/// 定義されていないフィールドも含めた実態の棚卸し用）。`depth_field` を指定すると、その
/// field 番号の length-delimited 中身を1段だけ再帰スキャンする（例: 0x15 の v_data=1）。
pub fn scan_message(tag: &str, data: &[u8], depth_field: Option<u32>) {
    if !enabled() {
        return;
    }
    let fields = scan_fields(data);
    let summary: Vec<String> = fields
        .iter()
        .map(|f| format!("f{}:{}({}B)", f.number, f.wire_name(), f.len))
        .collect();
    info!("PROBE msg {tag}: {} fields [{}]", fields.len(), summary.join(", "));
    if let Some(target) = depth_field {
        for f in &fields {
            if f.number == target && f.wire_type == 2 {
                let inner = &data[f.payload_start..f.payload_start + f.len];
                let inner_fields = scan_fields(inner);
                let inner_summary: Vec<String> = inner_fields
                    .iter()
                    .map(|g| format!("f{}:{}({}B)", g.number, g.wire_name(), g.len))
                    .collect();
                info!(
                    "PROBE msg {tag}.f{target}: {} fields [{}]",
                    inner_fields.len(),
                    inner_summary.join(", ")
                );
            }
        }
    }
}

/// エンティティ attr の全量ダンプ。値は varint として読めれば数値、読めなければ先頭 hex。
pub fn log_attrs(ctx: &str, uuid: i64, attrs: &[pb::RawAttr]) {
    if !enabled() {
        return;
    }
    let rendered: Vec<String> = attrs
        .iter()
        .map(|a| format!("{}={}", a.id, render_value(&a.raw_data)))
        .collect();
    info!("PROBE attrs [{ctx}]: uuid={uuid} n={} {{{}}}", attrs.len(), rendered.join(", "));
}

fn render_value(data: &[u8]) -> String {
    if data.is_empty() {
        return "-".to_string();
    }
    // 短い raw_data は varint として解釈を試みる（attr 値の大半は bare varint）。
    if data.len() <= 10 {
        let mut cursor = std::io::Cursor::new(data);
        if let Ok(v) = prost::encoding::decode_varint(&mut cursor) {
            if cursor.position() as usize == data.len() {
                return v.to_string();
            }
        }
    }
    // 構造化値も後から完全に再解析できるよう、長さと全バイトの hex を記録する。
    let head: String = data.iter().map(|b| format!("{b:02x}")).collect();
    format!("[{}B]{head}", data.len())
}

struct FieldScan {
    number: u32,
    wire_type: u8,
    /// length-delimited のときは中身の長さ、それ以外は消費バイト数
    len: usize,
    /// length-delimited のときの中身開始オフセット（それ以外は未使用）
    payload_start: usize,
}

impl FieldScan {
    fn wire_name(&self) -> &'static str {
        match self.wire_type {
            0 => "varint",
            1 => "i64",
            2 => "len",
            5 => "i32",
            _ => "?",
        }
    }
}

/// protobuf ワイヤフォーマットのトップレベルフィールドを列挙する（値の解釈はしない）。
/// 壊れた/未知の wire type に当たったらそこで打ち切る（部分結果を返す）。
fn scan_fields(data: &[u8]) -> Vec<FieldScan> {
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(data);
    while (cursor.position() as usize) < data.len() {
        let Ok(key) = prost::encoding::decode_varint(&mut cursor) else {
            break;
        };
        let number = (key >> 3) as u32;
        let wire_type = (key & 0x7) as u8;
        if number == 0 {
            break;
        }
        let start = cursor.position() as usize;
        let (len, payload_start) = match wire_type {
            0 => {
                if prost::encoding::decode_varint(&mut cursor).is_err() {
                    break;
                }
                (cursor.position() as usize - start, start)
            }
            1 => {
                if start + 8 > data.len() {
                    break;
                }
                cursor.set_position((start + 8) as u64);
                (8, start)
            }
            5 => {
                if start + 4 > data.len() {
                    break;
                }
                cursor.set_position((start + 4) as u64);
                (4, start)
            }
            2 => {
                let Ok(l) = prost::encoding::decode_varint(&mut cursor) else {
                    break;
                };
                let ps = cursor.position() as usize;
                let end = ps.saturating_add(l as usize);
                if end > data.len() {
                    break;
                }
                cursor.set_position(end as u64);
                (l as usize, ps)
            }
            _ => break,
        };
        out.push(FieldScan { number, wire_type, len, payload_start });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn scan_fields_enumerates_wire_structure() {
        let msg = pb::SkillLevelInfo { skill_id: 3902, current_level: 30, remodel_level: 5 };
        let bytes = msg.encode_to_vec();
        let fields = scan_fields(&bytes);
        assert_eq!(
            fields.iter().map(|f| (f.number, f.wire_type)).collect::<Vec<_>>(),
            vec![(1, 0), (2, 0), (3, 0)]
        );
    }

    #[test]
    fn render_value_decodes_varint_and_hexes_long_data() {
        assert_eq!(render_value(&[0x8e, 0x1e]), "3854");
        assert_eq!(render_value(&[]), "-");
        let long = vec![0xffu8; 30];
        let rendered = render_value(&long);
        assert!(rendered.starts_with("[30B]ffff"));
        assert_eq!(rendered.matches("ff").count(), 30);
    }
}

fn full_hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn log_buff_snapshot(ctx: &str, raw: &[u8], info: &pb::BuffSnapshot) {
    if !enabled() {
        return;
    }
    let source = info
        .fight_source_info
        .map(|s| format!("{}/{}", s.fight_source_type, s.source_config_id))
        .unwrap_or_else(|| "-".to_string());
    info!(
        "PROBE buff-snapshot [{ctx}]: buff_uuid={} base_id={} level={} host_uuid={} table_uuid={} create_time={} fire_uuid={} layer={} part_id={} count={} duration={} source={} raw=[{}B]{}",
        info.buff_uuid,
        info.base_id,
        info.level,
        info.host_uuid,
        info.table_uuid,
        info.create_time,
        info.fire_uuid,
        info.layer,
        info.part_id,
        info.count,
        info.duration,
        source,
        raw.len(),
        full_hex(raw)
    );
}

pub fn log_buff_tick(ctx: &str, raw: &[u8], tick: &pb::BuffTick) {
    if !enabled() {
        return;
    }
    info!(
        "PROBE buff-tick [{ctx}]: host_uuid={} buff_uuid={} base_id={} duration={} create_time={} layer={} raw=[{}B]{}",
        tick.host_uuid,
        tick.buff_uuid,
        tick.base_id,
        tick.duration,
        tick.create_time,
        tick.layer,
        raw.len(),
        full_hex(raw)
    );
}

pub fn log_buff_event(event: &pb::BuffEvent, payload: Option<&pb::BuffPayload>) {
    if !enabled() {
        return;
    }
    let (buff_type, detail) = payload
        .map(|p| (p.buff_type.to_string(), p.detail_raw.as_slice()))
        .unwrap_or_else(|| ("decode-error".to_string(), &[]));
    info!(
        "PROBE buff-event: event_type={} buff_uuid={} host_uuid={} buff_type={} body=[{}B]{} detail=[{}B]{}",
        event.event_type,
        event.buff_uuid,
        event.host_uuid,
        buff_type,
        event.body_raw.len(),
        full_hex(&event.body_raw),
        detail.len(),
        full_hex(detail)
    );
}

pub fn log_buff_change(ctx: &str, raw: &[u8], change: &pb::BuffChange) {
    if !enabled() {
        return;
    }
    info!(
        "PROBE buff-change [{ctx}]: layer={} duration={} create_time={} raw=[{}B]{}",
        change.layer,
        change.duration,
        change.create_time,
        raw.len(),
        full_hex(raw)
    );
}

// ─── DPS過小評価の原因切り分け用カウンタ(M2/M3/M5/M6) ─────────────────────
// `BPSR_PROBE=1` のときのみ計上する（無効時は各関数の enabled() チェック1回のみでゼロコスト）。
// エンカウンター終了（8秒無通信タイムアウト）のたびに processor.rs が
// log_and_reset_encounter_summary() を呼び、このモジュール限定のカウンタは1行のサマリー
// ログを出したうえでリセットする（次のエンカウンターの数値と混ざらないように）。
// M4（capture::status の常時カウンタ）はこのモジュールが所有しないため読むだけでリセットしない。

/// M2: attacker_uuid が取れず捨てたレコードの件数
static SKIP_NO_ATTACKER_COUNT: AtomicU64 = AtomicU64::new(0);
/// M2: attacker_uuid が取れず捨てたレコードの実効値（lucky_value優先）合計
static SKIP_NO_ATTACKER_VALUE: AtomicI64 = AtomicI64::new(0);
/// M2: skill_uid(owner_id)==0 で捨てたレコードの件数
static SKIP_NO_SKILL_COUNT: AtomicU64 = AtomicU64::new(0);
/// M2: skill_uid(owner_id)==0 で捨てたレコードの実効値（lucky_value優先）合計
static SKIP_NO_SKILL_VALUE: AtomicI64 = AtomicI64::new(0);
/// M3: attacker のエンティティ種別が Player 以外（召喚エンティティ自身など）に積まれた
/// ダメージの件数
static NON_PLAYER_ATTACKER_COUNT: AtomicU64 = AtomicU64::new(0);
/// M3: 同上のダメージの実効値（lucky_value優先）合計
static NON_PLAYER_ATTACKER_VALUE: AtomicI64 = AtomicI64::new(0);
/// M5: value と lucky_value が両方非ゼロで同時出現したレコード数
static LUCKY_VALUE_COLLISION_COUNT: AtomicU64 = AtomicU64::new(0);
/// M5: log_lucky_collision で実際にログ行を出す最大件数（以降はカウンタのみ）。
/// BPSR_PROBE=1 のセッションでは M4(DROPPED_FRAMES) を同時計測するため、per-record で
/// ログI/Oを出し続けると観測対象自体を悪化させてしまう（S4）。
const LUCKY_COLLISION_LOG_SAMPLE: u64 = 50;

/// M2: attacker_uuid 不明で捨てたレコードを計上する。`actual_value` は
/// combat_stats::actual_value と同じ「lucky_value優先」の実効値（processor.rs 側で
/// 計算して渡す。ラッキーヒットは value==0 で来る想定のため、生 value だと欠損量を
/// 過小評価してしまう＝process_stats が採用する値と揃える）。
pub fn record_skip_no_attacker(actual_value: i64) {
    if !enabled() {
        return;
    }
    SKIP_NO_ATTACKER_COUNT.fetch_add(1, Ordering::Relaxed);
    SKIP_NO_ATTACKER_VALUE.fetch_add(actual_value, Ordering::Relaxed);
}

/// M2: skill_uid(owner_id)==0 で捨てたレコードを計上する（`actual_value` の意味は上と同じ）。
pub fn record_skip_no_skill(actual_value: i64) {
    if !enabled() {
        return;
    }
    SKIP_NO_SKILL_COUNT.fetch_add(1, Ordering::Relaxed);
    SKIP_NO_SKILL_VALUE.fetch_add(actual_value, Ordering::Relaxed);
}

/// M3: attacker が Player 以外のエンティティ種別に積まれたダメージを計上する
/// （召喚の帰属漏れ＝top_summoner_id==0 で DPS 一覧から落ちるケース）。
/// `actual_value` は combat_stats::actual_value と同じ「lucky_value優先」の実効値
/// （既存の dmg_stats 合計と揃えて比較できるように processor.rs 側で計算して渡す）。
pub fn record_non_player_attacker(actual_value: i64) {
    if !enabled() {
        return;
    }
    NON_PLAYER_ATTACKER_COUNT.fetch_add(1, Ordering::Relaxed);
    NON_PLAYER_ATTACKER_VALUE.fetch_add(actual_value, Ordering::Relaxed);
}

/// M5: value と lucky_value が両方非ゼロで同時出現したレコードを計上する。
/// カウンタ（サマリー行の `lucky_collision(n=..)`）は全件反映するが、ログ行は
/// 先頭 `LUCKY_COLLISION_LOG_SAMPLE` 件のサンプルのみ出す（S4: per-record I/O抑制）。
pub fn log_lucky_collision(value: i64, lucky_value: i64, hp_lessen_value: i64) {
    if !enabled() {
        return;
    }
    let prev_n = LUCKY_VALUE_COLLISION_COUNT.fetch_add(1, Ordering::Relaxed);
    if prev_n < LUCKY_COLLISION_LOG_SAMPLE {
        info!(
            "PROBE lucky-collision: value={value} lucky_value={lucky_value} hp_lessen_value={hp_lessen_value}"
        );
    }
}

/// M6: 戦闘時計（time_fight_start_ms）が起動した瞬間、そのデルタが damages を含んでいたか。
/// false なら自己バフ・詠唱等の非ダメージ delta で時計が起動しており、分母（経過時間）が
/// 実ダメージ開始より早く進み始めている可能性を示す。
pub fn log_fight_start(had_damages: bool) {
    if !enabled() {
        return;
    }
    info!("PROBE fight-start: had_damages={had_damages}");
}

/// M2/M3/M5 のカウンタと M4（capture::status の常時カウンタ）を1行のサマリーとしてログする。
/// M2/M3/M5 はこのモジュール限定の調査用カウンタなのでログ後にゼロへ戻す
/// （エンカウンター単位で数値を区切って読めるようにするため）。M4 は capture::status が
/// 所有する累計カウンタなのでここではリセットしない。
pub fn log_and_reset_encounter_summary() {
    if !enabled() {
        return;
    }
    let skip_attacker_n = SKIP_NO_ATTACKER_COUNT.swap(0, Ordering::Relaxed);
    let skip_attacker_v = SKIP_NO_ATTACKER_VALUE.swap(0, Ordering::Relaxed);
    let skip_skill_n = SKIP_NO_SKILL_COUNT.swap(0, Ordering::Relaxed);
    let skip_skill_v = SKIP_NO_SKILL_VALUE.swap(0, Ordering::Relaxed);
    let non_player_n = NON_PLAYER_ATTACKER_COUNT.swap(0, Ordering::Relaxed);
    let non_player_v = NON_PLAYER_ATTACKER_VALUE.swap(0, Ordering::Relaxed);
    let lucky_n = LUCKY_VALUE_COLLISION_COUNT.swap(0, Ordering::Relaxed);

    let subnet_cap_hits = crate::capture::status::SUBNET_CAP_HITS.load(Ordering::Relaxed);
    let reassembly_gaps = crate::capture::status::REASSEMBLY_GAPS.load(Ordering::Relaxed);
    let dropped_frames = crate::capture::status::DROPPED_FRAMES.load(Ordering::Relaxed);

    info!(
        "PROBE encounter-summary: skip_no_attacker(n={skip_attacker_n}, value={skip_attacker_v}) skip_no_skill(n={skip_skill_n}, value={skip_skill_v}) non_player_attacker(n={non_player_n}, value={non_player_v}) lucky_collision(n={lucky_n}) capture(subnet_cap_hits={subnet_cap_hits}, reassembly_gaps={reassembly_gaps}, dropped_frames={dropped_frames})"
    );
}
