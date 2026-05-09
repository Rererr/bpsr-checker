pub mod binary_reader;
pub mod server;
pub mod tcp_reassembler;
#[cfg(target_os = "windows")]
pub mod windivert;

use crate::engine::encounter::EncounterMutex;
use crate::engine::processor;
use crate::error::AppResult;
use crate::protocol::opcodes::PktEnvelope;
use log::{info, warn};
use tauri::{AppHandle, Manager};

pub async fn start(app_handle: AppHandle) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    {
        let mut rx = windivert::start_capture();
        process_packets(&app_handle, &mut rx).await;
    }

    #[cfg(not(target_os = "windows"))]
    {
        log::warn!("Packet capture only available on Windows. Running in UI-only mode.");
        let (_tx, mut rx) = tokio::sync::mpsc::channel::<PktEnvelope>(1);
        process_packets(&app_handle, &mut rx).await;
    }

    Ok(())
}

// process_opcode の呼び出しは単一の async タスクで順次行われる。
// タスク内での encounter ロックの取得は排他的に処理されるため、
// conn_to_uid / active_connection の更新に race condition はない。
async fn process_packets(
    app_handle: &AppHandle,
    rx: &mut tokio::sync::mpsc::Receiver<PktEnvelope>,
) {
    while let Some(env) = rx.recv().await {
        // Check if paused
        {
            let state = app_handle.state::<EncounterMutex>();
            if let Ok(encounter) = state.lock() {
                if encounter.is_paused {
                    continue;
                }
            }
        }

        if let Err(e) = processor::process_opcode(app_handle, env) {
            warn!("Error processing packet: {e}");
        }
    }
    info!("Packet receiver closed");
}
