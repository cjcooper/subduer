use super::*;

const POD: [u8; 4] = [10, 0, 0, 5];
const PEER: [u8; 4] = [192, 0, 2, 9];

fn ext(kind: u16, data: &[u8]) -> Vec<u8> {
    [&kind.to_be_bytes()[..], &(data.len() as u16).to_be_bytes(), data].concat()
}

fn handshake(msg: u8, body: &[u8]) -> Vec<u8> {
    let len = body.len() as u32;
    let message = [&[msg][..], &len.to_be_bytes()[1..], body].concat();
    [&[22, 3, 1][..], &(message.len() as u16).to_be_bytes(), &message].concat()
}

fn client_hello_record(sni: Option<&str>, versions: &[u16]) -> Vec<u8> {
    let mut exts = vec![];
    if let Some(name) = sni {
        let entry = [&[0u8][..], &(name.len() as u16).to_be_bytes(), name.as_bytes()].concat();
        exts.extend(ext(0, &[&(entry.len() as u16).to_be_bytes()[..], &entry].concat()));
    }
    if !versions.is_empty() {
        let list: Vec<u8> = versions.iter().flat_map(|v| v.to_be_bytes()).collect();
        exts.extend(ext(0x2b, &[&[list.len() as u8][..], &list].concat()));
    }
    let body = [
        &[3, 3][..],
        &[7; 32],
        &[0],             // session id
        &[0, 2, 0x13, 1], // cipher suites
        &[1, 0],          // compression
        &(exts.len() as u16).to_be_bytes(),
        &exts,
    ]
    .concat();
    handshake(1, &body)
}

fn server_hello_record(legacy: u16, selected: Option<u16>, cipher: u16, random: [u8; 32]) -> Vec<u8> {
    let exts = selected.map(|v| ext(0x2b, &v.to_be_bytes())).unwrap_or_default();
    let body = [&legacy.to_be_bytes()[..], &random, &[0], &cipher.to_be_bytes(), &[0], &(exts.len() as u16).to_be_bytes(), &exts].concat();
    handshake(2, &body)
}

fn tcp(src_port: u16, dst_port: u16, payload: &[u8]) -> Vec<u8> {
    let mut h = vec![0u8; 20];
    h[0..2].copy_from_slice(&src_port.to_be_bytes());
    h[2..4].copy_from_slice(&dst_port.to_be_bytes());
    h[12] = 5 << 4;
    [h, payload.to_vec()].concat()
}

fn ipv4(src: [u8; 4], dst: [u8; 4], tcp: &[u8]) -> Vec<u8> {
    let mut h = vec![0u8; 20];
    h[0] = 0x45;
    h[2..4].copy_from_slice(&((20 + tcp.len()) as u16).to_be_bytes());
    h[9] = 6;
    h[12..16].copy_from_slice(&src);
    h[16..20].copy_from_slice(&dst);
    [h, tcp.to_vec()].concat()
}

fn ethernet(ip: &[u8], vlan: bool) -> Vec<u8> {
    let tag: &[u8] = if vlan { &[0x81, 0x00, 0, 1] } else { &[] };
    [&[0u8; 12][..], tag, &[0x08, 0x00], ip].concat()
}

/// A pcap file of `frames` (seconds, frame) with link type `link`.
fn pcap(little: bool, nanos: bool, link: u32, frames: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let w = |v: u32| if little { v.to_le_bytes() } else { v.to_be_bytes() };
    let magic: u32 = if nanos { 0xa1b2_3c4d } else { 0xa1b2_c3d4 };
    let mut out = [w(magic).to_vec(), vec![0, 0, 0, 0], vec![0; 8], w(65535).to_vec(), w(link).to_vec()].concat();
    out[4..8].copy_from_slice(&if little { [2, 0, 4, 0] } else { [0, 2, 0, 4] });
    for (secs, f) in frames {
        out.extend([w(*secs), w(if nanos { 500_000_000 } else { 500_000 }), w(f.len() as u32), w(f.len() as u32)].concat());
        out.extend(f);
    }
    out
}

