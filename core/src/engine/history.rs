//! 戦闘履歴（EncounterSnapshot）のメモリ保持とディスク永続化。
//!
//! consumables.rs / name_cache.rs と同じ init/load/save パターン: main.rs から
//! `%APPDATA%\bpsr-checker\history.json` のパスを `init` で注入する（デモモードでは
//! 呼ばない）。`init` 未呼び出し時は path が None のままとなり、push/clear の保存は
//! no-op になる（EncounterSnapshot は壁時計に依存しないため、consumables のような
//! 期限切れ処理は不要。保存時に HISTORY_LIMIT 件へ切り詰めるのみ）。

use crate::engine::runtime_settings::HISTORY_LIMIT;
use crate::models::EncounterSnapshot;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock, RwLock};

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn history() -> &'static Mutex<VecDeque<EncounterSnapshot>> {
    static HISTORY: OnceLock<Mutex<VecDeque<EncounterSnapshot>>> = OnceLock::new();
    HISTORY.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub fn push(mut snapshot: EncounterSnapshot) {
    snapshot.id = NEXT_ID.fetch_add(1, Ordering::Relaxed) as f64;
    let items = {
        let mut guard = match history().lock() {
            Ok(g) => g,
            Err(e) => {
                log::error!("history::push: lock poisoned: {e}");
                return;
            }
        };
        guard.push_back(snapshot);
        let limit = HISTORY_LIMIT.load(Ordering::Relaxed);
        while guard.len() > limit {
            guard.pop_front();
        }
        guard.iter().cloned().collect::<Vec<_>>()
    };
    save_to_path(current_path().as_deref(), &items);
}

pub fn snapshot_list() -> Vec<EncounterSnapshot> {
    let guard = match history().lock() {
        Ok(g) => g,
        Err(e) => {
            log::error!("history::snapshot_list: lock poisoned: {e}");
            return vec![];
        }
    };
    guard.iter().rev().cloned().collect()
}

pub fn clear() {
    if let Ok(mut g) = history().lock() {
        g.clear();
    }
    save_to_path(current_path().as_deref(), &[]);
}

pub fn trim_to_limit() {
    let limit = HISTORY_LIMIT.load(Ordering::Relaxed);
    if let Ok(mut g) = history().lock() {
        while g.len() > limit {
            g.pop_front();
        }
    }
    // 上限変更時点では保存しない（次回 push 時の全書き出しで反映されれば十分）。
}

// ─── ディスク永続化 ─────────────────────────────────────────────────────────

/// 保存ファイル構造（前方互換のため version 付き。consumables.rs と同形）。
#[derive(Serialize, Deserialize)]
struct HistoryFile {
    version: u32,
    /// 古い→新しい順（VecDeque の内部順と同じ）。
    encounters: Vec<EncounterSnapshot>,
}

const FILE_VERSION: u32 = 1;

static PERSIST_PATH: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

fn persist_path() -> &'static RwLock<Option<PathBuf>> {
    PERSIST_PATH.get_or_init(|| RwLock::new(None))
}

fn current_path() -> Option<PathBuf> {
    persist_path().read().ok().and_then(|g| g.clone())
}

/// 保存先パスを登録し、既存ファイルがあれば HISTORY_LIMIT 件まで読み込んで VecDeque を
/// 初期化する（起動時に1回）。呼ばない場合（デモモード等）は push/clear の保存が no-op になる。
pub fn init(path: PathBuf) {
    {
        let Ok(mut g) = persist_path().write() else {
            warn!("history: ロック取得失敗 (init)");
            return;
        };
        g.replace(path.clone());
    }

    let limit = HISTORY_LIMIT.load(Ordering::Relaxed);
    let loaded = load_from_path(&path, limit);
    let max_id = loaded.iter().map(|s| s.id as u64).max().unwrap_or(0);
    NEXT_ID.store(max_id + 1, Ordering::Relaxed);

    let Ok(mut guard) = history().lock() else {
        warn!("history: ロック取得失敗 (init 反映)");
        return;
    };
    let count = loaded.len();
    *guard = loaded.into();
    info!("history: 読み込み完了 {count} 件 ({})", path.display());
}

