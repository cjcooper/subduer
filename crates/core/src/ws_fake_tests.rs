#![allow(dead_code)]
//! Test support: a tiny websocket "API server" on 127.0.0.1 that speaks the
//! `v5.channel.k8s.io` exec protocol, so exec-based code (shells, uploads,
//! downloads, browsing) can run end to end without a cluster. Each test gives
//! it a script of steps: send bytes on a channel, wait for the client, send
//! the exit `Status`. Port-forwards use the same framing (see [`forwarding`]).
//! The in-memory `MockApi` can't do this: websocket
//! upgrades need a real connection.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// What the client has sent so far.
#[derive(Debug, Default, Clone)]
pub struct Seen {
    pub stdin: Vec<u8>,
    /// The client signalled end of stdin.
    pub stdin_closed: bool,
    /// Terminal size messages, as sent (JSON).
    pub resizes: Vec<String>,
    /// The connection or websocket closed.
    pub closed: bool,
}

pub enum Step {
    /// Bytes on a channel: 1 stdout, 2 stderr.
    Out(u8, Vec<u8>),
    /// Reads from the client until this holds (or it disconnects).
    Wait(fn(&Seen) -> bool),
    /// The exit status, on channel 3.
    Status(Value),
    /// Does nothing for a while.
    Pause(Duration),
}

/// Port-forward: the first frame on each of a port's two channels (data
/// `0`, errors `1` for the first port) is the port number, little-endian.
pub fn forwarding(port: u16) -> Vec<Step> {
    vec![Step::Out(0, port.to_le_bytes().to_vec()), Step::Out(1, port.to_le_bytes().to_vec())]
}

pub struct Fake {
    pub client: kube::Client,
    /// Request lines, e.g. `GET /api/v1/…/exec?… HTTP/1.1`.
    pub requests: Arc<Mutex<Vec<String>>>,
    pub seen: Arc<Mutex<Seen>>,
}

pub fn out(text: &str) -> Step {
    Step::Out(1, text.as_bytes().to_vec())
}

pub fn err(text: &str) -> Step {
    Step::Out(2, text.as_bytes().to_vec())
}

pub fn success() -> Step {
    Step::Status(serde_json::json!({"metadata": {}, "status": "Success"}))
}

/// A non-zero exit, as the kubelet reports it.
pub fn exit(code: u32) -> Step {
    Step::Status(serde_json::json!({
        "metadata": {}, "status": "Failure", "reason": "NonZeroExitCode",
        "message": format!("command terminated with non-zero exit code: exit status {code}"),
        "details": {"causes": [{"reason": "ExitCode", "message": code.to_string()}]}
    }))
}

/// Starts the server; it answers one connection with `steps`.
pub async fn start(steps: Vec<Step>) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(vec![]));
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (r, s) = (requests.clone(), seen.clone());
    tokio::spawn(async move {
        let Ok((socket, _)) = listener.accept().await else { return };
        let _ = tokio::time::timeout(Duration::from_secs(20), session(socket, steps, r, s)).await;
    });
    let client = kube::Client::try_from(kube::Config::new(url.parse().unwrap())).unwrap();
    Fake { client, requests, seen }
}

async fn session(
    socket: tokio::net::TcpStream,
    steps: Vec<Step>,
    requests: Arc<Mutex<Vec<String>>>,
    seen: Arc<Mutex<Seen>>,
) {
    let (read, mut write) = socket.into_split();
    let mut read = BufReader::new(read);
    let mut key = String::new();
    let mut line = String::new();
    read.read_line(&mut line).await.unwrap();
    requests.lock().unwrap().push(line.trim_end().to_owned());
    loop {
        line.clear();
        if read.read_line(&mut line).await.unwrap() == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("sec-websocket-key")
        {
            key = value.trim().to_owned();
        }
    }
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: v5.channel.k8s.io\r\n\r\n",
        accept_key(&key)
    );
    write.write_all(response.as_bytes()).await.unwrap();
    for step in steps {
        match step {
            Step::Out(channel, bytes) => {
                let mut payload = vec![channel];
                payload.extend(bytes);
                let _ = write.write_all(&frame(0x2, &payload)).await;
            }
            Step::Status(status) => {
                let mut payload = vec![3];
                payload.extend(status.to_string().into_bytes());
                let _ = write.write_all(&frame(0x2, &payload)).await;
            }
            Step::Wait(done) => {
                while !done(&seen.lock().unwrap()) && !seen.lock().unwrap().closed {
                    read_message(&mut read, &seen).await;
                }
            }
            Step::Pause(time) => tokio::time::sleep(time).await,
        }
    }
    let _ = write.write_all(&frame(0x8, &[])).await;
    let _ = write.shutdown().await;
    // Drain until the client goes away, so it never sees a reset.
    while !seen.lock().unwrap().closed {
        read_message(&mut read, &seen).await;
    }
}

/// Reads one frame from the client and records it.
async fn read_message(read: &mut (impl AsyncReadExt + Unpin), seen: &Mutex<Seen>) {
    let mut head = [0u8; 2];
    if read.read_exact(&mut head).await.is_err() {
        seen.lock().unwrap().closed = true;
        return;
    }
    let opcode = head[0] & 0x0f;
    let masked = head[1] & 0x80 != 0;
    let mut len = u64::from(head[1] & 0x7f);
    if len == 126 {
        let mut b = [0u8; 2];
        read.read_exact(&mut b).await.unwrap();
        len = u64::from(u16::from_be_bytes(b));
    } else if len == 127 {
        let mut b = [0u8; 8];
        read.read_exact(&mut b).await.unwrap();
        len = u64::from_be_bytes(b);
    }
    let mut mask = [0u8; 4];
    if masked {
        read.read_exact(&mut mask).await.unwrap();
    }
    let mut payload = vec![0u8; len as usize];
    read.read_exact(&mut payload).await.unwrap();
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    let mut seen = seen.lock().unwrap();
    match (opcode, payload.split_first()) {
        (0x8, _) => seen.closed = true,
        (0x2, Some((0, data))) => seen.stdin.extend_from_slice(data),
        (0x2, Some((4, data))) => seen.resizes.push(String::from_utf8_lossy(data).into_owned()),
        (0x2, Some((255, [0]))) => seen.stdin_closed = true,
        _ => {}
    }
}

/// One unmasked, final server frame.
fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => f.push(n as u8),
        n if n <= 0xffff => {
            f.push(126);
            f.extend((n as u16).to_be_bytes());
        }
        n => {
            f.push(127);
            f.extend((n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(payload);
    f
}

fn accept_key(key: &str) -> String {
    let digest = sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    base64::engine::general_purpose::STANDARD.encode(digest)
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend(((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (slot, word) in w.iter_mut().zip(chunk.chunks(4)) {
            *slot = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*word);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (dst, word) in out.chunks_mut(4).zip(h) {
        dst.copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[test]
fn accept_key_matches_rfc_6455() {
    assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}
