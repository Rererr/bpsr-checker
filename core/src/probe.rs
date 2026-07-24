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
