use crate::capture::binary_reader::BinaryReader;
use crate::capture::server::Server;
use crate::protocol::constants::{
    self, SERVICE_UUID, SOCIAL_NTF_NOTIFY_METHOD_ID, SOCIAL_NTF_SERVICE_ID,
};
use crate::protocol::opcodes::{FragmentType, Pkt, PktEnvelope};
use log::debug;
use tokio::sync::mpsc;

const FRAME_HEADER_MIN: u32 = 6;

enum FrameOutcome {
    Forward { op: Pkt, payload: Vec<u8> },
    Skip,
    Reframe(BinaryReader),
}

macro_rules! try_read {
    ($expr:expr, $ctx:literal) => {
        match $expr {
            Ok(v) => v,
            Err(e) => {
                debug!("{}: {e}", $ctx);
                return None;
            }
        }
    };
}

pub fn process_packet(
    initial: BinaryReader,
    out: &mpsc::Sender<PktEnvelope>,
    conn: Server,
) {
    let mut stream = initial;

    while stream.remaining() > 0 {
        let frame_bytes = match peek_frame(&mut stream) {
            Some(bytes) => bytes,
            None => break,
        };

        let mut frame = BinaryReader::from(frame_bytes);
        let outcome = match parse_frame(&mut frame) {
            Some(o) => o,
            None => continue,
        };

        match outcome {
            FrameOutcome::Forward { op, payload } => {
                if let Err(err) = out.blocking_send(PktEnvelope {
                    op,
                    data: payload,
                    conn: Some(conn),
                }) {
                    debug!("dispatch dropped: {err}");
                }
            }
            FrameOutcome::Skip => continue,
            FrameOutcome::Reframe(inner) => {
                stream = inner;
            }
        }
    }
}

fn peek_frame(stream: &mut BinaryReader) -> Option<Vec<u8>> {
    let frame_len = match stream.peek_u32() {
        Ok(len) => len,
        Err(e) => {
            debug!("frame: cannot peek length ({e})");
            return None;
        }
    };
    if frame_len < FRAME_HEADER_MIN {
        debug!("frame: undersized ({frame_len} < {FRAME_HEADER_MIN})");
        return None;
    }
    match stream.read_bytes(frame_len as usize) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            debug!("frame: cannot consume {frame_len} bytes ({e})");
            None
        }
    }
}

fn parse_frame(frame: &mut BinaryReader) -> Option<FrameOutcome> {
    try_read!(frame.read_u32(), "frame: length skip");
    let raw_type = try_read!(frame.read_u16(), "frame: type word");
    let compressed = (raw_type & constants::packet::COMPRESSION_FLAG) != 0;
    let kind = FragmentType::from(constants::packet::extract_type(raw_type));

    match kind {
        FragmentType::Notify => decode_notify(frame, compressed),
        FragmentType::FrameDown => decode_framedown(frame, compressed),
        other => {
            debug!("frame: ignored kind {other:?}");
            Some(FrameOutcome::Skip)
        }
    }
}

fn decode_notify(frame: &mut BinaryReader, compressed: bool) -> Option<FrameOutcome> {
    let service = try_read!(frame.read_u64(), "notify: service id");
    let _stub = try_read!(frame.read_u32(), "notify: stub id");
    let method = try_read!(frame.read_u32(), "notify: method id");

    let body = maybe_decompress(frame.read_remaining(), compressed, "notify")?;

    if service == SOCIAL_NTF_SERVICE_ID && method == SOCIAL_NTF_NOTIFY_METHOD_ID {
        return Some(FrameOutcome::Forward {
            op: Pkt::SocialEnvelope,
            payload: body,
        });
    }
    if service != SERVICE_UUID {
        return Some(FrameOutcome::Skip);
    }

    match Pkt::try_from(method) {
        Ok(op) => Some(FrameOutcome::Forward { op, payload: body }),
        Err(_) => {
            debug!("notify: unmapped method 0x{method:08x} on service 0x{service:016x}");
            Some(FrameOutcome::Skip)
        }
    }
}

fn decode_framedown(frame: &mut BinaryReader, compressed: bool) -> Option<FrameOutcome> {
    try_read!(frame.read_u32(), "framedown: sequence id");
    if frame.remaining() == 0 {
        debug!("framedown: empty payload");
        return None;
    }
    let bytes = frame.read_remaining();
    let next = maybe_decompress(bytes, compressed, "framedown")?;
    Some(FrameOutcome::Reframe(BinaryReader::from(next)))
}

fn maybe_decompress(bytes: &[u8], compressed: bool, ctx: &str) -> Option<Vec<u8>> {
    if !compressed {
        return Some(bytes.to_vec());
    }
    match zstd::decode_all(bytes) {
        Ok(decoded) => Some(decoded),
        Err(e) => {
            debug!("{ctx}: zstd failure ({e})");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::binary_reader::BinaryReader;
    use crate::capture::server::Server;

    #[test]
    fn empty_stream_terminates() {
        let (tx, _rx) = mpsc::channel::<PktEnvelope>(1);
        let reader = BinaryReader::from(vec![]);
        let conn = Server::new([0, 0, 0, 0], 0, [0, 0, 0, 0], 0);
        process_packet(reader, &tx, conn);
    }
}
