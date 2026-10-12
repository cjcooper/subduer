//! Reading TLS versions out of a packet capture: every ClientHello and
//! ServerHello in a pcap file, paired by connection, so a capture can show
//! which version and cipher each connection settled on. Works on the file
//! alone. Reads classic pcap (either byte order, micro- or nanosecond
//! times) with Ethernet (and VLAN tags), Linux cooked (v1, v2) or raw IP
//! links, over IPv4 or IPv6.
//!
//! Only what the handshake shows in the clear is read. In TLS 1.3 that's
//! the ClientHello and ServerHello; the rest of the handshake is encrypted.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::types::{TlsConnection, TlsDirection, TlsScan};
use crate::{Error, Result};

/// A ServerHello with this random is a HelloRetryRequest (RFC 8446 4.1.3).
const RETRY_RANDOM: [u8; 32] = [
    0xcf, 0x21, 0xad, 0x74, 0xe5, 0x9a, 0x61, 0x11, 0xbe, 0x1d, 0x8c, 0x02, 0x1e, 0x65, 0xb8, 0x91, 0xc2, 0xa2, 0x11, 0x16, 0x7a, 0xbb, 0x8c,
    0x5e, 0x07, 0x9e, 0x09, 0xe2, 0xc8, 0xa8, 0x33, 0x9c,
];
const SNI: u16 = 0x0000;
const SUPPORTED_VERSIONS: u16 = 0x002b;

/// Bounds-checked big-endian reads.
struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.b.len() < n {
            return None;
        }
        let (head, rest) = self.b.split_at(n);
        self.b = rest;
        Some(head)
    }
    /// Up to `n` bytes: a message cut short by the capture keeps what it has.
    fn take_up_to(&mut self, n: usize) -> &'a [u8] {
        let n = n.min(self.b.len());
        self.take(n).unwrap_or_default()
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn u24(&mut self) -> Option<usize> {
        self.take(3).map(|b| usize::from(b[0]) << 16 | usize::from(b[1]) << 8 | usize::from(b[2]))
    }
    /// A length-prefixed vector (`len` bytes of length).
    fn vector(&mut self, len: usize) -> Option<&'a [u8]> {
        let n = match len {
            1 => usize::from(self.u8()?),
            2 => usize::from(self.u16()?),
            _ => self.u24()?,
        };
        self.take(n)
    }
}

pub fn version_name(v: u16) -> String {
    match v {
        0x0304 => "TLS 1.3".into(),
        0x0303 => "TLS 1.2".into(),
        0x0302 => "TLS 1.1".into(),
        0x0301 => "TLS 1.0".into(),
        0x0300 => "SSL 3.0".into(),
        0x0002 => "SSL 2.0".into(),
        _ => format!("0x{v:04x}"),
    }
}

/// Reserved values clients send to keep servers tolerant (RFC 8701).
fn is_grease(v: u16) -> bool {
    v & 0x0f0f == 0x0a0a && v >> 8 == v & 0xff
}

/// IANA names of the cipher suites seen in practice; others by number.
pub fn cipher_name(c: u16) -> String {
    let name = match c {
        0x1301 => "TLS_AES_128_GCM_SHA256",
        0x1302 => "TLS_AES_256_GCM_SHA384",
        0x1303 => "TLS_CHACHA20_POLY1305_SHA256",
        0x1304 => "TLS_AES_128_CCM_SHA256",
        0x1305 => "TLS_AES_128_CCM_8_SHA256",
        0xc02b => "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
        0xc02c => "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
        0xc02f => "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
        0xc030 => "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
        0xcca8 => "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256",
        0xcca9 => "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256",
        0xc013 => "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA",
        0xc014 => "TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA",
        0xc009 => "TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA",
        0xc00a => "TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA",
        0xc023 => "TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA256",
        0xc027 => "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA256",
        0x009c => "TLS_RSA_WITH_AES_128_GCM_SHA256",
        0x009d => "TLS_RSA_WITH_AES_256_GCM_SHA384",
        0x002f => "TLS_RSA_WITH_AES_128_CBC_SHA",
        0x0035 => "TLS_RSA_WITH_AES_256_CBC_SHA",
        0x003c => "TLS_RSA_WITH_AES_128_CBC_SHA256",
        0x003d => "TLS_RSA_WITH_AES_256_CBC_SHA256",
        0x000a => "TLS_RSA_WITH_3DES_EDE_CBC_SHA",
        0x0005 => "TLS_RSA_WITH_RC4_128_SHA",
        _ => return format!("0x{c:04x}"),
    };
    name.into()
}

