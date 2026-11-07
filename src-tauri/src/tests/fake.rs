//! A fake Kubernetes API server on 127.0.0.1, for code that needs a real
//! connection: clients built from a kubeconfig (`connect`), and exec
//! websockets (shells, uploads, downloads, captures), which the in-memory
//! `MockApi` can't upgrade. Plain requests get canned JSON; each exec takes the
//! next script of steps (send bytes on a channel, wait for the client, send
//! the exit `Status`). Runs on its own threads, so it serves any runtime.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

/// What the client has sent on an exec so far.
#[derive(Debug, Default, Clone)]
pub struct Seen {
    pub stdin: Vec<u8>,
    /// Terminal size messages, as sent (JSON).
    pub resizes: Vec<String>,
    /// The client closed the websocket or the connection.
    pub closed: bool,
}

pub enum Step {
    /// Bytes on a channel: 1 stdout, 2 stderr.
    Out(u8, Vec<u8>),
    /// Reads from the client until this holds (or it goes away).
    Wait(fn(&Seen) -> bool),
    /// The exit status, on channel 3.
    Status(Value),
}

pub fn out(text: &str) -> Step {
    Step::Out(1, text.as_bytes().to_vec())
}

pub fn success() -> Step {
    Step::Status(json!({"metadata": {}, "status": "Success"}))
}

pub fn failure(message: &str) -> Step {
    Step::Status(json!({"metadata": {}, "status": "Failure", "reason": "InternalError", "message": message}))
}

type Hook = Box<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
struct Routes {
    /// `METHOD /path` to (status, body).
    plain: HashMap<String, (u16, Value)>,
    /// Paths that never answer.
    hang: Vec<String>,
    /// Exec scripts, used in order.
    execs: VecDeque<Vec<Step>>,
    /// Called with `METHOD /path` before answering.
    hook: Option<Hook>,
}

#[derive(Clone)]
pub struct Fake {
    pub url: String,
    routes: Arc<Mutex<Routes>>,
    /// Request lines received, e.g. `GET /version?… HTTP/1.1`.
    pub requests: Arc<Mutex<Vec<String>>>,
    /// What the client sent on the latest exec.
    pub seen: Arc<Mutex<Seen>>,
}

