use crate::capture::binary_reader::BinaryReader;
use crate::capture::server::Server;
use crate::capture::tcp_reassembler::TcpReassembler;
use crate::protocol::opcodes::{Pkt, PktEnvelope};
use crate::protocol::packet_parser::process_packet;
use etherparse::NetSlice::Ipv4;
use etherparse::SlicedPacket;
use etherparse::TransportSlice::Tcp;
use log::{debug, error, info, warn};
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::watch;
use windivert::WinDivert;
use windivert::layer::NetworkLayer;
use windivert::prelude::{WinDivertFlags, WinDivertShutdownMode};

// Global sender for restart signal
static RESTART_SENDER: OnceLock<watch::Sender<bool>> = OnceLock::new();

/// Thread-shareable wrapper around an open WinDivert handle.
///
/// Rust forces `WinDivert::shutdown` to take `&mut self`, but the underlying
/// `WinDivertShutdown` C API is documented as thread-safe and is in fact
/// designed to be called from a different thread than the one blocked in
/// `WinDivertRecv`. We use `UnsafeCell` + `unsafe impl Send/Sync` to bypass
/// the borrow rule and let the exit handler signal recv to abort.
struct SharedDivert {
    inner: UnsafeCell<WinDivert<NetworkLayer>>,
}
unsafe impl Send for SharedDivert {}
unsafe impl Sync for SharedDivert {}

impl SharedDivert {
    fn new(divert: WinDivert<NetworkLayer>) -> Self {
        Self {
            inner: UnsafeCell::new(divert),
        }
    }
    fn recv<'a>(
        &self,
        buffer: Option<&'a mut [u8]>,
    ) -> Result<
        windivert::packet::WinDivertPacket<'a, NetworkLayer>,
        windivert::error::WinDivertError,
    > {
        // SAFETY: WinDivert::recv only reads &self in the underlying impl.
        unsafe { (*self.inner.get()).recv(buffer) }
    }
    fn shutdown(&self, mode: WinDivertShutdownMode) {
        // SAFETY: WinDivertShutdown is thread-safe per the WinDivert C docs;
        // it is intended to be called from a different thread than the one
        // blocking on recv() to abort the pending recv.
        unsafe {
            let _ = (*self.inner.get()).shutdown(mode);
        }
    }
}

// Active capture handle. Set when the recv loop opens WinDivert; cleared
// when the loop exits. The exit handler reads this to abort recv and close
// the handle so `WinDivert::uninstall()` can succeed.
static ACTIVE_DIVERT: OnceLock<Mutex<Option<Arc<SharedDivert>>>> = OnceLock::new();

fn active_divert_slot() -> &'static Mutex<Option<Arc<SharedDivert>>> {
    ACTIVE_DIVERT.get_or_init(|| Mutex::new(None))
}

/// Signal the WinDivert handle to abort its blocking recv() and unblock the
/// capture loop so the kernel handle can be released. Called on app exit
/// just before `WinDivert::uninstall()`.
pub fn request_shutdown() {
    if let Ok(guard) = active_divert_slot().lock() {
        if let Some(divert) = guard.as_ref() {
            divert.shutdown(WinDivertShutdownMode::Both);
        }
    }
}

/// ACTIVE_DIVERT スロットが None になったか確認する。
/// ExitRequested ハンドラーでポーリングに使う。
pub fn is_handle_closed() -> bool {
    active_divert_slot().lock().map_or(true, |g| g.is_none())
}

fn emit_server_handover(packet_sender: &tokio::sync::mpsc::Sender<PktEnvelope>) {
    let _ = packet_sender.try_send(PktEnvelope {
        op: Pkt::ServerHandover,
        data: vec![],
        conn: None,
    });
}

const HANDLE_CLEANUP_DELAY_MS: u64 = 500;
const MAX_SUBNET_CONNECTIONS: usize = 16;