/// What a ClientHello says.
#[derive(Debug, Default, PartialEq)]
struct ClientHello {
    server_name: Option<String>,
    offered: Vec<u16>,
}

/// What a ServerHello says.
#[derive(Debug, PartialEq)]
struct ServerHello {
    version: u16,
    cipher: u16,
    retry: bool,
}

fn client_hello(body: &[u8]) -> Option<ClientHello> {
    let mut r = Reader::new(body);
    let legacy = r.u16()?;
    r.take(32)?;
    r.vector(1)?;
    r.vector(2)?;
    r.vector(1)?;
    let mut hello = ClientHello { offered: vec![legacy], ..ClientHello::default() };
    // A hello cut short by the capture still has its version; extensions are best effort.
    let Some(mut exts) = r.vector(2).map(Reader::new) else { return Some(hello) };
    while let (Some(kind), Some(data)) = (exts.u16(), exts.vector(2)) {
        let mut d = Reader::new(data);
        match kind {
            SNI => {
                let mut list = Reader::new(d.vector(2).unwrap_or_default());
                while let (Some(name_type), Some(name)) = (list.u8(), list.vector(2)) {
                    if name_type == 0 {
                        hello.server_name = Some(String::from_utf8_lossy(name).into_owned());
                    }
                }
            }
            SUPPORTED_VERSIONS => {
                let mut list = Reader::new(d.vector(1).unwrap_or_default());
                let versions: Vec<u16> = std::iter::from_fn(|| list.u16()).filter(|v| !is_grease(*v)).collect();
                if !versions.is_empty() {
                    hello.offered = versions;
                }
            }
            _ => {}
        }
    }
    Some(hello)
}

fn server_hello(body: &[u8]) -> Option<ServerHello> {
    let mut r = Reader::new(body);
    let legacy = r.u16()?;
    let random = r.take(32)?;
    r.vector(1)?;
    let cipher = r.u16()?;
    r.u8()?;
    let mut hello = ServerHello { version: legacy, cipher, retry: random == RETRY_RANDOM };
    let Some(mut exts) = r.vector(2).map(Reader::new) else { return Some(hello) };
    while let (Some(kind), Some(data)) = (exts.u16(), exts.vector(2)) {
        if kind == SUPPORTED_VERSIONS
            && let Some(v) = Reader::new(data).u16()
        {
            hello.version = v;
        }
    }
    Some(hello)
}

enum Hello {
    Client(ClientHello),
    Server(ServerHello),
}

/// The hellos in a TCP payload: handshake records (type 22) and the
/// ClientHello (1) and ServerHello (2) messages in them.
fn hellos(payload: &[u8]) -> Vec<Hello> {
    let mut out = vec![];
    let mut r = Reader::new(payload);
    while let (Some(kind), Some(_version), Some(len)) = (r.u8(), r.u16(), r.u16()) {
        let fragment = r.take_up_to(usize::from(len));
        if kind != 22 {
            continue;
        }
        let mut m = Reader::new(fragment);
        while let (Some(msg), Some(len)) = (m.u8(), m.u24()) {
            let body = m.take_up_to(len);
            match msg {
                1 => out.extend(client_hello(body).map(Hello::Client)),
                2 => out.extend(server_hello(body).map(Hello::Server)),
                _ => {}
            }
        }
    }
    out
}

/// An address and port.
type Endpoint = (IpAddr, u16);

/// One TCP segment's ends and payload.
struct Segment<'a> {
    src: Endpoint,
    dst: Endpoint,
    payload: &'a [u8],
}

