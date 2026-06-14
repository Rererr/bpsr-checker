#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
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

/// SceneDelta.buff_list の buff_config_id から重複使用無効デバフのキャラを判定。
/// BuffTable.json と実機ログの両方で確認済み。
pub fn classify_buff(buff_config_id: i64) -> BuffSourceKind {
    match buff_config_id {
        2110050 => BuffSourceKind::Basilisk, // バジリスク (実機ログ確認)
        2110055 => BuffSourceKind::Tarta,    // タータ "烈焰焚身" Heart of Flame
        2110056 => BuffSourceKind::Tina,     // ティナ "时间凝滞" Time Acceleration Decree
        2110057 => BuffSourceKind::Aluna,    // アルーナ "祈愿禁止" Blessing of Life
        _ => BuffSourceKind::Other,
    }
}