/// ファイルを読み込み、古い順に先頭から `limit` 件を超える分を切り詰めて返す。
/// ファイル無し/パース失敗は空 Vec（warn ログ、パニックしない）。
fn load_from_path(path: &Path, limit: usize) -> Vec<EncounterSnapshot> {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => {
            info!("history: ファイルなし ({})、空で起動", path.display());
            return Vec::new();
        }
    };
    let parsed: HistoryFile = match serde_json::from_str(&data) {
        Ok(f) => f,
        Err(e) => {
            warn!("history: パース失敗 ({}): {e}、空で起動", path.display());
            return Vec::new();
        }
    };
    let mut encounters = parsed.encounters;
    if encounters.len() > limit {
        let excess = encounters.len() - limit;
        encounters.drain(0..excess);
    }
    encounters
}

/// `items`（古い→新しい順）を `path` へ書き出す。`path` が `None`（init 未呼び出し）なら no-op。
fn save_to_path(path: Option<&Path>, items: &[EncounterSnapshot]) {
    let Some(path) = path else {
        return; // init 未呼び出し = 永続化しない（デモモード等）
    };
    let file = HistoryFile {
        version: FILE_VERSION,
        encounters: items.to_vec(),
    };
    let json = match serde_json::to_string(&file) {
        Ok(j) => j,
        Err(e) => {
            warn!("history: シリアライズ失敗: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!("history: ディレクトリ作成失敗 ({}): {e}", parent.display());
            return;
        }
    }
    if let Err(e) = std::fs::write(path, json) {
        warn!("history: 保存失敗 ({}): {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("bpsr-history-test-{name}-{nanos}.json"))
    }

    fn snapshot(id: f64) -> EncounterSnapshot {
        EncounterSnapshot {
            id,
            total_dmg: 12345.0,
            participant_player_uids: vec![1.0, 2.0],
            ..Default::default()
        }
    }

    // save_to_path → load_from_path のラウンドトリップで内容が保たれる。
    #[test]
    fn save_then_load_roundtrip() {
        let path = unique_temp_path("roundtrip");
        let items = vec![snapshot(1.0), snapshot(2.0)];
        save_to_path(Some(&path), &items);

        let loaded = load_from_path(&path, 20);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, 1.0);
        assert_eq!(loaded[1].id, 2.0);
        assert_eq!(loaded[1].total_dmg, 12345.0);

        let _ = std::fs::remove_file(&path);
    }

    // limit を超える件数は古い順(先頭)から切り詰められ、新しい方が残る。
    #[test]
    fn load_prunes_to_limit_keeping_newest() {
        let path = unique_temp_path("prune");
        let items: Vec<EncounterSnapshot> = (1..=5).map(|i| snapshot(i as f64)).collect();
        save_to_path(Some(&path), &items);

        let loaded = load_from_path(&path, 2);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, 4.0);
        assert_eq!(loaded[1].id, 5.0);

        let _ = std::fs::remove_file(&path);
    }

    // 壊れた JSON はパニックせず空 Vec で復帰する。
    #[test]
    fn load_recovers_from_corrupt_file() {
        let path = unique_temp_path("corrupt");
        std::fs::write(&path, "not valid json").expect("write corrupt fixture");

        let loaded = load_from_path(&path, 20);
        assert!(loaded.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    // ファイルが存在しない場合も空 Vec（初回起動相当）。
    #[test]
    fn load_missing_file_returns_empty() {
        let path = unique_temp_path("missing");
        let loaded = load_from_path(&path, 20);
        assert!(loaded.is_empty());
    }

    // init 未呼び出し（path=None）相当: save は no-op でパニックしない。
    #[test]
    fn save_is_noop_without_path() {
        save_to_path(None, &[snapshot(1.0)]);
    }

    // init → push → snapshot_list → clear の実配線を通しで確認する。
    // モジュールのグローバル状態（PERSIST_PATH / HISTORY / NEXT_ID）に触れるのはこの
    // テストのみで、他のテストは純粋関数 (load_from_path/save_to_path) しか呼ばないため
    // 並列実行しても競合しない。
    #[test]
    fn init_push_snapshot_clear_end_to_end() {
        let path = unique_temp_path("e2e");
        init(path.clone());
        push(snapshot(100.0));
        push(snapshot(101.0));

        let list = snapshot_list();
        assert_eq!(list.len(), 2);

        let saved = load_from_path(&path, 20);
        assert_eq!(saved.len(), 2);

        clear();
        assert!(snapshot_list().is_empty());
        let saved_after_clear = load_from_path(&path, 20);
        assert!(saved_after_clear.is_empty());

        let _ = std::fs::remove_file(&path);
    }
}
