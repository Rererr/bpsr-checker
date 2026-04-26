pub mod binary_reader;
pub mod server;
pub mod tcp_reassembler;
#[cfg(target_os = "windows")]
pub mod windivert;

use crate::engine::encounter::EncounterMutex;
use crate::engine::processor;
use crate::error::AppResult;
use crate::protocol::opcodes::Pkt;
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
        log::warn!(
            "Packet capture only available on Windows. Running in UI-only mode."
        );
        let (_tx, mut rx) = tokio::sync::mpsc::channel::<(Pkt, Vec<u8>)>(1);
        process_packets(&app_handle, &mut rx).await;
    }

    Ok(())
}

async fn process_packets(
    app_handle: &AppHandle,
    rx: &mut tokio::sync::mpsc::Receiver<(Pkt, Vec<u8>)>,
) {
    while let Some((op, data)) = rx.recv().await {
        // Check if paused
        {
            let state = app_handle.state::<EncounterMutex>();
            if let Ok(encounter) = state.lock() {
                if encounter.is_paused {
                    continue;
                }
            }
        }

        if let Err(e) = processor::process_opcode(app_handle, op, data) {
            warn!("Error processing packet: {e}");
        }
    }
    info!("Packet receiver closed");
}
