//! プロトコル棚卸し用の調査ログ（環境変数 `BPSR_PROBE` で有効化。無効時はゼロコスト）。
//! 集計ロジックには一切影響しない。結果の分析・知見は docs-private/protocol/ に永続化する。
//!
//! # モードの選び方
//!
//! ログは起動ごとの truncate だけでローテーションを持たないため、**用途に足りる最小のモードを
//! 選ぶこと**。
//!
//! - `BPSR_PROBE=summary` … 集計カウンタとエンカウンター終了時のサマリー行のみ。ログはほぼ
//!   増えない。**DPS の突合など「数値の内訳」を見たいだけならこれで足りる**
//! - `BPSR_PROBE=1`（`full` も可） … 上記に加えてプロトコルの全ダンプ。1計測で数MB、長い
//!   セッションでは数十MB規模になる。**プロトコルの実態調査そのものが目的のときだけ使う**
//!
//! full で記録される内容:
//! - 全 notify メソッド（既知/未知を問わず。service, method, ペイロード長）
//! - パケットの protobuf トップレベルフィールド構造（field 番号・wire type・長さ）
//! - エンティティ attr の全量ダンプ（attr id と値。既知アームで decode しない id も含む）
//! - buff のスナップショット/tick/変更通知の生データ

use crate::protocol::pb;
use log::info;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// probe の詳細度。環境変数 `BPSR_PROBE` の値で決まる。
///
/// ログはローテーションを持たない（起動ごとの truncate のみ）ため、全ダンプを常用すると
/// 1セッションで数十MBに達する。集計カウンタだけが欲しい調査では `summary` を使うこと。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProbeLevel {
    /// 無効（既定）。
    Off,
    /// 集計カウンタとエンカウンター終了時のサマリー行のみ。ログはほとんど増えない。
    Summary,
    /// 上記に加えてプロトコルの全ダンプ（パケット・attr・buff）。**ログが数十MB規模に膨らむ**。
    Full,
}

static LEVEL: LazyLock<ProbeLevel> =
    LazyLock::new(|| match std::env::var("BPSR_PROBE").as_deref() {
        Ok("1") | Ok("full") => ProbeLevel::Full,
        Ok("summary") => ProbeLevel::Summary,
        _ => ProbeLevel::Off,
    });

/// 集計カウンタを取るか（`summary` / `full` の両方で true）。
pub fn enabled() -> bool {
    *LEVEL != ProbeLevel::Off
}

/// プロトコルの全ダンプを出すか（`full` のみ）。**per-record でログI/Oが走る**ため、
/// カウンタだけで足りる調査ではこれを外した `summary` を使う（ログ肥大と、書き込み負荷が
/// キャプチャスレッドを止めて取りこぼしを増やすのを避けるため）。
pub fn dump_enabled() -> bool {
    *LEVEL == ProbeLevel::Full
}

pub fn log_client_tcp(conn: &crate::capture::server::Server, seq: u32, data: &[u8]) {
    if !dump_enabled() || data.is_empty() {
        return;
    }
    info!(
        "PROBE client-tcp: conn={conn} seq={seq} len={} raw={}",
        data.len(),
        full_hex(data)
    );
}

pub fn log_client_frame(conn: &crate::capture::server::Server, data: &[u8]) {
    if !dump_enabled() {
        return;
    }
    info!(
        "PROBE client-frame: conn={conn} len={} raw={}",
        data.len(),
        full_hex(data)
    );
}

pub fn log_server_tcp(conn: &crate::capture::server::Server, seq: u32, data: &[u8]) {
    if !dump_enabled() || data.is_empty() {
        return;
    }
    info!(
        "PROBE server-tcp: conn={conn} seq={seq} len={} raw={}",
        data.len(),
        full_hex(data)
    );
}

pub fn log_server_frame(conn: &crate::capture::server::Server, data: &[u8]) {
    if !dump_enabled() {
        return;
    }
    info!(
        "PROBE server-frame: conn={conn} len={} raw={}",
        data.len(),
        full_hex(data)
    );
}