/// The IP packet in a link-layer frame.
fn ip_packet(link: u32, frame: &[u8]) -> Option<&[u8]> {
    let (mut ethertype, mut rest) = match link {
        // Ethernet.
        1 => {
            let mut r = Reader::new(frame);
            r.take(12)?;
            (r.u16()?, r.b)
        }
        // Linux cooked capture, v1 and v2.
        113 => (u16::from_be_bytes([*frame.get(14)?, *frame.get(15)?]), frame.get(16..)?),
        276 => (u16::from_be_bytes([*frame.first()?, *frame.get(1)?]), frame.get(20..)?),
        // Raw IP.
        101 | 12 | 228 | 229 => return Some(frame),
        _ => return None,
    };
    // VLAN tags.
    while matches!(ethertype, 0x8100 | 0x88a8) {
        let mut r = Reader::new(rest);
        r.take(2)?;
        ethertype = r.u16()?;
        rest = r.b;
    }
    matches!(ethertype, 0x0800 | 0x86dd).then_some(rest)
}

fn tcp_segment(ip: &[u8]) -> Option<Segment<'_>> {
    let (src, dst, tcp) = match ip.first()? >> 4 {
        4 => {
            let ihl = usize::from(ip[0] & 0x0f) * 4;
            let total = usize::from(u16::from_be_bytes([*ip.get(2)?, *ip.get(3)?]));
            let fragment = u16::from_be_bytes([*ip.get(6)?, *ip.get(7)?]) & 0x1fff;
            if *ip.get(9)? != 6 || fragment != 0 || ihl < 20 {
                return None;
            }
            let addr = |at: usize| -> Option<IpAddr> { Some(Ipv4Addr::from(<[u8; 4]>::try_from(ip.get(at..at + 4)?).ok()?).into()) };
            (addr(12)?, addr(16)?, ip.get(ihl..total.min(ip.len()).max(ihl))?)
        }
        6 => {
            let len = usize::from(u16::from_be_bytes([*ip.get(4)?, *ip.get(5)?]));
            if *ip.get(6)? != 6 {
                return None;
            }
            let addr = |at: usize| -> Option<IpAddr> { Some(Ipv6Addr::from(<[u8; 16]>::try_from(ip.get(at..at + 16)?).ok()?).into()) };
            (addr(8)?, addr(24)?, ip.get(40..(40 + len).min(ip.len()))?)
        }
        _ => return None,
    };
    let port = |at: usize| -> Option<u16> { Some(u16::from_be_bytes([*tcp.get(at)?, *tcp.get(at + 1)?])) };
    let offset = usize::from(*tcp.get(12)? >> 4) * 4;
    Some(Segment { src: (src, port(0)?), dst: (dst, port(2)?), payload: tcp.get(offset..)? })
}

fn endpoint((ip, port): (IpAddr, u16)) -> String {
    match ip {
        IpAddr::V4(a) => format!("{a}:{port}"),
        IpAddr::V6(a) => format!("[{a}]:{port}"),
    }
}

/// How a pcap file is written: byte order, time unit and link type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    little: bool,
    nanos: bool,
    pub link: u32,
}

impl Format {
    /// From the 24-byte file header; `Ok(None)` if more is needed.
    pub fn read(head: &[u8]) -> Result<Option<Format>> {
        let Some(magic) = head.get(..4) else { return Ok(None) };
        let (little, nanos) = match magic {
            [0xd4, 0xc3, 0xb2, 0xa1] => (true, false),
            [0x4d, 0x3c, 0xb2, 0xa1] => (true, true),
            [0xa1, 0xb2, 0xc3, 0xd4] => (false, false),
            [0xa1, 0xb2, 0x3c, 0x4d] => (false, true),
            [0x0a, 0x0d, 0x0d, 0x0a] => return Err(Error::Message("this is a pcapng file; save it as pcap to scan it".into())),
            _ => return Err(Error::Message("this isn't a pcap file".into())),
        };
        let Some(header) = head.get(..24) else { return Ok(None) };
        let mut f = Format { little, nanos, link: 0 };
        f.link = f.word(&header[20..24]) & 0x0fff_ffff;
        Ok(Some(f))
    }

    fn word(&self, b: &[u8]) -> u32 {
        let b = [b[0], b[1], b[2], b[3]];
        if self.little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }
    }

    /// A packet record's (time, captured length) from its 16-byte header.
    fn record(&self, head: &[u8]) -> (f64, usize) {
        let (secs, frac) = (self.word(&head[0..4]), self.word(&head[4..8]));
        (f64::from(secs) + f64::from(frac) / if self.nanos { 1e9 } else { 1e6 }, self.word(&head[8..12]) as usize)
    }
}

