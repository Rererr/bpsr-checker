use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Serialize, Deserialize)]
struct SelectedUidFile {
    uid: Option<i64>,
}

#[derive(Debug, Default)]
struct SelectedUidState {
    uid: Option<i64>,
    path: Option<PathBuf>,
}

static SELECTED_UID: OnceLock<RwLock<SelectedUidState>> = OnceLock::new();

fn state() -> &'static RwLock<SelectedUidState> {
    SELECTED_UID.get_or_init(|| RwLock::new(SelectedUidState::default()))
}

pub fn init(path: PathBuf) {
    let Ok(mut guard) = state().write() else {
        warn!("selected_uid: ロック取得失敗 (init)");
        return;
    };
    guard.path = Some(path.clone());

    let Ok(data) = std::fs::read_to_string(&path) else {
        info!(
            "selected_uid: ファイルなし ({})、未設定で起動",
            path.display()
        );
        return;
    };
    let Ok(file) = serde_json::from_str::<SelectedUidFile>(&data) else {
        warn!(
            "selected_uid: パース失敗 ({})、未設定で起動",
            path.display()
        );
        return;
    };
    guard.uid = file.uid;
    info!("selected_uid: 読み込み完了 uid={:?}", guard.uid);
}

pub fn get() -> Option<i64> {
    state().read().ok()?.uid
}

pub fn set(uid: Option<i64>) {
    let Ok(mut guard) = state().write() else {
        warn!("selected_uid: ロック取得失敗 (set)");
        return;
    };
    guard.uid = uid;
    save_locked(&guard);
    info!("selected_uid: 更新 uid={:?}", uid);
}

pub fn flush() {
    let (path, uid) = {
        let Ok(guard) = state().read() else {
            return;
        };
        (guard.path.clone(), guard.uid)
    };
    let Some(path) = path else { return };
    let file = SelectedUidFile { uid };
    let Ok(data) = serde_json::to_string(&file) else {
        warn!("selected_uid: シリアライズ失敗");
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, data) {
        warn!("selected_uid: 保存失敗 ({}): {e}", path.display());
    }
}

fn save_locked(guard: &std::sync::RwLockWriteGuard<SelectedUidState>) {
    let Some(path) = &guard.path else { return };
    let file = SelectedUidFile { uid: guard.uid };
    let Ok(data) = serde_json::to_string(&file) else {
        warn!("selected_uid: シリアライズ失敗");
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(path, data) {
        warn!("selected_uid: 保存失敗 ({}): {e}", path.display());
    }
}

