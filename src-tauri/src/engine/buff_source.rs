use crate::engine::buff_tracker::BuffStateSnapshot;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, specta::Type)]
pub enum BuffSourceKind {
    Tina,
    Aluna,
    Tarta,
    Basilisk,
    Other,
}

impl BuffSourceKind {
    pub fn from_str(s: &str) -> Self {
        match s {
            "Tina" => Self::Tina,
            "Aluna" => Self::Aluna,
            "Tarta" => Self::Tarta,
            "Basilisk" => Self::Basilisk,
            _ => Self::Other,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tina => "Tina",
            Self::Aluna => "Aluna",
            Self::Tarta => "Tarta",
            Self::Basilisk => "Basilisk",
            Self::Other => "Other",
        }
    }
}

/// base_id からどのバトルイマジンのバフか判定する。
/// バジリスクの buff_id は実機ログ収集後に追加予定。
pub fn classify(base_id: i32) -> BuffSourceKind {
    match base_id {
        // ティナ: バフ buff_id / 再使用不可デバフ effect_id(392101=150s)
        30001..=31101 | 140001..=141000 | 5001921 | 392101 => BuffSourceKind::Tina,
        4801 | 8801..=8901 | 35101..=36101 => BuffSourceKind::Tarta,
        15001..=16000 => BuffSourceKind::Aluna,
        _ => BuffSourceKind::Other,
    }
}

/// 同キャラの複数バフから代表（最長残時間）を選んで返す。
pub fn aggregate_by_kind(
    snapshots: &[BuffStateSnapshot],
    _now_ms: u128,
) -> HashMap<BuffSourceKind, BuffStateSnapshot> {
    let mut result: HashMap<BuffSourceKind, BuffStateSnapshot> = HashMap::new();

    for snap in snapshots {
        let kind = classify(snap.base_id);
        if kind == BuffSourceKind::Other {
            continue;
        }

        let entry = result.entry(kind).or_insert_with(|| snap.clone());
        if snap.remaining_ms > entry.remaining_ms {
            *entry = snap.clone();
        }
    }

    result
}