impl Fake {
    pub fn start() -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let fake = Fake {
            url: format!("http://{}", listener.local_addr().unwrap()),
            routes: Default::default(),
            requests: Default::default(),
            seen: Default::default(),
        };
        let server = fake.clone();
        std::thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                let server = server.clone();
                std::thread::spawn(move || server.serve(socket));
            }
        });
        fake
    }

    pub fn on(&self, method: &str, path: &str, body: Value) -> &Self {
        self.routes.lock().unwrap().plain.insert(format!("{method} {path}"), (200, body));
        self
    }

    pub fn hang(&self, path: &str) -> &Self {
        self.routes.lock().unwrap().hang.push(path.to_owned());
        self
    }

    pub fn exec(&self, steps: Vec<Step>) -> &Self {
        self.routes.lock().unwrap().execs.push_back(steps);
        self
    }

    pub fn hook(&self, hook: impl Fn(&str) + Send + Sync + 'static) -> &Self {
        self.routes.lock().unwrap().hook = Some(Box::new(hook));
        self
    }

    /// A client for the server, as kube-rs builds it from a kubeconfig.
    pub fn client(&self) -> subduer_core::kube::Client {
        subduer_core::kube::Client::try_from(subduer_core::kube::Config::new(self.url.parse().unwrap())).unwrap()
    }

    /// Enough for a connect: the version, discovery of Pods only, and namespaces.
    pub fn cluster(&self) -> &Self {
        self.on("GET", "/version", json!({"major": "1", "minor": "31", "gitVersion": "v1.31.0"}))
            .on("GET", "/api", json!({"kind": "APIVersions", "versions": ["v1"], "serverAddressByClientCIDRs": []}))
            .on("GET", "/api/v1", json!({"kind": "APIResourceList", "groupVersion": "v1", "resources": [
                {"name": "pods", "singularName": "pod", "namespaced": true, "kind": "Pod", "verbs": ["get", "list", "watch"]},
            ]}))
            .on("GET", "/apis", json!({"kind": "APIGroupList", "apiVersion": "v1", "groups": []}))
            .on("GET", "/api/v1/namespaces", json!({"apiVersion": "v1", "kind": "NamespaceList", "metadata": {}, "items": [
                {"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "shop"}},
            ]}))
    }

    fn serve(&self, socket: TcpStream) {
        socket.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        let mut read = BufReader::new(socket.try_clone().unwrap());
        let mut write = socket;
        let mut line = String::new();
        if read.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let request = line.trim_end().to_owned();
        self.requests.lock().unwrap().push(request.clone());
        let (mut key, mut length, mut upgrade) = (String::new(), 0usize, false);
        loop {
            line.clear();
            if read.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap_or_default();
            match name.to_ascii_lowercase().as_str() {
                "sec-websocket-key" => key = value.trim().to_owned(),
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "upgrade" => upgrade = true,
                _ => {}
            }
        }
        let mut body = vec![0u8; length];
        let _ = read.read_exact(&mut body);
        let mut words = request.split(' ');
        let method = words.next().unwrap_or_default();
        let path = words.next().unwrap_or_default().split('?').next().unwrap_or_default();
        let route = format!("{method} {path}");
        let routes = self.routes.lock().unwrap();
        if let Some(hook) = &routes.hook {
            hook(&route);
        }
        if routes.hang.iter().any(|h| h == path) {
            drop(routes);
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        let mut routes = routes;
        // Execs without a script are refused, as RBAC would.
        if upgrade && let Some(steps) = routes.execs.pop_front() {
            drop(routes);
            *self.seen.lock().unwrap() = Seen::default();
            return session(read, write, &key, steps, &self.seen);
        }
        let status = |code: u16, reason: &str, message: String| {
            (code, json!({"kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure", "message": message, "reason": reason, "code": code}))
        };
        let (status, body) = match routes.plain.get(&route) {
            _ if upgrade => status(403, "Forbidden", format!("{route} is forbidden")),
            Some(found) => found.clone(),
            None => status(404, "NotFound", format!("{route} isn't faked")),
        };
        drop(routes);
        let body = body.to_string();
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = write.write_all(response.as_bytes());
    }
}

fn session(mut read: BufReader<TcpStream>, mut write: TcpStream, key: &str, steps: Vec<Step>, seen: &Mutex<Seen>) {
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: v5.channel.k8s.io\r\n\r\n",
        accept_key(key)
    );
    let _ = write.write_all(response.as_bytes());
    for step in steps {
        match step {
            Step::Out(channel, bytes) => {
                let mut payload = vec![channel];
                payload.extend(bytes);
                let _ = write.write_all(&frame(0x2, &payload));
            }
            Step::Status(status) => {
                let mut payload = vec![3];
                payload.extend(status.to_string().into_bytes());
                let _ = write.write_all(&frame(0x2, &payload));
            }
            Step::Wait(done) => {
                while !done(&seen.lock().unwrap()) && !seen.lock().unwrap().closed {
                    read_message(&mut read, seen);
                }
            }
        }
    }
    let _ = write.write_all(&frame(0x8, &[]));
    let _ = write.shutdown(std::net::Shutdown::Write);
    // Drain until the client goes away, so it never sees a reset.
    while !seen.lock().unwrap().closed {
        read_message(&mut read, seen);
    }
}

/// Reads one frame from the client and records it.
fn read_message(read: &mut impl Read, seen: &Mutex<Seen>) {
    let mut head = [0u8; 2];
    if read.read_exact(&mut head).is_err() {
        seen.lock().unwrap().closed = true;
        return;
    }
    let opcode = head[0] & 0x0f;
    let mut len = u64::from(head[1] & 0x7f);
    if len == 126 {
        let mut b = [0u8; 2];
        let _ = read.read_exact(&mut b);
        len = u64::from(u16::from_be_bytes(b));
    } else if len == 127 {
        let mut b = [0u8; 8];
        let _ = read.read_exact(&mut b);
        len = u64::from_be_bytes(b);
    }
    // Client frames are always masked.
    let mut mask = [0u8; 4];
    let _ = read.read_exact(&mut mask);
    let mut payload = vec![0u8; len as usize];
    let _ = read.read_exact(&mut payload);
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    let mut seen = seen.lock().unwrap();
    match (opcode, payload.split_first()) {
        (0x8, _) => seen.closed = true,
        (0x2, Some((0, data))) => seen.stdin.extend_from_slice(data),
        (0x2, Some((4, data))) => seen.resizes.push(String::from_utf8_lossy(data).into_owned()),
        _ => {}
    }
}

/// One unmasked, final server frame.
fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => f.push(n as u8),
        n => {
            f.push(126);
            f.extend((n as u16).to_be_bytes());
        }
    }
    f.extend_from_slice(payload);
    f
}

fn accept_key(key: &str) -> String {
    base64(&sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()))
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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
