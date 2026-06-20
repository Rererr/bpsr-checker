//! バフタイマーで追跡するプレイヤーのウォッチリスト（src/stores/watchlist.ts を移植）。
//! excluded で手動削除の巻き戻りを防止。%APPDATA%\bpsr-checker\watchlist.json に保存。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX: usize = 30;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Watchlist {
    #[serde(default)]
    pub watched: Vec<i64>,
    #[serde(default)]
    pub excluded: Vec<i64>,
}

fn path() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base)
        .join("bpsr-checker")
        .join("watchlist.json")
}

impl Watchlist {
    pub fn load() -> Self {
        match std::fs::read_to_string(path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let p = path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            if let Err(e) = std::fs::write(&p, json) {
                log::warn!("watchlist save failed: {e}");
            }
        }
    }

    /// ウォッチボタンの挙動: ウォッチ中なら解除して excluded へ、未追跡なら追加(上限内)。
    pub fn toggle(&mut self, uid: i64) {
        if let Some(pos) = self.watched.iter().position(|&u| u == uid) {
            self.watched.remove(pos);
            if !self.excluded.contains(&uid) {
                self.excluded.push(uid);
            }
        } else if self.watched.len() < MAX {
            self.watched.push(uid);
            self.excluded.retain(|&u| u != uid);
        }
    }

    /// 自キャラを先頭へ自動追加（旧 seedLocalPlayer）。
    /// uid=0・excluded・既追加なら何もしない。変更があれば true。
    pub fn seed_local(&mut self, uid: i64) -> bool {
        if uid == 0 || self.excluded.contains(&uid) || self.watched.contains(&uid) {
            return false;
        }
        if self.watched.len() >= MAX {
            return false;
        }
        self.watched.insert(0, uid);
        true
    }

    /// プレイヤー群を末尾へ一括自動追加（旧 bulkAddPlayers）。
    /// excluded・既追加・上限超過分はスキップ。変更があれば true。
    pub fn bulk_add(&mut self, uids: &[i64]) -> bool {
        let mut changed = false;
        for &uid in uids {
            if self.watched.len() >= MAX {
                break;
            }
            if uid == 0 || self.excluded.contains(&uid) || self.watched.contains(&uid) {
                continue;
            }
            self.watched.push(uid);
            changed = true;
        }
        changed
    }

    /// ウォッチ対象をクリア（エンカウンターリセット時。excluded は手動削除の意思として維持）。
    pub fn clear_watched(&mut self) {
        self.watched.clear();
    }
}
