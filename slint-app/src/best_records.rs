//! 自己ベスト記録の永続化（settings.rs の読み書きパターンを踏襲）。
//! %APPDATA%\bpsr-checker\best_records.json に保存する。
//! キー=計測時間(秒。丸め)。同じ計測時間設定どうしのみで比較する（時間が違うと不公平なため）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const CURRENT_VERSION: u32 = 1;

/// 1 duration ぶんの自己ベスト記録。
/// 構造体単位で #[serde(default)] を付与し、将来フィールドを追加しても旧ファイルの
/// パースが失敗して記録が無言消失しないようにする（Default 導出済み）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BestRecord {
    pub dps: f64,
    pub total_dmg: f64,
    /// 記録日時（UNIX epoch ミリ秒）。
    pub recorded_at_ms: i64,
}

/// version 付きの記録ファイル全体。
/// 現状 version は将来のフォーマット移行用に書き込むのみで、読み込み時に分岐して使うことは
/// していない（パース自体はできるが値は捨てる）。前方互換の実体はこの構造体単位・
/// BestRecord 構造体単位の #[serde(default)] が担っている。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BestRecords {
    pub version: u32,
    /// キー=計測時間(秒)を文字列化したもの（JSON オブジェクトキーは文字列限定のため）。
    pub records: HashMap<String, BestRecord>,
}

impl Default for BestRecords {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            records: HashMap::new(),
        }
    }
}

/// JSON 文字列からのパース（IO と分離した純粋関数。テストはこちらを直接叩く）。
/// 壊れていればログを出して空扱いにする（落とさない）。
fn parse(s: &str) -> BestRecords {
    match serde_json::from_str::<BestRecords>(s) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("best_records: 壊れたファイルのため空扱いにします: {e}");
            BestRecords::default()
        }
    }
}

fn path() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base)
        .join("bpsr-checker")
        .join("best_records.json")
}

impl BestRecords {
    pub fn load() -> Self {
        match std::fs::read_to_string(path()) {
            Ok(s) => parse(&s),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let p = path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&p, json) {
                    log::warn!("best_records save failed: {e}");
                }
            }
            Err(e) => log::warn!("best_records serialize failed: {e}"),
        }
    }

    /// duration_sec の記録を取得（存在しなければ None）。
    pub fn get(&self, duration_sec: u32) -> Option<&BestRecord> {
        self.records.get(&duration_sec.to_string())
    }

    /// dps が既存記録を上回っていれば更新して true を返す（既存が無ければ常に新記録）。
    pub fn try_update(
        &mut self,
        duration_sec: u32,
        dps: f64,
        total_dmg: f64,
        recorded_at_ms: i64,
    ) -> bool {
        let key = duration_sec.to_string();
        let is_new = match self.records.get(&key) {
            Some(r) => dps > r.dps,
            None => true,
        };
        if is_new {
            self.records.insert(
                key,
                BestRecord {
                    dps,
                    total_dmg,
                    recorded_at_ms,
                },
            );
        }
        is_new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // IO(path())には触れず、文字列のみでラウンドトリップ・破損復帰・更新判定を検証する
    // （実 %APPDATA% を汚染・依存しないための意図的な設計）。

    #[test]
    fn roundtrip_via_json_string() {
        let mut recs = BestRecords::default();
        recs.try_update(180, 12345.6, 999_999.0, 1_700_000_000_000);
        let json = serde_json::to_string_pretty(&recs).unwrap();
        let restored = parse(&json);
        assert_eq!(restored.version, CURRENT_VERSION);
        assert_eq!(restored.get(180).unwrap().dps, 12345.6);
        assert_eq!(restored.get(180).unwrap().total_dmg, 999_999.0);
    }

    #[test]
    fn corrupted_json_falls_back_to_empty() {
        let restored = parse("{ this is not valid json !!");
        assert_eq!(restored.records.len(), 0);
        assert_eq!(restored.version, CURRENT_VERSION);
    }

    #[test]
    fn try_update_only_when_higher() {
        let mut recs = BestRecords::default();
        assert!(recs.try_update(180, 100.0, 1000.0, 1));
        assert!(!recs.try_update(180, 50.0, 1000.0, 2)); // 更新されない(下回る)
        assert_eq!(recs.get(180).unwrap().dps, 100.0);
        assert!(recs.try_update(180, 150.0, 1000.0, 3)); // 更新される(上回る)
        assert_eq!(recs.get(180).unwrap().dps, 150.0);
    }

    #[test]
    fn different_durations_are_independent() {
        let mut recs = BestRecords::default();
        recs.try_update(180, 100.0, 1000.0, 1);
        recs.try_update(300, 50.0, 1000.0, 2);
        assert_eq!(recs.get(180).unwrap().dps, 100.0);
        assert_eq!(recs.get(300).unwrap().dps, 50.0);
        assert!(recs.get(60).is_none());
    }

    // 前方互換の確認: BestRecord に将来フィールドが増えても、それを持たない「旧形式」の
    // JSON が引き続きパースできること（#[serde(default)] が効いていることの回帰防止）。
    #[test]
    fn old_shape_json_without_newer_fields_still_parses() {
        // recorded_at_ms を欠いた(将来 BestRecord に必須フィールド追加が起きた場合を模した)
        // 旧形式の1レコードを与える。
        let json = r#"{
            "version": 1,
            "records": {
                "180": { "dps": 42.0, "total_dmg": 100.0 }
            }
        }"#;
        let restored = parse(json);
        assert_eq!(restored.records.len(), 1);
        assert_eq!(restored.get(180).unwrap().dps, 42.0);
        assert_eq!(restored.get(180).unwrap().recorded_at_ms, 0); // default 値

        // ファイル自体(version含む)がまるごと欠けているケース(初回起動相当)も空扱いで壊れない。
        let restored_empty = parse("{}");
        assert_eq!(restored_empty.records.len(), 0);
    }
}