pub fn log_udp(src: &str, src_port: u16, dst: &str, dst_port: u16, data: &[u8]) {
    if !dump_enabled() || data.is_empty() {
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
    if !dump_enabled() {
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
    if !dump_enabled() {
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
    if !dump_enabled() {
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
    if !dump_enabled() {
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
    if !dump_enabled() {
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
    if !dump_enabled() {
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
    if !dump_enabled() {
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
/// M3: 非Player attacker の内訳ログを出した回数（サンプル上限の判定用）
static NON_PLAYER_ATTACKER_LOGGED: AtomicU64 = AtomicU64::new(0);
/// M3: record_non_player_attacker で実際に内訳ログを出す最大件数（理由は上の SAMPLE と同じ）
const NON_PLAYER_ATTACKER_LOG_SAMPLE: u64 = 40;

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

/// M3: attacker が Player 以外のエンティティ種別に積まれたダメージを計上する。
/// `actual_value` は combat_stats::actual_value と同じ「lucky_value優先」の実効値
/// （processor.rs 側で process_stats と同じ計算をして渡す）。
///
/// **この計数は `dmg_stats` 合計と足し合わせて比較する突合用ではない**（内訳の切り分け専用）。
/// processor.rs は attacker が Monster のダメージだけを `dmg_stats` から除外するため、Unknown
/// 種別（召喚）の attacker 分はここでも `dmg_stats` 側でも二重に計上される。合計を突き合わせても
/// 一致しないので、突合ではなく「非Player attacker がどれだけ・どんな内訳で発生しているか」を
/// 個別に確認する用途に限定して使うこと。
///
/// **この計数には性質の異なる2つが混在する**ので、合計値だけで結論を出してはいけない。
/// - 召喚の帰属漏れ（top_summoner_id==0 で attacker が召喚エンティティ自身になり、
///   compute.rs の Player フィルタで DPS 一覧から落ちる）＝自分の火力が減る
/// - モンスター自身の与ダメージ（反撃する木人・敵の攻撃）＝そもそも自分とは無関係
///
/// 2026-08-15 の実機計測では、反撃する木人で 45〜104 件立っていたものが、
/// **反撃しない木人では 0 件**になった。つまり当時観測されていたのは後者（木人の反撃）で、
/// 帰属漏れではなかった。合計だけを見て「帰属漏れが原因」と誤結論した実例がある。
/// 種別を切り分けられるよう、先頭 [`NON_PLAYER_ATTACKER_LOG_SAMPLE`] 件は内訳をログする。
pub fn record_non_player_attacker(actual_value: i64, attacker_uuid: i64, top_summoner_id: i64) {
    if !enabled() {
        return;
    }
    NON_PLAYER_ATTACKER_COUNT.fetch_add(1, Ordering::Relaxed);
    NON_PLAYER_ATTACKER_VALUE.fetch_add(actual_value, Ordering::Relaxed);
    if NON_PLAYER_ATTACKER_LOGGED.fetch_add(1, Ordering::Relaxed) >= NON_PLAYER_ATTACKER_LOG_SAMPLE
    {
        return;
    }
    let kind = pb::EntityKind::from(attacker_uuid);
    info!(
        "PROBE non-player-attacker: attacker_uuid={attacker_uuid} kind={kind:?} \
         top_summoner_id={top_summoner_id} value={actual_value}"
    );
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

/// M7: 召喚エンティティ由来（`top_summoner_id != 0`）のダメージのスキル別内訳。
/// 値は (件数, 実効値合計)。キーは skill_uid（`owner_id`）。
static SUMMON_DAMAGE_BY_SKILL: LazyLock<Mutex<HashMap<i32, (u64, i64)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// M8: 総ダメージ（`encounter.dmg_stats`）へ計上した非Healダメージの対象別内訳。
/// 値は (件数, 実効値合計)。キーは (target の UUID, target の monster_id)。
///
/// キーに `uuid >> 16` ではなく UUID をそのまま使う。上位ビットは種別ごとに独立した連番で
/// プレイヤー・モンスター・召喚体の間で衝突するため、潰すと「同じ対象へのダメージ」を
/// 数えたことにならず、初撃対象ロックのキー設計の根拠に使えない。
static DAMAGE_BY_TARGET: LazyLock<Mutex<HashMap<(i64, Option<u32>), (u64, i64)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// M11: 同じ母集団の攻撃者別内訳。キーは attacker の UUID（`top_summoner_id` で
/// 召喚主へ寄せたあとの値）。自分のみ計測を入れたときに何が落ちるかを測る。
static DAMAGE_BY_ATTACKER: LazyLock<Mutex<HashMap<i64, (u64, i64)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// M7/M8 の内訳ログに並べる最大件数（1行が肥大しないよう上位のみ出す）。
const BREAKDOWN_TOP_N: usize = 12;

// ─── M9〜M14: 計測スコープ（初撃対象ロック / 自分のみ計測）の実測 ─────────────────
//
// 実装前に確定させたいのは次の6点。いずれも集計には影響しない。
//
// - M9  計測時計の起点になったデルタが自分の一撃だったか（`fight-start` 行）
// - M10 初撃対象ロックを入れたら総ダメージがどれだけ落ちるか（`encounter-scope` 行）
// - M11 自分のみ計測を入れたら総ダメージがどれだけ落ちるか（同上・攻撃者別内訳）
// - M12 木人やボスの UUID 種別コードが Monster(64) か Unknown か（`encounter-targets` 行）
// - M13 `DamageRecord.is_dead` が実機で立つか、継続ダメージで多重に立たないか（`is-dead` 行）
// - M14 範囲攻撃が対象ごとに別 SceneDelta で届くか（`delta-batch` 行）

/// M10: 初撃対象ロックのシミュレーション対象（UUID）。0 は未確立。
/// エンカウンターサマリーでリセットする（実装側の `clear_combat_stats` に相当）。
static LOCK_SIM_TARGET: AtomicI64 = AtomicI64::new(0);

/// M10/M11: `encounter.dmg_stats` へ積んだ非Healダメージの母集団と、その部分集合。
/// self / locked / both は total の部分集合で、同じタイミングで加算する
/// （別々の条件で数えると割合が出せなくなる）。
static SCOPE_TOTAL_N: AtomicU64 = AtomicU64::new(0);
static SCOPE_TOTAL_V: AtomicI64 = AtomicI64::new(0);
static SCOPE_SELF_N: AtomicU64 = AtomicU64::new(0);
static SCOPE_SELF_V: AtomicI64 = AtomicI64::new(0);
static SCOPE_LOCKED_N: AtomicU64 = AtomicU64::new(0);
static SCOPE_LOCKED_V: AtomicI64 = AtomicI64::new(0);
static SCOPE_BOTH_N: AtomicU64 = AtomicU64::new(0);
static SCOPE_BOTH_V: AtomicI64 = AtomicI64::new(0);

/// 自キャラ UID が未確定（0）のあいだに処理したぶん。未確定の窓の実害を測る。
static SCOPE_NO_SELF_UID_N: AtomicU64 = AtomicU64::new(0);
static SCOPE_NO_SELF_UID_V: AtomicI64 = AtomicI64::new(0);

/// M13: `is_dead=true` のレコード。件数は全数、明細は先頭のみ出す。
static IS_DEAD_COUNT: AtomicU64 = AtomicU64::new(0);
static IS_DEAD_LOGGED: AtomicU64 = AtomicU64::new(0);
const IS_DEAD_LOG_SAMPLE: u64 = 15;

/// M14: ダメージを含む WorldDeltaBatch の形。
static BATCH_WITH_DAMAGE: AtomicU64 = AtomicU64::new(0);
static BATCH_MULTI_TARGET: AtomicU64 = AtomicU64::new(0);
static BATCH_MAX_TARGETS: AtomicU64 = AtomicU64::new(0);
static BATCH_LOGGED: AtomicU64 = AtomicU64::new(0);
static BATCH_MULTI_LOGGED: AtomicU64 = AtomicU64::new(0);
/// 単一対象のバッチは最初の数件だけ形を見れば足りる。複数対象は範囲攻撃の到来パターンを
/// 確定させる本命なので、別枠で多めに出す。
const BATCH_LOG_SAMPLE: u64 = 5;
const BATCH_MULTI_LOG_SAMPLE: u64 = 15;

/// M7: 召喚体が出したダメージをスキル別に計上する。`actual_value` は
/// combat_stats::actual_value と同じ「lucky_value優先」の実効値。
///
/// [`record_non_player_attacker`] とは**別の観点**であることに注意。あちらは
/// `top_summoner_id` が付かず主人へ寄せ *られなかった* ものを数える（＝帰属漏れの検出）。
/// こちらは `top_summoner_id` で主人へ正しく寄せた上で「元々は召喚体が出した分」を数える。
/// 当アプリはこれを主人の火力として合算するが、ゲーム内の木人計測パネルが召喚体のダメージを
/// 数えていない場合、**この合計がそのまま両者の表示差になる**。
pub fn record_summon_damage(skill_uid: i32, actual_value: i64) {
    if !enabled() {
        return;
    }
    // 中身は計数のみでロック跨ぎの不変条件を持たないため、poison しても値は使える
    // （他スレッドの panic で調査ログが丸ごと死ぬほうが困る）。
    let mut map = SUMMON_DAMAGE_BY_SKILL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let slot = map.entry(skill_uid).or_insert((0, 0));
    slot.0 += 1;
    slot.1 += actual_value;
}

/// M8/M10/M11/M12: `encounter.dmg_stats` へ実際に積んだ非Healダメージを、対象別・攻撃者別・
/// 「初撃対象ロックを入れていたら残ったか」別に計上する（`actual_value` の意味は上と同じ）。
/// 呼び出し位置は dmg_stats への加算と同じ分岐に置くこと。
///
/// 集計そのものには影響しない。ロックは記録の中だけで張り、実際の加算は従来どおり全対象ぶん
/// 行う。したがってこの計測を入れた状態の表示は現行版と一致する。
///
/// `self_uid` は自キャラのプレイヤー UID。0 は未確定を表し、その間はロックを張らない
/// （実装予定の条件と同じ。未確定の窓で無関係な対象へロックが確定するのを防ぐ）。
pub fn record_damage_scope(
    target_uuid: i64,
    target_monster_id: Option<u32>,
    attacker_uuid: i64,
    self_uid: i64,
    actual_value: i64,
) {
    if !enabled() {
        return;
    }
    use crate::protocol::constants::entity as entity_const;

    let self_uuid = if self_uid == 0 {
        0
    } else {
        self_uid << 16 | entity_const::PLAYER_TYPE_CODE
    };
    let from_self = self_uuid != 0 && attacker_uuid == self_uuid;
    let target_code = target_uuid & entity_const::TYPE_MASK as i64;
    let target_is_player = target_code == entity_const::PLAYER_TYPE_CODE;

    // 初撃対象ロックのシミュレーション。実装予定の条件（自分の与ダメージ／対象はプレイヤー
    // 以外／エンカウンターにつき1回）をそのまま再現する。compare_exchange で「最初の1件」を
    // 取り、勝った側だけがログを出す。
    if from_self
        && !target_is_player
        && LOCK_SIM_TARGET
            .compare_exchange(0, target_uuid, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        info!(
            "PROBE lock-sim: locked target_uuid={target_uuid} target_code={target_code} monster_id={target_monster_id:?} attacker_uuid={attacker_uuid}"
        );
    }
    let locked = LOCK_SIM_TARGET.load(Ordering::Relaxed);
    let on_locked = locked != 0 && locked == target_uuid;

    SCOPE_TOTAL_N.fetch_add(1, Ordering::Relaxed);
    SCOPE_TOTAL_V.fetch_add(actual_value, Ordering::Relaxed);
    if from_self {
        SCOPE_SELF_N.fetch_add(1, Ordering::Relaxed);
        SCOPE_SELF_V.fetch_add(actual_value, Ordering::Relaxed);
    }
    if on_locked {
        SCOPE_LOCKED_N.fetch_add(1, Ordering::Relaxed);
        SCOPE_LOCKED_V.fetch_add(actual_value, Ordering::Relaxed);
    }
    if from_self && on_locked {
        SCOPE_BOTH_N.fetch_add(1, Ordering::Relaxed);
        SCOPE_BOTH_V.fetch_add(actual_value, Ordering::Relaxed);
    }
    if self_uid == 0 {
        SCOPE_NO_SELF_UID_N.fetch_add(1, Ordering::Relaxed);
        SCOPE_NO_SELF_UID_V.fetch_add(actual_value, Ordering::Relaxed);
    }

    {
        let mut map = DAMAGE_BY_TARGET
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let slot = map
            .entry((target_uuid, target_monster_id))
            .or_insert((0, 0));
        slot.0 += 1;
        slot.1 += actual_value;
    }
    {
        let mut map = DAMAGE_BY_ATTACKER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let slot = map.entry(attacker_uuid).or_insert((0, 0));
        slot.0 += 1;
        slot.1 += actual_value;
    }
}

/// 内訳マップを「実効値の降順・上位 [`BREAKDOWN_TOP_N`] 件」の1行文字列にする。
/// 打ち切った件数は `+N more` として必ず明示する（黙って切ると「全部これだけ」と誤読される）。
fn format_breakdown<K>(entries: Vec<(K, (u64, i64))>, label: impl Fn(&K) -> String) -> String {
    let mut entries = entries;
    entries.sort_by(|a, b| b.1.1.cmp(&a.1.1));
    let shown: Vec<String> = entries
        .iter()
        .take(BREAKDOWN_TOP_N)
        .map(|(k, (n, v))| format!("{} n={n} value={v}", label(k)))
        .collect();
    let rest = entries.len().saturating_sub(shown.len());
    if rest == 0 {
        format!("[{}]", shown.join(", "))
    } else {
        format!("[{}, +{rest} more]", shown.join(", "))
    }
}

/// M6/M9: 戦闘時計（`time_fight_start_ms`）が起動した瞬間の帰属。
///
/// M6 は「そのデルタが damages を含んでいたか」だけを見ていた。false なら自己バフ・詠唱等の
/// 非ダメージ delta で時計が起動しており、分母が実ダメージ開始より早く進み始めていることを示す。
///
/// M9 はここに「そのダメージが自分のものだったか」を足す。`Pending3Min` からの遷移も同じ
/// ブロックで起きるため、計測ボタンの起点が自分の一撃だったかがこの1行で決まる
/// （`self_damage=false` なら他人の与ダメージや自分の被弾で計測窓が回り始めている）。
#[allow(clippy::too_many_arguments)]
pub fn log_fight_start(
    had_damages: bool,
    self_damage: bool,
    damages_n: usize,
    target_uuid: i64,
    target_monster_id: Option<u32>,
    self_uid: i64,
    pending_measure: bool,
) {
    if !enabled() {
        return;
    }
    let target_code = target_uuid & crate::protocol::constants::entity::TYPE_MASK as i64;
    let target_kind = pb::EntityKind::from(target_uuid);
    info!(
        "PROBE fight-start: had_damages={had_damages} self_damage={self_damage} damages_n={damages_n} \
         target_uuid={target_uuid} target_code={target_code} target_kind={target_kind:?} \
         monster_id={target_monster_id:?} self_uid={self_uid} pending_measure={pending_measure}"
    );
}

/// M13: `DamageRecord.is_dead` が立ったレコード。エンジンはこのフィールドを一度も読んで
/// おらず、実機で立つのか、継続ダメージのティックごとに多重に立つのか、ボスのギミック死亡で
/// 省略されるのかがどれも未検証である。ロック解除の判定に使えるかどうかをここで決める。
pub fn record_is_dead(
    target_uuid: i64,
    target_monster_id: Option<u32>,
    attacker_uuid: i64,
    skill_uid: i32,
    actual_value: i64,
    is_heal: bool,
) {
    if !enabled() {
        return;
    }
    let n = IS_DEAD_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if IS_DEAD_LOGGED.fetch_add(1, Ordering::Relaxed) >= IS_DEAD_LOG_SAMPLE {
        return;
    }
    let target_code = target_uuid & crate::protocol::constants::entity::TYPE_MASK as i64;
    info!(
        "PROBE is-dead: #{n} target_uuid={target_uuid} target_code={target_code} \
         monster_id={target_monster_id:?} attacker_uuid={attacker_uuid} skill={skill_uid} \
         value={actual_value} heal={is_heal}"
    );
}

/// M14: ダメージを含む `WorldDeltaBatch` の形を計上する。
///
/// 1つの `SceneDelta` は `uuid` ひとつ（＝被弾側1体）しか持てないため、範囲攻撃が複数の敵に
/// 当たったなら対象ごとに別のデルタで届くはずである。これは protobuf の構造からの推論であって
/// 実測ではないので、同一バッチ内の distinct 対象数を数えて確かめる。
/// `distinct_targets >= 2` のバッチが観測できれば、初撃対象ロックを `target_key` 1個の比較で
/// 実装してよいと確定する。
pub fn record_delta_batch(deltas: &[pb::SceneDelta]) {
    if !enabled() {
        return;
    }
    let mut with_damage = 0usize;
    let mut damage_records = 0usize;
    let mut targets: Vec<i64> = Vec::new();
    for delta in deltas {
        let n = delta
            .skill_effects
            .as_ref()
            .map_or(0, |effects| effects.damages.len());
        if n == 0 {
            continue;
        }
        with_damage += 1;
        damage_records += n;
        if !targets.contains(&delta.uuid) {
            targets.push(delta.uuid);
        }
    }
    if with_damage == 0 {
        return;
    }

    BATCH_WITH_DAMAGE.fetch_add(1, Ordering::Relaxed);
    BATCH_MAX_TARGETS.fetch_max(targets.len() as u64, Ordering::Relaxed);
    let multi = targets.len() > 1;
    if multi {
        BATCH_MULTI_TARGET.fetch_add(1, Ordering::Relaxed);
    }

    // 複数対象のバッチは本命なので別枠で数える。単一対象は形の確認に数件あれば足りる。
    let should_log = if multi {
        BATCH_MULTI_LOGGED.fetch_add(1, Ordering::Relaxed) < BATCH_MULTI_LOG_SAMPLE
    } else {
        BATCH_LOGGED.fetch_add(1, Ordering::Relaxed) < BATCH_LOG_SAMPLE
    };
    if !should_log {
        return;
    }
    let codes: Vec<i64> = targets
        .iter()
        .map(|uuid| uuid & crate::protocol::constants::entity::TYPE_MASK as i64)
        .collect();
    info!(
        "PROBE delta-batch: deltas={} with_damage={with_damage} damage_records={damage_records} \
         distinct_targets={} targets={targets:?} target_codes={codes:?}",
        deltas.len(),
        targets.len()
    );
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

    // M7: 召喚体由来のダメージ（主人へ合算済み）の内訳。ゲーム内の木人パネルが召喚体を
    // 数えていないなら、この total がそのまま表示差になるはず、という突合に使う。
    let summon: HashMap<i32, (u64, i64)> = {
        let mut map = SUMMON_DAMAGE_BY_SKILL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *map)
    };
    let summon_n: u64 = summon.values().map(|(n, _)| n).sum();
    let summon_v: i64 = summon.values().map(|(_, v)| v).sum();
    let summon_breakdown = format_breakdown(summon.into_iter().collect(), |k| format!("skill={k}"));
    info!("PROBE encounter-summon: total(n={summon_n}, value={summon_v}) by_skill={summon_breakdown}");

    // M8/M12: 総ダメージへ積んだ非Healダメージの対象別内訳。distinct が 1 なら計測対象以外へは
    // 一切飛んでいない＝ターゲットロックは当環境では不要、と判断できる。
    // 種別コード（UUID 下位16bit）別の内訳を同じマップから導く（同じ母集団を二重に数えない）。
    // 64=Monster / 640=Player / それ以外は EntityKind::Unknown。木人が Unknown で来るなら、
    // ロック候補を Monster に限定する実装は当環境で黙って無効化される。
    let targets: HashMap<(i64, Option<u32>), (u64, i64)> = {
        let mut map = DAMAGE_BY_TARGET
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *map)
    };
    let targets_distinct = targets.len();
    let mut by_code: HashMap<i64, (u64, i64)> = HashMap::new();
    for (&(uuid, _), &(n, v)) in &targets {
        let slot = by_code
            .entry(uuid & crate::protocol::constants::entity::TYPE_MASK as i64)
            .or_insert((0, 0));
        slot.0 += n;
        slot.1 += v;
    }
    let code_breakdown = format_breakdown(by_code.into_iter().collect(), |code| {
        format!("code={code}({:?})", pb::EntityKind::from(*code))
    });
    let targets_breakdown = format_breakdown(targets.into_iter().collect(), |(uuid, monster_id)| {
        format!("target_uuid={uuid} monster_id={monster_id:?}")
    });
    info!(
        "PROBE encounter-targets: distinct={targets_distinct} by_code={code_breakdown} by_target={targets_breakdown}"
    );

    // M11: 同じ母集団の攻撃者別内訳。自分のみ計測で何が落ちるかを対象別と対で読む。
    let attackers: HashMap<i64, (u64, i64)> = {
        let mut map = DAMAGE_BY_ATTACKER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *map)
    };
    let attackers_distinct = attackers.len();
    let attackers_breakdown = format_breakdown(attackers.into_iter().collect(), |uuid| {
        format!(
            "attacker_uuid={uuid}({:?})",
            pb::EntityKind::from(*uuid)
        )
    });
    info!(
        "PROBE encounter-attackers: distinct={attackers_distinct} by_attacker={attackers_breakdown}"
    );

    // M10/M11: 二つの絞り込みを入れていたら総ダメージがどう変わっていたか。
    // total が現行の表示値、self / locked / both がそれぞれの機能を入れた場合の値になる。
    let total_n = SCOPE_TOTAL_N.swap(0, Ordering::Relaxed);
    let total_v = SCOPE_TOTAL_V.swap(0, Ordering::Relaxed);
    let self_n = SCOPE_SELF_N.swap(0, Ordering::Relaxed);
    let self_v = SCOPE_SELF_V.swap(0, Ordering::Relaxed);
    let locked_n = SCOPE_LOCKED_N.swap(0, Ordering::Relaxed);
    let locked_v = SCOPE_LOCKED_V.swap(0, Ordering::Relaxed);
    let both_n = SCOPE_BOTH_N.swap(0, Ordering::Relaxed);
    let both_v = SCOPE_BOTH_V.swap(0, Ordering::Relaxed);
    let no_uid_n = SCOPE_NO_SELF_UID_N.swap(0, Ordering::Relaxed);
    let no_uid_v = SCOPE_NO_SELF_UID_V.swap(0, Ordering::Relaxed);
    let lock_target = LOCK_SIM_TARGET.swap(0, Ordering::Relaxed);
    let pct = |v: i64| {
        if total_v == 0 {
            0.0
        } else {
            v as f64 * 100.0 / total_v as f64
        }
    };
    info!(
        "PROBE encounter-scope: lock_sim_target={lock_target} total(n={total_n}, value={total_v}) \
         self(n={self_n}, value={self_v}, {:.1}%) locked(n={locked_n}, value={locked_v}, {:.1}%) \
         both(n={both_n}, value={both_v}, {:.1}%) no_self_uid(n={no_uid_n}, value={no_uid_v})",
        pct(self_v),
        pct(locked_v),
        pct(both_v)
    );

    // M13/M14: is_dead の出現数と、ダメージ入りバッチの形。
    let is_dead_n = IS_DEAD_COUNT.swap(0, Ordering::Relaxed);
    IS_DEAD_LOGGED.store(0, Ordering::Relaxed);
    let batch_n = BATCH_WITH_DAMAGE.swap(0, Ordering::Relaxed);
    let batch_multi = BATCH_MULTI_TARGET.swap(0, Ordering::Relaxed);
    let batch_max = BATCH_MAX_TARGETS.swap(0, Ordering::Relaxed);
    BATCH_LOGGED.store(0, Ordering::Relaxed);
    BATCH_MULTI_LOGGED.store(0, Ordering::Relaxed);
    info!(
        "PROBE encounter-shape: is_dead(n={is_dead_n}) delta_batches(with_damage={batch_n}, \
         multi_target={batch_multi}, max_targets={batch_max})"
    );
}
