use std::collections::BTreeMap;

pub struct TcpReassembler {
    pub cache: BTreeMap<usize, Vec<u8>>, // sequence -> payload
    pub next_seq: Option<usize>,         // next expected sequence number
    pub data: Vec<u8>,
}

impl TcpReassembler {
    pub fn new() -> Self {
        Self {
            cache: BTreeMap::new(),
            next_seq: None,
            data: Vec::new(),
        }
    }

    pub fn clear(&mut self, seq_number: usize) {
        self.cache = BTreeMap::new();
        self.data.clear();
        self.next_seq = Some(seq_number);
    }
}

impl Default for TcpReassembler {
    fn default() -> Self {
        Self::new()
    }
}
