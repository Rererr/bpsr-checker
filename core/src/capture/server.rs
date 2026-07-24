use std::fmt;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Server {
    src_addr: [u8; 4],
    src_port: u16,
    dst_addr: [u8; 4],
    dst_port: u16,
}

impl Server {
    pub fn new(src_addr: [u8; 4], src_port: u16, dst_addr: [u8; 4], dst_port: u16) -> Self {
        Self {
            src_addr,
            src_port,
            dst_addr,
            dst_port,
        }
    }

    pub fn src_addr(&self) -> [u8; 4] {
        self.src_addr
    }

    pub fn dst_addr(&self) -> [u8; 4] {
        self.dst_addr
    }

    pub fn reversed(&self) -> Self {
        Self::new(self.dst_addr, self.dst_port, self.src_addr, self.src_port)
    }

    pub fn src_matches_subnet(&self, prefix: &[u8; 2]) -> bool {
        self.src_addr[0..2] == prefix[..]
    }

    pub fn dst_matches_subnet(&self, prefix: &[u8; 2]) -> bool {
        self.dst_addr[0..2] == prefix[..]
    }
}

impl fmt::Display for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{} -> {}:{}",
            ip_to_str(&self.src_addr),
            self.src_port,
            ip_to_str(&self.dst_addr),
            self.dst_port
        )
    }
}

fn ip_to_str(ip: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}
