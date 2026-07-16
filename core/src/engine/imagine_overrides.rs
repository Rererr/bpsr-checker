//! バトルイマジン表示名のユーザー上書き（表示名の調整・IGNOREによる非表示化）。
//! `%APPDATA%\bpsr-checker\imagine_overrides.json` に永続化する。
//! Git DB([`crate::engine::imagine_skills`])の正典名(canonical)をキーに、
//! ローカルの表示名/非表示設定だけを保持する（正典名そのものは変更しない）。

use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// 1件分の上書き設定。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImagineOverride {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub ignored: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OverrideFile {
    #[serde(default)]
    overrides: HashMap<String, ImagineOverride>,
}

#[derive(Default)]
struct OverrideStore {
    overrides: HashMap<String, ImagineOverride>,
    path: Option<PathBuf>,
}

static STORE: OnceLock<Mutex<OverrideStore>> = OnceLock::new();

fn store() -> &'static Mutex<OverrideStore> {
    STORE.get_or_init(|| Mutex::new(OverrideStore::default()))
}

/// `store()` をロックする。汚染（他スレッドのpanic後）時も無警告に諦めず、復旧して警告する。
/// `resolve_display` の `None` は「IGNORE＝非表示」を意味し、write系の no-op は設定変更が
/// 無反応になる（＝ユーザー操作が消える）ため、読み書きとも同じ方針で揃えている。
fn lock_store_warn_on_poison() -> MutexGuard<'static, OverrideStore> {
    store().lock().unwrap_or_else(|e| {
        warn!("Imagine overrides: lock poisoned, recovering");
        e.into_inner()
    })
}

/// バッキングファイルパスを注入して初期化する。既存ファイルがあれば読み込む
/// （`name_cache::init` と同型のパターン）。
pub fn init(path: PathBuf) {
    let Ok(mut guard) = store().lock() else {
        return;
    };
    guard.path = Some(path.clone());

    let Ok(data) = std::fs::read_to_string(&path) else {
        return;
    };
    match serde_json::from_str::<OverrideFile>(&data) {
        Ok(file) => guard.overrides = file.overrides,
        Err(e) => warn!(
            "Imagine overrides: failed to parse {}: {e}, starting fresh",
            path.display()
        ),
    }
}

/// 表示名を設定する。`None`または空文字はクリア（未設定）扱い。
pub fn set_display(canonical: &str, display: Option<String>) {
    let mut guard = lock_store_warn_on_poison();
    let display = display.filter(|s| !s.is_empty());
    let entry = guard.overrides.entry(canonical.to_string()).or_default();
    entry.display_name = display;
    save_locked(&guard);
}

/// IGNORE（非表示）設定を切り替える。
pub fn set_ignored(canonical: &str, ignored: bool) {
    let mut guard = lock_store_warn_on_poison();
    let entry = guard.overrides.entry(canonical.to_string()).or_default();
    entry.ignored = ignored;
    save_locked(&guard);
}

/// `canonical` の上書きを丸ごと削除する（表示名・IGNOREとも既定に戻す）。
pub fn clear(canonical: &str) {
    let mut guard = lock_store_warn_on_poison();
    guard.overrides.remove(canonical);
    save_locked(&guard);
}

/// `canonical` の表示解決。IGNOREなら `None`（＝非表示）、表示名が設定済みならそれ、
/// 未設定なら `canonical` 自身を返す。同型の [`crate::engine::name_cache`] は
/// 「ロック失敗時は静かに諦める」方針だが、この関数の `None` は「IGNORE＝非表示」を意味するため
/// `lock_store_warn_on_poison` で復旧し、ロック汚染を理由に全イマジン表示が消えるのは避ける。
pub fn resolve_display(canonical: &str) -> Option<String> {
    let guard = lock_store_warn_on_poison();
    match guard.overrides.get(canonical) {
        None => Some(canonical.to_string()),
        Some(o) if o.ignored => None,
        Some(o) => match &o.display_name {
            Some(name) if !name.is_empty() => Some(name.clone()),
            _ => Some(canonical.to_string()),
        },
    }
}

/// UIモデル構築用に上書き値そのものを取得する（未設定なら `None`）。
pub fn get(canonical: &str) -> Option<ImagineOverride> {
    let guard = lock_store_warn_on_poison();
    guard.overrides.get(canonical).cloned()
}

fn save_locked(guard: &MutexGuard<OverrideStore>) {
    let Some(path) = &guard.path else { return };
    let file = OverrideFile {
        overrides: guard.overrides.clone(),
    };
    let Ok(data) = serde_json::to_string(&file) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(path, data) {
        warn!(
            "Imagine overrides: failed to save to {}: {e}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // static な STORE を共有するため、他テストと衝突しない専用の合成canonical名を使う。
    // `init` は path をプロセス全体で共有してしまい並列テストで壊れるため、ここでは呼ばない
    // （path未設定＝save_locked が常にno-opのままなのでファイルI/Oは発生しない）。

    #[test]
    fn resolve_display_falls_back_to_canonical_when_unset() {
        let _guard = crate::engine::imagine_test_support::guard();
        let canonical = "__override_test_unset__";
        clear(canonical);
        assert_eq!(resolve_display(canonical), Some(canonical.to_string()));
        assert_eq!(get(canonical), None);
    }

    #[test]
    fn set_display_overrides_resolution_and_empty_string_clears_it() {
        let _guard = crate::engine::imagine_test_support::guard();
        let canonical = "__override_test_display__";
        set_display(canonical, Some("表示名".to_string()));
        assert_eq!(resolve_display(canonical), Some("表示名".to_string()));
        assert_eq!(
            get(canonical).unwrap().display_name,
            Some("表示名".to_string())
        );

        set_display(canonical, Some(String::new()));
        assert_eq!(resolve_display(canonical), Some(canonical.to_string()));

        clear(canonical);
        assert_eq!(get(canonical), None);
    }

    #[test]
    fn set_ignored_hides_resolution_until_cleared() {
        let _guard = crate::engine::imagine_test_support::guard();
        let canonical = "__override_test_ignored__";
        set_ignored(canonical, true);
        assert_eq!(resolve_display(canonical), None);

        set_ignored(canonical, false);
        assert_eq!(resolve_display(canonical), Some(canonical.to_string()));

        clear(canonical);
    }

    #[test]
    fn override_file_schema_round_trips_camel_case_json() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "サンプル".to_string(),
            ImagineOverride {
                display_name: Some("表示".to_string()),
                ignored: true,
            },
        );
        let file = OverrideFile { overrides };
        let json = serde_json::to_string(&file).expect("serialize");
        assert!(json.contains("\"displayName\":\"表示\""));
        assert!(json.contains("\"ignored\":true"));

        let parsed: OverrideFile = serde_json::from_str(&json).expect("deserialize");
        let entry = parsed.overrides.get("サンプル").expect("entry present");
        assert_eq!(entry.display_name, Some("表示".to_string()));
        assert!(entry.ignored);
    }
}
