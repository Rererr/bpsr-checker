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