fn eth_frame(src: [u8; 4], sport: u16, dst: [u8; 4], dport: u16, record: &[u8]) -> Vec<u8> {
    ethernet(&ipv4(src, dst, &tcp(sport, dport, record)), false)
}

#[test]
fn hellos_pair_into_connections() {
    let pod = IpAddr::from(POD);
    let frames = vec![
        // Out: the pod asks for TLS 1.3 (with a GREASE value) and gets it.
        (100, eth_frame(POD, 40000, PEER, 443, &client_hello_record(Some("api.example.com"), &[0x0a0a, 0x0304, 0x0303]))),
        (100, eth_frame(PEER, 443, POD, 40000, &server_hello_record(0x0303, Some(0x0304), 0x1301, [1; 32]))),
        // In: a client gets TLS 1.2 (no supported_versions in the answer).
        (101, eth_frame(PEER, 50000, POD, 8443, &client_hello_record(None, &[0x0304, 0x0303]))),
        (101, eth_frame(POD, 8443, PEER, 50000, &server_hello_record(0x0303, None, 0xcca8, [1; 32]))),
        // Neither end is the pod; an old client with no extensions; no answer captured.
        (102, eth_frame(PEER, 50001, [198, 51, 100, 1], 443, &client_hello_record(None, &[]))),
    ];
    let scan = scan(&pcap(true, false, 1, &frames), &[pod]).unwrap();
    assert_eq!((scan.packets, scan.hellos, scan.connections.len()), (5, 5, 3));
    let [out, inbound, other] = &scan.connections[..] else { panic!() };
    assert_eq!((out.client.as_str(), out.server.as_str(), out.direction), ("10.0.0.5:40000", "192.0.2.9:443", TlsDirection::Outbound));
    assert_eq!(out.server_name.as_deref(), Some("api.example.com"));
    assert_eq!(out.offered, ["TLS 1.3", "TLS 1.2"], "GREASE left out");
    assert_eq!((out.version.as_deref(), out.cipher.as_deref()), (Some("TLS 1.3"), Some("TLS_AES_128_GCM_SHA256")));
    assert_eq!(out.first_seen, 100.5);
    assert_eq!(inbound.direction, TlsDirection::Inbound);
    assert_eq!((inbound.version.as_deref(), inbound.cipher.as_deref()), (Some("TLS 1.2"), Some("TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256")));
    assert_eq!((other.direction, other.offered.clone(), other.version.clone()), (TlsDirection::Other, vec!["TLS 1.2".to_owned()], None));
}

#[test]
fn a_retry_and_a_missed_client_hello() {
    let frames = vec![
        (1, eth_frame(POD, 40000, PEER, 443, &client_hello_record(None, &[0x0304]))),
        (1, eth_frame(PEER, 443, POD, 40000, &server_hello_record(0x0303, Some(0x0304), 0x1302, RETRY_RANDOM))),
        // The client's second hello, after the retry, is the same connection.
        (1, eth_frame(POD, 40000, PEER, 443, &client_hello_record(None, &[0x0304]))),
        (1, eth_frame(PEER, 443, POD, 40000, &server_hello_record(0x0303, Some(0x0304), 0x1302, [1; 32]))),
        // An answer whose hello came before the capture started.
        (2, eth_frame(PEER, 443, POD, 40001, &server_hello_record(0x0301, None, 0x002f, [1; 32]))),
    ];
    let scan = scan(&pcap(true, false, 1, &frames), &[IpAddr::from(POD)]).unwrap();
    assert_eq!(scan.connections.len(), 2);
    let retried = &scan.connections[0];
    assert!(retried.retried);
    assert_eq!((retried.version.as_deref(), retried.cipher.as_deref()), (Some("TLS 1.3"), Some("TLS_AES_256_GCM_SHA384")));
    let late = &scan.connections[1];
    assert_eq!((late.client.as_str(), late.direction, late.offered.len()), ("10.0.0.5:40001", TlsDirection::Outbound, 0));
    assert_eq!((late.version.as_deref(), late.cipher.as_deref()), (Some("TLS 1.0"), Some("TLS_RSA_WITH_AES_128_CBC_SHA")));
}