/// Reads a pcap stream as it arrives, as a capture runs: feed it bytes, and
/// whole packets are read; a packet cut off waits for the rest.
pub struct Scanner {
    pod_ips: Vec<IpAddr>,
    format: Option<Format>,
    /// Bytes not read yet.
    pending: Vec<u8>,
    out: TlsScan,
    /// Connection (client, server) → its index in `out.connections`.
    open: HashMap<(Endpoint, Endpoint), usize>,
}

impl Scanner {
    /// `pod_ips` (the captured pod's addresses) decide which way each
    /// connection went.
    pub fn new(pod_ips: Vec<IpAddr>) -> Self {
        Self { pod_ips, format: None, pending: vec![], out: TlsScan::default(), open: HashMap::new() }
    }

    /// Reads what `bytes` complete. Fails only if the stream isn't pcap.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.pending.extend_from_slice(bytes);
        let format = match self.format {
            Some(f) => f,
            None => match Format::read(&self.pending)? {
                Some(f) => {
                    self.pending.drain(..24);
                    self.format = Some(f);
                    f
                }
                None => return Ok(()),
            },
        };
        let pending = std::mem::take(&mut self.pending);
        let mut at = 0;
        while let Some(head) = pending.get(at..at + 16) {
            let (t, included) = format.record(head);
            let Some(frame) = pending.get(at + 16..at + 16 + included) else { break };
            self.packet(format.link, t, frame);
            at += 16 + included;
        }
        self.pending = pending[at..].to_vec();
        Ok(())
    }

    /// What's been read so far.
    pub fn result(&self) -> &TlsScan {
        &self.out
    }

    fn direction(&self, client: &Endpoint, server: &Endpoint) -> TlsDirection {
        if self.pod_ips.contains(&server.0) {
            TlsDirection::Inbound
        } else if self.pod_ips.contains(&client.0) {
            TlsDirection::Outbound
        } else {
            TlsDirection::Other
        }
    }

    fn connection(&mut self, t: f64, client: Endpoint, server: Endpoint, hello: Option<ClientHello>) -> usize {
        let hello = hello.unwrap_or_default();
        self.out.connections.push(TlsConnection {
            first_seen: t,
            client: endpoint(client),
            server: endpoint(server),
            direction: self.direction(&client, &server),
            server_name: hello.server_name,
            offered: hello.offered.into_iter().map(version_name).collect(),
            version: None,
            cipher: None,
            retried: false,
        });
        self.out.connections.len() - 1
    }

    fn packet(&mut self, link: u32, t: f64, frame: &[u8]) {
        self.out.packets += 1;
        let Some(seg) = ip_packet(link, frame).and_then(tcp_segment) else { return };
        for hello in hellos(seg.payload) {
            self.out.hellos += 1;
            match hello {
                Hello::Client(h) => {
                    let key = (seg.src, seg.dst);
                    // The second ClientHello after a retry belongs to the same connection.
                    if let Some(&i) = self.open.get(&key)
                        && self.out.connections[i].version.is_none()
                    {
                        continue;
                    }
                    let i = self.connection(t, seg.src, seg.dst, Some(h));
                    self.open.insert(key, i);
                }
                Hello::Server(h) => {
                    let key = (seg.dst, seg.src);
                    let i = match self.open.get(&key) {
                        Some(&i) => i,
                        // The client's hello came before the capture started.
                        None => {
                            let i = self.connection(t, seg.dst, seg.src, None);
                            self.open.insert(key, i);
                            i
                        }
                    };
                    let c = &mut self.out.connections[i];
                    if h.retry {
                        c.retried = true;
                    } else {
                        c.version = Some(version_name(h.version));
                        c.cipher = Some(cipher_name(h.cipher));
                    }
                }
            }
        }
    }
}

/// Every TLS connection whose hellos are in a whole `pcap` file. A packet
/// cut short at the end (the capture stopped mid-write) is left out.
pub fn scan(pcap: &[u8], pod_ips: &[IpAddr]) -> Result<TlsScan> {
    if pcap.is_empty() {
        return Err(Error::Message("the capture is empty".into()));
    }
    let mut s = Scanner::new(pod_ips.to_vec());
    s.feed(pcap)?;
    if s.format.is_none() {
        return Err(Error::Message("the capture's header is cut short".into()));
    }
    Ok(s.out)
}

#[cfg(test)]
#[path = "tls_scan_tests.rs"]
mod tests;
