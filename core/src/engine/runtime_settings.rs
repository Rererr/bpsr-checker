use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};

pub static COMBAT_EXIT_TIMEOUT_MS: AtomicU64 = AtomicU64::new(8000);
pub static HISTORY_LIMIT: AtomicUsize = AtomicUsize::new(20);
pub static TS_INTERVAL_MS: AtomicU64 = AtomicU64::new(1000);
pub static TS_SAMPLES: AtomicUsize = AtomicUsize::new(60);

/// ON のとき DPS / ヒール / スキル / 時系列の集計をすべて省略し、
/// バフ追跡（イマジンデバフタイマー）のみ動作させる軽量モード。
pub static IMAGINE_ONLY_MODE: AtomicBool = AtomicBool::new(false);

pub fn imagine_only_mode() -> bool {
    IMAGINE_ONLY_MODE.load(Ordering::Relaxed)
}

/// ON のとき、ダメージ0＋食事/シロップ持ちの特例行（compute::build_players_window_unsorted
/// 参照）を自分/PTメンバー限定にする。AOI appear 同期で街中の無関係プレイヤー全員が
/// 0ダメージ行として並ぶのを抑える設定。既定 true（settings.rs の Settings::default と揃える）。
pub static PARTY_ONLY_CONSUMABLES: AtomicBool = AtomicBool::new(true);

pub fn party_only_consumables() -> bool {
    PARTY_ONLY_CONSUMABLES.load(Ordering::Relaxed)
}

pub fn set_party_only_consumables(enabled: bool) {
    PARTY_ONLY_CONSUMABLES.store(enabled, Ordering::Relaxed);
}

/// 名前辞書（スキル/モンスター/バフ）の表示言語。起動時に settings.language から一度設定する。
/// UI 文字列(@tr)は Slint 側が別管理だが、起動時に同じ値へ揃える。0=ja / 1=en / 2=zh。
pub static DISPLAY_LANG: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ja = 0,
    En = 1,
    Zh = 2,
}

impl Lang {
    /// settings.language のロケールコードから。未知は ja 既定。
    pub fn from_code(code: &str) -> Lang {
        match code {
            "en" => Lang::En,
            "zh" => Lang::Zh,
            _ => Lang::Ja,
        }
    }
}

pub fn set_display_lang(lang: Lang) {
    DISPLAY_LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn display_lang() -> Lang {
    match DISPLAY_LANG.load(Ordering::Relaxed) {
        1 => Lang::En,
        2 => Lang::Zh,
        _ => Lang::Ja,
    }
}
