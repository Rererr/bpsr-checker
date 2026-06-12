//! パケット観測パイプラインを専用スレッド上の tokio ランタイムで起動する。
//! core::capture::start が共有 EncounterMutex を直接更新し、UI 側は Timer で
//! compute 関数をポーリングして取得する（Tauri 版と同じポーリング方式）。

use bpsr_core::capture::status::{self, STATE_FAILED};
use bpsr_core::engine::encounter::EncounterMutex;
use std::sync::Arc;
use std::thread;

pub fn spawn(enc: Arc<EncounterMutex>) {
    let builder = thread::Builder::new().name("bpsr-capture".into());
    let spawn_result = builder.spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                log::error!("failed to build tokio runtime for capture: {e}");
                status::set_state(STATE_FAILED);
                return;
            }
        };
        rt.block_on(async move {
            if let Err(e) = bpsr_core::capture::start(enc).await {
                log::error!("capture pipeline error: {e}");
                status::set_state(STATE_FAILED);
            }
        });
    });
    if let Err(e) = spawn_result {
        log::error!("failed to spawn capture thread: {e}");
        status::set_state(STATE_FAILED);
    }
}