pub fn start_capture() -> tokio::sync::mpsc::Receiver<PktEnvelope> {
    // 戦闘開始時のパケット波を吸収するため大きめに確保
    const PACKET_CHANNEL_CAPACITY: usize = 4096;
    let (packet_sender, packet_receiver) =
        tokio::sync::mpsc::channel::<PktEnvelope>(PACKET_CHANNEL_CAPACITY);
    let (restart_sender, mut restart_receiver) = watch::channel(false);
    RESTART_SENDER.set(restart_sender.clone()).ok();
    // WinDivert::recv() は同期ブロッキング呼び出しのため専用スレッドで動かす。
    // tokio ランタイムスレッドを占有せず、チャネル満杯時も recv が止まらない。
    std::thread::spawn(move || {
        loop {
            read_packets_blocking(&packet_sender, &mut restart_receiver);
            // Wait for restart signal
            while !*restart_receiver.borrow() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            // Reset signal to false before next loop
            let _ = restart_sender.send(false);
            // Delay to allow kernel to fully release the old handle
            std::thread::sleep(std::time::Duration::from_millis(HANDLE_CLEANUP_DELAY_MS));
        }
    });
    packet_receiver
}

fn read_packets_blocking(
    packet_sender: &tokio::sync::mpsc::Sender<PktEnvelope>,
    restart_receiver: &mut watch::Receiver<bool>,
) {
    // 過去のクラッシュ等でサービスが残っている場合の自動回復
    if let Err(e) = force_uninstall_service() {
        warn!("WinDivert pre-open cleanup: {e}");
    }
    let windivert = match WinDivert::network(
        "!loopback && ip && tcp",
        0,
        WinDivertFlags::new().set_sniff(),
    ) {
        Ok(handle) => {
            info!("WinDivert handle opened");
            handle
        }
        Err(e) => {
            error!("Failed to initialize WinDivert: {e}");
            return;
        }
    };
    let windivert = Arc::new(SharedDivert::new(windivert));
    if let Ok(mut slot) = active_divert_slot().lock() {
        *slot = Some(windivert.clone());
    }

    let mut windivert_buffer = vec![0u8; 10 * 1024 * 1024];
    let mut known_server: Option<Server> = None;
    let mut tcp_reassembler = TcpReassembler::new();
    let mut game_subnet: Option<[u8; 2]> = None;
    let mut subnet_reassemblers: HashMap<Server, TcpReassembler> = HashMap::new();

    while let Ok(packet) = windivert.recv(Some(&mut windivert_buffer)) {
        let Ok(network_slices) = SlicedPacket::from_ip(packet.data.as_ref()) else {
            continue;
        };
        let Some(Ipv4(ip_packet)) = network_slices.net else {
            continue;
        };
        let Some(Tcp(tcp_packet)) = network_slices.transport else {
            continue;
        };
        let curr_server = Server::new(
            ip_packet.header().source(),
            tcp_packet.to_header().source_port,
            ip_packet.header().destination(),
            tcp_packet.to_header().destination_port,
        );

        if known_server != Some(curr_server) {
            let tcp_payload = tcp_packet.payload();
            let mut detected = false;

            // 1. Try to identify game server via fragment signature
            let mut tcp_payload_reader = BinaryReader::from(tcp_payload.to_vec());
            if tcp_payload_reader.remaining() >= 10 {
                match tcp_payload_reader.read_bytes(10) {
                    Ok(bytes) => {
                        if bytes[4] == 0 {
                            const FRAG_LENGTH_SIZE: usize = 4;
                            let mut i = 0usize;
                            while tcp_payload_reader.remaining() >= FRAG_LENGTH_SIZE {
                                i += 1;
                                if i > 1000 {
                                    info!(
                                        "Potential infinite loop in server detection, iteration={i}"
                                    );
                                }
                                let tcp_frag_payload_len = match tcp_payload_reader.read_u32() {
                                    Ok(len) => len.saturating_sub(FRAG_LENGTH_SIZE as u32) as usize,
                                    Err(e) => {
                                        debug!("Malformed TCP fragment: failed to read_u32: {e}");
                                        break;
                                    }
                                };
                                if tcp_payload_reader.remaining() >= tcp_frag_payload_len {
                                    match tcp_payload_reader.read_bytes(tcp_frag_payload_len) {
                                        Ok(tcp_frag) => {
                                            let signature = crate::protocol::constants::server_detection::SERVER_SIGNATURE;
                                            let offset = crate::protocol::constants::packet_layout::SERVER_SIGNATURE_OFFSET;
                                            if tcp_frag.len() >= offset + signature.len()
                                                && tcp_frag[offset..offset + signature.len()]
                                                    == signature[..]
                                            {
                                                info!(
                                                    "Got Scene Server Address (by change): {curr_server}"
                                                );
                                                update_known_server(
                                                    &curr_server,
                                                    &mut known_server,
                                                    &mut game_subnet,
                                                    &mut tcp_reassembler,
                                                    tcp_packet.sequence_number().wrapping_add(
                                                        tcp_payload_reader.len() as u32,
                                                    ),
                                                    &mut subnet_reassemblers,
                                                );
                                                emit_server_handover(packet_sender);
                                                detected = true;
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            debug!(
                                                "Malformed TCP fragment: failed to read_bytes: {e}"
                                            );
                                            break;
                                        }
                                    }
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        debug!("Malformed TCP payload: failed to read_bytes(10): {e}");
                    }
                }
            }

            // 2. Login return packet detection
            if !detected
                && known_server.is_none()
                && tcp_payload.len()
                    == crate::protocol::constants::server_detection::LOGIN_RETURN_SIGNATURE_SIZE
            {
                let sig1 = crate::protocol::constants::server_detection::LOGIN_RETURN_SIGNATURE_1;
                let sig2 = crate::protocol::constants::server_detection::LOGIN_RETURN_SIGNATURE_2;
                if tcp_payload.len() >= 20
                    && tcp_payload[0..10] == sig1[..]
                    && tcp_payload[14..20] == sig2[..]
                {
                    info!("Got Scene Server Address by Login Return Packet: {curr_server}");
                    update_known_server(
                        &curr_server,
                        &mut known_server,
                        &mut game_subnet,
                        &mut tcp_reassembler,
                        tcp_packet
                            .sequence_number()
                            .wrapping_add(tcp_payload.len() as u32),
                        &mut subnet_reassemblers,
                    );
                    emit_server_handover(packet_sender);
                    detected = true;
                }
            }

            // 3. Auto-track game subnet connections
            if !detected && !tcp_payload.is_empty() {
                if let Some(prefix) = &game_subnet {
                    if curr_server.src_matches_subnet(prefix) {
                        if !subnet_reassemblers.contains_key(&curr_server) {
                            if subnet_reassemblers.len() < MAX_SUBNET_CONNECTIONS {
                                subnet_reassemblers.insert(curr_server, TcpReassembler::new());
                            }
                        }
                        if let Some(reassembler) = subnet_reassemblers.get_mut(&curr_server) {
                            reassemble_and_process(
                                reassembler,
                                &tcp_packet,
                                packet_sender,
                                true,
                                curr_server,
                            );
                        }
                    }
                }
            }
            continue;
        }

        // Primary server reassembly
        reassemble_and_process(
            &mut tcp_reassembler,
            &tcp_packet,
            packet_sender,
            false,
            curr_server,
        );

        if *restart_receiver.borrow() {
            info!("WinDivert restart requested during packet processing, closing handle");
            break;
        }
    }

    if let Ok(mut slot) = active_divert_slot().lock() {
        *slot = None;
    }
    drop(windivert);
}

fn update_known_server(
    server: &Server,
    known_server: &mut Option<Server>,
    game_subnet: &mut Option<[u8; 2]>,
    reassembler: &mut TcpReassembler,
    seq: u32,
    subnet_reassemblers: &mut HashMap<Server, TcpReassembler>,
) {
    *known_server = Some(*server);
    let src = server.src_addr();
    let dst = server.dst_addr();
    let prefix = if src[0] != 10 && src[0] != 172 && src[0] != 192 {
        [src[0], src[1]]
    } else {
        [dst[0], dst[1]]
    };
    *game_subnet = Some(prefix);
    info!("Game server subnet detected: {}.{}.*", prefix[0], prefix[1]);
    reassembler.clear(seq);
    subnet_reassemblers.clear();
}

fn reassemble_and_process(
    reassembler: &mut TcpReassembler,
    tcp_packet: &etherparse::TcpSlice<'_>,
    packet_sender: &tokio::sync::mpsc::Sender<PktEnvelope>,
    clear_on_malformed: bool,
    conn: Server,
) {
    if tcp_packet.payload().is_empty() {
        return;
    }
    if reassembler.next_seq.is_none() {
        reassembler.next_seq = Some(tcp_packet.sequence_number());
    }

    // Only insert if this segment is not behind next_seq
    if let Some(next_seq) = reassembler.next_seq {
        let pkt_seq = tcp_packet.sequence_number();
        // pkt_seq >= next_seq in wrapping arithmetic: wrapping_sub == 0 when equal, or
        // the difference is small (recent seq) vs large (stale seq that wrapped past)
        if next_seq.wrapping_sub(pkt_seq) == 0 {
            reassembler
                .cache
                .insert(pkt_seq, Vec::from(tcp_packet.payload()));
        }
    }

    let mut i = 0usize;
    loop {
        let next_seq = match reassembler.next_seq {
            Some(s) => s,
            None => break,
        };
        if !reassembler.cache.contains_key(&next_seq) {
            break;
        }
        i += 1;
        if i % 1000 == 0 {
            warn!(
                "Potential infinite loop in cache processing: iteration={i}, next_seq={next_seq}, cache_size={}, data_len={}",
                reassembler.cache.len(),
                reassembler.data.len()
            );
        }
        let cached_tcp_data = reassembler.cache.remove(&next_seq).expect("just checked");
        reassembler.data.extend_from_slice(&cached_tcp_data);
        reassembler.next_seq = Some(next_seq.wrapping_add(cached_tcp_data.len() as u32));
    }

    while reassembler.data.len() > 4 {
        let packet_size = match reassembler.data[..4].try_into().map(u32::from_be_bytes) {
            Ok(sz) => sz,
            Err(e) => {
                debug!("Malformed reassembled packet: failed to read_u32: {e}");
                break;
            }
        };
        const MIN_PACKET_SIZE: u32 = 6;
        const MAX_PACKET_SIZE: u32 = 10 * 1024 * 1024;
        if packet_size < MIN_PACKET_SIZE || packet_size > MAX_PACKET_SIZE {
            if clear_on_malformed {
                reassembler.data.clear();
                break;
            }
            warn!(
                "Malformed reassembled packet: invalid packet_size={packet_size}, data_len={}",
                reassembler.data.len()
            );
            reassembler.data.drain(0..1);
            continue;
        }
        if reassembler.data.len() < packet_size as usize {
            break;
        }
        let packet: Vec<u8> = reassembler.data.drain(..packet_size as usize).collect();
        process_packet(BinaryReader::from(packet), packet_sender, conn);
    }
}

pub fn request_restart() {
    if let Some(sender) = RESTART_SENDER.get() {
        let _ = sender.send(true);
    }
}

#[cfg(target_os = "windows")]
pub fn force_uninstall_service() -> Result<(), String> {
    use windows::Win32::Foundation::{ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_NOT_ACTIVE};
    use windows::Win32::System::Services::{
        CloseServiceHandle, ControlService, DeleteService, OpenSCManagerW, OpenServiceW,
        SC_MANAGER_CONNECT, SERVICE_CONTROL_STOP, SERVICE_STATUS, SERVICE_STOP,
    };
    use windows::core::PCWSTR;

    // DELETE (汎用アクセス権 0x00010000) は windows::Win32::Storage::FileSystem の型付き定数だが
    // OpenServiceW の引数は u32 なので直接値を使う
    const DELETE: u32 = 0x0001_0000;

    let service_name: Vec<u16> = "WinDivert\0".encode_utf16().collect();

    unsafe {
        let scm = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)
            .map_err(|e| format!("OpenSCManagerW failed: {e}"))?;

        let svc = match OpenServiceW(
            scm,
            PCWSTR::from_raw(service_name.as_ptr()),
            SERVICE_STOP | DELETE,
        ) {
            Ok(h) => h,
            Err(e) => {
                let _ = CloseServiceHandle(scm);
                if e.code() == ERROR_SERVICE_DOES_NOT_EXIST.to_hresult() {
                    return Ok(());
                }
                return Err(format!("OpenServiceW failed: {e}"));
            }
        };

        let mut status = SERVICE_STATUS::default();
        if let Err(e) = ControlService(svc, SERVICE_CONTROL_STOP, &mut status).ok() {
            if e.code() != ERROR_SERVICE_NOT_ACTIVE.to_hresult() {
                let _ = CloseServiceHandle(svc);
                let _ = CloseServiceHandle(scm);
                return Err(format!("ControlService(STOP) failed: {e}"));
            }
        }

        let delete_result = DeleteService(svc).ok();

        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);

        delete_result.map_err(|e| format!("DeleteService failed: {e}"))?;
    }

    Ok(())
}
