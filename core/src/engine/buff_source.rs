#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum BuffSourceKind {
    Tina,
    Aluna,
    Tarta,
    Basilisk,
    Kartgriff,
    Other,
}

impl BuffSourceKind {
    pub fn from_str(s: &str) -> Self {
        match s {
            "Tina" => Self::Tina,
            "Aluna" => Self::Aluna,
            "Tarta" => Self::Tarta,
            "Basilisk" => Self::Basilisk,
            "Kartgriff" => Self::Kartgriff,
            _ => Self::Other,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tina => "Tina",
            Self::Aluna => "Aluna",
            Self::Tarta => "Tarta",
            Self::Basilisk => "Basilisk",
            Self::Kartgriff => "Kartgriff",
            Self::Other => "Other",
        }
    }
}

/// SceneDelta.buff_list の buff_config_id から重複使用無効デバフのキャラを判定。
/// BuffTable.json と実機ログの両方で確認済み。
pub fn classify_buff(buff_config_id: i64) -> BuffSourceKind {
    match buff_config_id {
        2110049 => BuffSourceKind::Kartgriff, // ガトグリフ "机械故障" Superconductor Surge
        2110050 => BuffSourceKind::Basilisk,  // バジリスク (実機ログ確認)
        2110055 => BuffSourceKind::Tarta,     // タータ "烈焰焚身" Heart of Flame
        2110056 => BuffSourceKind::Tina,      // ティナ "时间凝滞" Time Acceleration Decree
        2110057 => BuffSourceKind::Aluna,     // アルーナ "祈愿禁止" Blessing of Life
        _ => BuffSourceKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 奥義の重複使用無効デバフを持つイマジンは5体（BuffTable の
    /// 「効果中は同じ奥義を再付与できない」記述で全件走査して確認）。
    #[test]
    fn all_known_reuse_lock_debuffs_are_classified() {
        for (id, expected) in [
            (2110049, BuffSourceKind::Kartgriff),
            (2110050, BuffSourceKind::Basilisk),
            (2110055, BuffSourceKind::Tarta),
            (2110056, BuffSourceKind::Tina),
            (2110057, BuffSourceKind::Aluna),
        ] {
            assert_eq!(classify_buff(id), expected, "buff_config_id={id}");
        }
        assert_eq!(classify_buff(2110051), BuffSourceKind::Other);
    }

    #[test]
    fn kind_str_round_trips() {
        for kind in [
            BuffSourceKind::Tina,
            BuffSourceKind::Aluna,
            BuffSourceKind::Tarta,
            BuffSourceKind::Basilisk,
            BuffSourceKind::Kartgriff,
            BuffSourceKind::Other,
        ] {
            assert_eq!(BuffSourceKind::from_str(kind.as_str()), kind);
        }
    }
}