#[test]
fn file_and_link_formats() {
    let hello = client_hello_record(Some("a.example"), &[0x0304]);
    let segment = tcp(40000, 443, &hello);
    // Big-endian with nanosecond times, Ethernet with a VLAN tag.
    let vlan = scan(&pcap(false, true, 1, &[(7, ethernet(&ipv4(POD, PEER, &segment), true))]), &[]).unwrap();
    assert_eq!((vlan.connections.len(), vlan.connections[0].first_seen), (1, 7.5));
    // Linux cooked capture (tcpdump -i any).
    let sll = [&[0u8; 14][..], &[0x08, 0x00], &ipv4(POD, PEER, &segment)].concat();
    assert_eq!(scan(&pcap(true, false, 113, &[(1, sll)]), &[]).unwrap().connections.len(), 1);
    // Raw IPv6.
    let mut v6 = vec![0u8; 40];
    v6[0] = 0x60;
    v6[4..6].copy_from_slice(&(segment.len() as u16).to_be_bytes());
    v6[6] = 6;
    v6[23] = 1;
    v6[39] = 2;
    let v6 = [v6, segment.clone()].concat();
    let c = &scan(&pcap(true, false, 101, &[(1, v6)]), &[]).unwrap().connections[0];
    assert_eq!((c.client.as_str(), c.server.as_str()), ("[::1]:40000", "[::2]:443"));
    // A record cut short at the end is ignored; the rest is read.
    let mut cut = pcap(true, false, 1, &[(1, eth_frame(POD, 40000, PEER, 443, &hello))]);
    cut.extend([0u8; 10]);
    assert_eq!(scan(&cut, &[]).unwrap().connections.len(), 1);
    // A hello cut short by the snapshot length still gives its version.
    assert_eq!(hellos(&hello[..60]).len(), 1);

    assert_eq!(scan(&[0x0a, 0x0d, 0x0d, 0x0a, 0, 0], &[]).unwrap_err().to_string(), "this is a pcapng file; save it as pcap to scan it");
    assert_eq!(scan(b"hello world", &[]).unwrap_err().to_string(), "this isn't a pcap file");
    assert_eq!(scan(&[], &[]).unwrap_err().to_string(), "the capture is empty");
}

#[test]
fn a_stream_fed_in_pieces_reads_the_same() {
    let frames = vec![
        (1, eth_frame(POD, 40000, PEER, 443, &client_hello_record(Some("a.example"), &[0x0304]))),
        (1, eth_frame(PEER, 443, POD, 40000, &server_hello_record(0x0303, Some(0x0304), 0x1301, [1; 32]))),
    ];
    let file = pcap(true, false, 1, &frames);
    let whole = scan(&file, &[IpAddr::from(POD)]).unwrap();
    for size in [1, 7, 23, 100] {
        let mut s = Scanner::new(vec![IpAddr::from(POD)]);
        for piece in file.chunks(size) {
            s.feed(piece).unwrap();
        }
        assert_eq!(s.result(), &whole, "fed {size} bytes at a time");
    }
    // Half a header waits; a bad magic fails as soon as it's seen.
    let mut s = Scanner::new(vec![]);
    s.feed(&file[..10]).unwrap();
    assert_eq!(s.result().packets, 0);
    assert!(Scanner::new(vec![]).feed(b"GET / HTTP").is_err());
}

#[test]
fn names() {
    assert_eq!((version_name(0x0300), version_name(0x7f1c)), ("SSL 3.0".into(), "0x7f1c".into()));
    assert_eq!(cipher_name(0xfefe), "0xfefe");
    assert!(is_grease(0x1a1a) && !is_grease(0x0304));
}
