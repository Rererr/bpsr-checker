use crate::bridge::models::EncounterSnapshot;
use crate::engine::runtime_settings::HISTORY_LIMIT;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn history() -> &'static Mutex<VecDeque<EncounterSnapshot>> {
    static HISTORY: OnceLock<Mutex<VecDeque<EncounterSnapshot>>> = OnceLock::new();
    HISTORY.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub fn push(mut snapshot: EncounterSnapshot) {
    snapshot.id = NEXT_ID.fetch_add(1, Ordering::Relaxed) as f64;
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
}

pub fn trim_to_limit() {
    let limit = HISTORY_LIMIT.load(Ordering::Relaxed);
    if let Ok(mut g) = history().lock() {
        while g.len() > limit {
            g.pop_front();
        }
    }
}
