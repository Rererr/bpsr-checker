pub mod binary_reader;
pub mod server;
pub mod status;
pub mod tcp_reassembler;
#[cfg(target_os = "windows")]
pub mod windivert;

use crate::engine::encounter::EncounterMutex;
use crate::engine::processor;
use crate::error::AppResult;
use crate::protocol::opcodes::PktEnvelope;
use log::{info, warn};
use std::sync::Arc;

/// パケット観測パイプラインを開始する。`enc` は UI スレッドと共有する集計状態。
/// 単一の async タスクで順次 `process_opcode` を呼ぶため、ロック取得は排他的で
/// conn_to_uid / active_connection の更新に race condition はない。
pub async fn start(enc: Arc<EncounterMutex>) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    {
        let mut rx = windivert::start_capture();
        process_packets(&enc, &mut rx).await;
    }

    #[cfg(not(target_os = "windows"))]
    {
        log::warn!("Packet capture only available on Windows. Running in UI-only mode.");
        let (_tx, mut rx) = tokio::sync::mpsc::channel::<PktEnvelope>(1);
        process_packets(&enc, &mut rx).await;
    }

    Ok(())
}

async fn process_packets(enc: &EncounterMutex, rx: &mut tokio::sync::mpsc::Receiver<PktEnvelope>) {
    while let Some(env) = rx.recv().await {
        if let Err(e) = processor::process_opcode(enc, env) {
            warn!("Error processing packet: {e}");
        }
    }
    info!("Packet receiver closed");
}
