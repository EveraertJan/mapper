//! sACN (ANSI E1.31) data packets.

use std::net::Ipv4Addr;

/// UDP port for sACN.
pub const PORT: u16 = 5568;

const ACN_ID: &[u8; 12] = b"ASC-E1.17\0\0\0";
const VECTOR_ROOT_DATA: u32 = 0x0000_0004;
const VECTOR_FRAMING_DATA: u32 = 0x0000_0002;
const VECTOR_DMP_SET_PROPERTY: u8 = 0x02;

/// Default priority (0–200).
pub const DEFAULT_PRIORITY: u8 = 100;

/// Multicast group of a universe: 239.255.hi.lo.
pub fn multicast_addr(universe: u16) -> Ipv4Addr {
    let [hi, lo] = universe.to_be_bytes();
    Ipv4Addr::new(239, 255, hi, lo)
}

fn flags_length(len: usize) -> [u8; 2] {
    (0x7000 | (len as u16 & 0x0fff)).to_be_bytes()
}

/// An E1.31 data packet with start code 0 and up to 512 channels.
pub fn data_packet(
    cid: &[u8; 16],
    source_name: &str,
    priority: u8,
    universe: u16,
    sequence: u8,
    data: &[u8],
) -> Vec<u8> {
    let data = &data[..data.len().min(512)];
    let total = 126 + data.len();
    let mut p = Vec::with_capacity(total);
    // Root layer.
    p.extend_from_slice(&0x0010u16.to_be_bytes()); // preamble size
    p.extend_from_slice(&0u16.to_be_bytes()); // postamble size
    p.extend_from_slice(ACN_ID);
    p.extend_from_slice(&flags_length(total - 16));
    p.extend_from_slice(&VECTOR_ROOT_DATA.to_be_bytes());
    p.extend_from_slice(cid);
    // Framing layer.
    p.extend_from_slice(&flags_length(total - 38));
    p.extend_from_slice(&VECTOR_FRAMING_DATA.to_be_bytes());
    let mut name = [0u8; 64];
    let bytes = source_name.as_bytes();
    let n = bytes.len().min(63);
    name[..n].copy_from_slice(&bytes[..n]);
    p.extend_from_slice(&name);
    p.push(priority.min(200));
    p.extend_from_slice(&0u16.to_be_bytes()); // synchronization address
    p.push(sequence);
    p.push(0); // options
    p.extend_from_slice(&universe.to_be_bytes());
    // DMP layer.
    p.extend_from_slice(&flags_length(total - 115));
    p.push(VECTOR_DMP_SET_PROPERTY);
    p.push(0xa1); // address type & data type
    p.extend_from_slice(&0u16.to_be_bytes()); // first property address
    p.extend_from_slice(&1u16.to_be_bytes()); // address increment
    p.extend_from_slice(&((data.len() + 1) as u16).to_be_bytes());
    p.push(0); // DMX start code
    p.extend_from_slice(data);
    debug_assert_eq!(p.len(), total);
    p
}

/// Parses a data packet: (universe, sequence, data).
pub fn parse_data(p: &[u8]) -> Option<(u16, u8, &[u8])> {
    if p.len() < 126 || &p[4..16] != ACN_ID || p[125] != 0 {
        return None;
    }
    let universe = u16::from_be_bytes([p[113], p[114]]);
    let count = usize::from(u16::from_be_bytes([p[123], p[124]]));
    Some((universe, p[111], p.get(126..125 + count)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_universe_layout() {
        let cid = [7u8; 16];
        let p = data_packet(&cid, "RUSTMAP", 100, 0x0102, 9, &[0xaa; 512]);
        assert_eq!(p.len(), 638);
        assert_eq!(&p[0..4], &[0x00, 0x10, 0x00, 0x00]);
        assert_eq!(&p[16..18], &[0x72, 0x6e]); // 0x7000 | 622
        assert_eq!(&p[18..22], &[0, 0, 0, 4]);
        assert_eq!(&p[22..38], &cid);
        assert_eq!(&p[38..40], &[0x72, 0x58]); // 0x7000 | 600
        assert_eq!(&p[40..44], &[0, 0, 0, 2]);
        assert_eq!(&p[44..51], b"RUSTMAP");
        assert_eq!(p[108], 100);
        assert_eq!(p[111], 9);
        assert_eq!(&p[113..115], &[1, 2]);
        assert_eq!(&p[115..117], &[0x72, 0x0b]); // 0x7000 | 523
        assert_eq!(&p[117..119], &[0x02, 0xa1]);
        assert_eq!(&p[123..125], &[0x02, 0x01]); // 513 values
        assert_eq!(p[125], 0);
        assert_eq!(parse_data(&p), Some((0x0102, 9, &[0xaa; 512][..])));
    }

    #[test]
    fn multicast_group() {
        assert_eq!(multicast_addr(1), Ipv4Addr::new(239, 255, 0, 1));
        assert_eq!(multicast_addr(0x1234), Ipv4Addr::new(239, 255, 0x12, 0x34));
    }
}
