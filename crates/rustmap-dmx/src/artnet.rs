//! Art-Net 4 ArtDmx packets.

/// UDP port every Art-Net node listens on.
pub const PORT: u16 = 6454;

const OP_DMX: u16 = 0x5000;
const PROTOCOL_VERSION: u16 = 14;

/// An ArtDmx packet. `universe` is the 15-bit port-address (net, sub-net,
/// universe); `sequence` 1..=255 lets nodes reorder (0 disables that).
/// `data` is padded to an even length of at least 2, as the spec requires.
pub fn dmx_packet(universe: u16, sequence: u8, data: &[u8]) -> Vec<u8> {
    let data = &data[..data.len().min(512)];
    let len = data.len().max(2).next_multiple_of(2);
    let mut p = Vec::with_capacity(18 + len);
    p.extend_from_slice(b"Art-Net\0");
    p.extend_from_slice(&OP_DMX.to_le_bytes());
    p.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    p.push(sequence);
    p.push(0); // physical port
    let universe = universe & 0x7fff;
    p.push((universe & 0xff) as u8); // SubUni
    p.push((universe >> 8) as u8); // Net
    p.extend_from_slice(&(len as u16).to_be_bytes());
    p.extend_from_slice(data);
    p.resize(18 + len, 0);
    p
}

/// Parses an ArtDmx packet: (universe, sequence, data). For tests and the monitor.
pub fn parse_dmx(p: &[u8]) -> Option<(u16, u8, &[u8])> {
    if p.len() < 18 || &p[..8] != b"Art-Net\0" || u16::from_le_bytes([p[8], p[9]]) != OP_DMX {
        return None;
    }
    let universe = u16::from(p[14]) | (u16::from(p[15]) << 8);
    let len = usize::from(u16::from_be_bytes([p[16], p[17]]));
    Some((universe, p[12], p.get(18..18 + len)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_layout() {
        let p = dmx_packet(0x0123, 7, &[1, 2, 3]);
        assert_eq!(&p[..8], b"Art-Net\0");
        assert_eq!(&p[8..10], &[0x00, 0x50]); // OpDmx, little endian
        assert_eq!(&p[10..12], &[0, 14]); // version, big endian
        assert_eq!(p[12], 7);
        assert_eq!(p[14], 0x23); // SubUni
        assert_eq!(p[15], 0x01); // Net
        assert_eq!(&p[16..18], &[0, 4]); // odd length padded to even
        assert_eq!(&p[18..], &[1, 2, 3, 0]);
        assert_eq!(parse_dmx(&p), Some((0x0123, 7, &[1, 2, 3, 0][..])));
    }

    #[test]
    fn full_universe() {
        let p = dmx_packet(0, 1, &[9; 512]);
        assert_eq!(p.len(), 530);
        assert_eq!(&p[16..18], &[2, 0]);
    }
}
