//! Sends real UDP packets to localhost and parses them back.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::time::Duration;

use rustmap_dmx::{DmxSender, Ports, Protocol, UniverseData, artnet, parse_destination, sacn};

fn receiver() -> (UdpSocket, u16) {
    let s = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let port = s.local_addr().unwrap().port();
    (s, port)
}

fn universe(protocol: Protocol, universe: u16, fill: u8) -> UniverseData {
    UniverseData {
        protocol,
        destination: Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        universe,
        data: Box::new([fill; 512]),
    }
}

#[test]
fn artnet_and_sacn_reach_a_receiver() {
    let (art, art_port) = receiver();
    let (acn, acn_port) = receiver();
    let sender = DmxSender::with_ports(
        "RUSTMAP test",
        Ports {
            artnet: art_port,
            sacn: acn_port,
        },
    )
    .unwrap();
    sender.send(vec![
        universe(Protocol::ArtNet, 3, 0x11),
        universe(Protocol::Sacn, 7, 0x22),
    ]);

    let mut buf = [0u8; 1024];
    let n = art.recv(&mut buf).unwrap();
    let (u, seq, data) = artnet::parse_dmx(&buf[..n]).unwrap();
    assert_eq!((u, seq, data.len()), (3, 1, 512));
    assert!(data.iter().all(|v| *v == 0x11));

    let n = acn.recv(&mut buf).unwrap();
    let (u, _, data) = sacn::parse_data(&buf[..n]).unwrap();
    assert_eq!((u, data.len()), (7, 512));
    assert!(data.iter().all(|v| *v == 0x22));

    // Sequence numbers advance per universe.
    sender.send(vec![universe(Protocol::ArtNet, 3, 0x33)]);
    let n = art.recv(&mut buf).unwrap();
    let (_, seq, data) = artnet::parse_dmx(&buf[..n]).unwrap();
    assert_eq!((seq, data[0]), (2, 0x33));

    std::thread::sleep(Duration::from_millis(50));
    let stats = sender.stats();
    assert_eq!(stats.packets, 3);
    assert_eq!(stats.errors, 0);
}

#[test]
fn destinations() {
    assert_eq!(parse_destination("  "), Ok(None));
    assert_eq!(
        parse_destination("10.0.0.5"),
        Ok(Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))))
    );
    assert!(parse_destination("controller").is_err());
}
