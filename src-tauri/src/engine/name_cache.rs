use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_AGE_DAYS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CachedPlayer {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub class_id: Option<i32>,
    #[serde(default)]
    pub ability_score: Option<i32>,
    #[serde(default)]
    pub season_level: Option<i32>,
    #[serde(default)]
    pub season_strength: Option<i32>,
    #[serde(default)]
    pub last_seen_ms: u64,
}

#[derive(Default)]
struct NameCache {
    entries: HashMap<i64, CachedPlayer>,
    path: Option<PathBuf>,
}

static CACHE: OnceLock<Mutex<NameCache>> = OnceLock::new();

fn cache() -> &'static Mutex<NameCache> {
    CACHE.get_or_init(|| Mutex::new(NameCache::default()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Initialize the cache with a backing file path. Reads existing entries
/// from disk if the file exists. Skips entries older than MAX_AGE_DAYS.
pub fn init(path: PathBuf) {
    let Ok(mut guard) = cache().lock() else {
        return;
    };
    guard.path = Some(path.clone());

    let Ok(data) = std::fs::read_to_string(&path) else {
        info!("Name cache: no existing file at {}", path.display());
        return;
    };
    let Ok(map) = serde_json::from_str::<HashMap<String, CachedPlayer>>(&data) else {
        warn!(
            "Name cache: failed to parse {}, starting fresh",
            path.display()
        );
        return;
    };

    let cutoff = now_ms().saturating_sub(MAX_AGE_DAYS * 24 * 60 * 60 * 1000);
    for (k, v) in map {
        let Ok(uid) = k.parse::<i64>() else { continue };
        if v.last_seen_ms >= cutoff {
            guard.entries.insert(uid, v);
        }
    }
    info!(
        "Name cache: loaded {} entries from {}",
        guard.entries.len(),
        path.display()
    );
}

pub fn lookup(uid: i64) -> Option<CachedPlayer> {
    cache().lock().ok()?.entries.get(&uid).cloned()
}

/// Record a name/class/score/season update for a player. Persists to disk
/// if any field actually changed. Pass `None` for fields you don't have.
pub fn update(
    uid: i64,
    name: Option<&str>,
    class_id: Option<i32>,
    ability_score: Option<i32>,
    season_level: Option<i32>,
    season_strength: Option<i32>,
) {
    if uid == 0 {
        return;
    }
    let Ok(mut guard) = cache().lock() else {
        return;
    };
    let entry = guard.entries.entry(uid).or_default();

    let mut changed = false;
    if let Some(n) = name {
        if !n.is_empty() && entry.name != n {
            entry.name = n.to_string();
            changed = true;
        }
    }
    if let Some(c) = class_id {
        if c != 0 && entry.class_id != Some(c) {
            entry.class_id = Some(c);
            changed = true;
        }
    }
    if let Some(s) = ability_score {
        if s > 0 && entry.ability_score != Some(s) {
            entry.ability_score = Some(s);
            changed = true;
        }
    }
    if let Some(lv) = season_level {
        if lv > 0 && entry.season_level != Some(lv) {
            entry.season_level = Some(lv);
            changed = true;
        }
    }
    if let Some(st) = season_strength {
        if st > 0 && entry.season_strength != Some(st) {
            entry.season_strength = Some(st);
            changed = true;
        }
    }
    entry.last_seen_ms = now_ms();

    if changed {
        save_locked(&guard);
    }
}

/// Force write the cache to disk. Called on app exit so any pending
/// last_seen_ms updates aren't lost.
pub fn flush() {
    let Ok(guard) = cache().lock() else { return };
    save_locked(&guard);
}

fn save_locked(guard: &MutexGuard<NameCache>) {
    let Some(path) = &guard.path else { return };
    let map: HashMap<String, CachedPlayer> = guard
        .entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let Ok(data) = serde_json::to_string(&map) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(path, data) {
        warn!("Name cache: failed to save to {}: {e}", path.display());
    }
}
