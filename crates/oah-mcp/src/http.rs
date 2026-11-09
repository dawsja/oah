//! MCP over HTTP: the Streamable HTTP transport (MCP 2025-03-26 and later), for this computer
//! and your private network, beside the local socket that `oah mcp` (stdio) uses.
//!
//! `POST /mcp` takes one JSON-RPC message (or a batch). A request's reply comes back as JSON;
//! a tool call (which can wait minutes on a chat) comes back as an event stream instead: its
//! progress as it goes, comments to keep the connection alive, then the reply. Notifications get
//! 202. `DELETE /mcp` ends a session. There's no `GET` stream (oah sends nothing unasked): 405.
//!
//! Who gets in, checked in this order:
//! - It listens on 127.0.0.1, or (Settings → MCP → other devices) on 0.0.0.0 for loopback and
//!   private-network peers only. The `Host` must be
//!   `localhost` or a loopback/private IP at its port, so a web page can't reach it through a
//!   rebound DNS name.
//! - No browsers: anything with an `Origin`, or a `Sec-Fetch-Site` other than `none`, is refused
//!   (a page's requests always carry them; Node's fetch sends only `Sec-Fetch-Mode`), the body
//!   must be `application/json`, and there are no CORS headers.
//! - `Authorization: Bearer <token>`: a 256-bit random token (Settings → MCP), compared in
//!   constant time. A wrong one gets a bare 401 after a short delay; past [`FAIL_MAX`] in
//!   [`FAIL_WINDOW`], wrong tokens get 429 (the right one still works, so a noisy process can't
//!   lock you out).
//! - All of that is checked on the request's head, before any of its body is read. A request
//!   has [`REQUEST_DEADLINE`] to arrive whole, and at most [`MAX_CONNECTIONS`] are served at once.
//! - Sessions (`Mcp-Session-Id`) are random, end after [`SESSION_IDLE`] unused, and all end
//!   when the token changes.
//!
//! What happened (clients connecting, refusals, tool calls) is kept for Settings → MCP.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use crate::server::{reply_to, VERSIONS};
use crate::tools::Ctx;

/// Where clients post.
pub const PATH: &str = "/mcp";
/// Wrong tokens allowed in [`FAIL_WINDOW`] before they get 429.
pub const FAIL_MAX: usize = 20;
pub const FAIL_WINDOW: Duration = Duration::from_secs(60);
/// A session unused this long is gone (the client starts a new one).
pub const SESSION_IDLE: Duration = Duration::from_secs(60 * 60);
/// A comment on a tool call's event stream this often, so nothing in between times it out.
const KEEPALIVE: Duration = Duration::from_secs(15);
/// An idle keep-alive connection, or one sending a request too slowly, is closed after this.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// How long one request (head and body) may take to arrive, however it trickles in.
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(60);
/// Connections served at once; more are closed straight away.
pub const MAX_CONNECTIONS: usize = 64;
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 4 * 1024 * 1024;
/// A chunk-size line or a trailer.
const MAX_LINE: usize = 4 * 1024;
const MAX_TRAILERS: usize = 32;
/// Entries kept in the activity log.
const ACTIVITY: usize = 60;

/// A random token: `oah_` and 64 hex digits (256 bits).
pub fn new_token() -> String {
    format!("oah_{}", random_hex(32))
}

fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("the OS random source");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Something that happened, for Settings → MCP.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// Unix ms of the latest one.
    pub at: u64,
    /// `connected`, `closed`, `call` or `refused`.
    pub kind: &'static str,
    pub detail: String,
    /// The same thing this many times in a row.
    pub count: u32,
}

struct Session {
    /// The client's name and version, from `initialize`.
    client: String,
    last: Instant,
    /// Requests the client cancelled: their replies aren't sent.
    cancelled: HashSet<String>,
}

/// The token, sessions, failure count and activity, shared by the server and the app.
pub struct HttpState {
    token: RwLock<String>,
    sessions: Mutex<HashMap<String, Session>>,
    failures: Mutex<VecDeque<Instant>>,
    activity: Mutex<VecDeque<Activity>>,
    /// How long a wrong token waits for its 401.
    tarpit: Duration,
}

impl HttpState {
    pub fn new(token: String) -> HttpState {
        HttpState::with_tarpit(token, Duration::from_millis(300))
    }

    fn with_tarpit(token: String, tarpit: Duration) -> HttpState {
        HttpState { token: RwLock::new(token), sessions: Mutex::default(), failures: Mutex::default(), activity: Mutex::default(), tarpit }
    }

    pub fn token(&self) -> String {
        self.token.read().unwrap().clone()
    }

    /// A new token: every session ends, and clients need the new one.
    pub fn set_token(&self, token: String) {
        *self.token.write().unwrap() = token;
        let ended = std::mem::take(&mut *self.sessions.lock().unwrap()).len();
        self.log("closed", format!("Token changed; {ended} session{} ended", if ended == 1 { "" } else { "s" }));
    }

    /// Sessions used within [`SESSION_IDLE`].
    pub fn sessions(&self) -> usize {
        self.sessions.lock().unwrap().values().filter(|s| s.last.elapsed() < SESSION_IDLE).count()
    }

    /// The activity log, newest first.
    pub fn activity(&self) -> Vec<Activity> {
        self.activity.lock().unwrap().iter().rev().cloned().collect()
    }

    fn log(&self, kind: &'static str, detail: String) {
        let mut a = self.activity.lock().unwrap();
        let at = now_ms();
        // The same thing again soon after: counted, not listed twice.
        if let Some(last) = a.back_mut().filter(|l| l.kind == kind && l.detail == detail && at.saturating_sub(l.at) < 60_000) {
            last.at = at;
            last.count += 1;
            return;
        }
        a.push_back(Activity { at, kind, detail, count: 1 });
        while a.len() > ACTIVITY {
            a.pop_front();
        }
    }

    fn token_ok(&self, header: Option<&str>) -> bool {
        use subtle::ConstantTimeEq;
        let Some(given) = header.and_then(|h| h.strip_prefix("Bearer ").or(h.strip_prefix("bearer "))) else { return false };
        let token = self.token.read().unwrap();
        !token.is_empty() && given.trim().as_bytes().ct_eq(token.as_bytes()).into()
    }

    /// Counts a wrong token; true when there have been too many lately.
    fn fail(&self) -> bool {
        let mut f = self.failures.lock().unwrap();
        while f.front().is_some_and(|t| t.elapsed() > FAIL_WINDOW) {
            f.pop_front();
        }
        f.push_back(Instant::now());
        f.len() > FAIL_MAX
    }

    fn purge(&self) {
        self.sessions.lock().unwrap().retain(|_, s| s.last.elapsed() < SESSION_IDLE);
    }
}

/// The running listener. Dropping it stops it (open connections finish what they're doing).
pub struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Loopback, private LAN (RFC 1918), link-local or VPN/Tailscale (CGNAT 100.64.0.0/10, IPv6 ULA).
fn private_or_loopback(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
        }
        IpAddr::V6(v6) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// Listens on `port` (0: any free port) until the [`Server`] is dropped: on 127.0.0.1 only, or
/// with `lan` on every address for loopback and private-network peers.
pub fn start(port: u16, lan: bool, ctx: Arc<Ctx>, state: Arc<HttpState>) -> io::Result<Server> {
    let listener = TcpListener::bind((if lan { Ipv4Addr::UNSPECIFIED } else { Ipv4Addr::LOCALHOST }, port))?;
    listener.set_nonblocking(true)?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, listener.local_addr()?.port()));
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let open = Arc::new(AtomicUsize::new(0));
    let thread = thread::Builder::new().name("oah-mcp-http".into()).spawn(move || {
        while !s.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((conn, peer)) => {
                    // On Windows an accepted socket inherits the listener's non-blocking mode;
                    // a connection waiting for its next request must block.
                    if !private_or_loopback(peer.ip()) || conn.set_nonblocking(false).is_err() {
                        continue;
                    }
                    if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                        open.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                    let (ctx, state, held, stop) = (ctx.clone(), state.clone(), open.clone(), s.clone());
                    let spawned = thread::Builder::new().spawn(move || {
                        connection(conn, addr.port(), &ctx, &state, &stop);
                        held.fetch_sub(1, Ordering::SeqCst);
                    });
                    if spawned.is_err() {
                        open.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(50)),
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }
    })?;
    Ok(Server { addr, stop, thread: Some(thread) })
}

// ---- Requests --------------------------------------------------------------------------------

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// A status, for a request that can't be read.
struct Refuse(u16);

/// Reads from `stream` into `buf`, giving up at `deadline`. `Ok(0)`: the client closed.
fn read_some(stream: &mut TcpStream, buf: &mut Vec<u8>, deadline: Instant) -> io::Result<usize> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(io::ErrorKind::TimedOut.into());
    }
    stream.set_read_timeout(Some(left.min(READ_TIMEOUT)))?;
    let mut chunk = [0u8; 8192];
    let n = stream.read(&mut chunk)?;
    buf.extend_from_slice(&chunk[..n]);
    Ok(n)
}

/// Reads one request's head from `stream`; `buf` keeps what came after it (the body, and maybe
/// the next request). `None`: the client closed, or went quiet between requests.
fn read_head(stream: &mut TcpStream, buf: &mut Vec<u8>) -> Result<Option<(Request, Instant)>, Refuse> {
    // Waiting for the next request on a kept connection is idle time, not a slow request.
    let mut deadline = Instant::now() + READ_TIMEOUT;
    let mut started = !buf.is_empty();
    if started {
        deadline = Instant::now() + REQUEST_DEADLINE;
    }
    let head_len = loop {
        if let Some(i) = find(buf, b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return Err(Refuse(431));
        }
        match read_some(stream, buf, deadline) {
            Ok(0) => return Ok(None),
            Ok(_) if !started => {
                started = true;
                deadline = Instant::now() + REQUEST_DEADLINE;
            }
            Ok(_) => {}
            Err(_) if started => return Err(Refuse(408)),
            Err(_) => return Ok(None),
        }
    };
    let mut raw = [httparse::EMPTY_HEADER; 64];
    let mut parsed = httparse::Request::new(&mut raw);
    match parsed.parse(&buf[..head_len]) {
        Ok(httparse::Status::Complete(_)) => {}
        _ => return Err(Refuse(400)),
    }
    let method = parsed.method.unwrap_or("").to_string();
    let path = parsed.path.unwrap_or("").to_string();
    let headers: Vec<(String, String)> =
        parsed.headers.iter().map(|h| (h.name.to_ascii_lowercase(), String::from_utf8_lossy(h.value).trim().to_string())).collect();
    buf.drain(..head_len);
    Ok(Some((Request { method, path, headers, body: Vec::new() }, deadline)))
}

/// Reads the body of `req` (whose head has been read) by `deadline`.
fn read_body(stream: &mut TcpStream, buf: &mut Vec<u8>, req: &Request, deadline: Instant) -> Result<Vec<u8>, Refuse> {
    if req.header("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
        return read_chunked(stream, buf, deadline);
    }
    let len: usize = req.header("content-length").map(|l| l.parse().map_err(|_| Refuse(400))).transpose()?.unwrap_or(0);
    if len > MAX_BODY {
        return Err(Refuse(413));
    }
    fill(stream, buf, len, deadline)?;
    Ok(buf.drain(..len).collect())
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Reads until `buf` holds at least `len` bytes.
fn fill(stream: &mut TcpStream, buf: &mut Vec<u8>, len: usize, deadline: Instant) -> Result<(), Refuse> {
    while buf.len() < len {
        match read_some(stream, buf, deadline) {
            Ok(0) | Err(_) => return Err(Refuse(400)),
            Ok(_) => {}
        }
    }
    Ok(())
}

/// The length of the next line in `buf` (up to its CRLF), reading more as needed; lines longer
/// than [`MAX_LINE`] are refused.
fn line(stream: &mut TcpStream, buf: &mut Vec<u8>, deadline: Instant) -> Result<usize, Refuse> {
    loop {
        if let Some(i) = find(&buf[..buf.len().min(MAX_LINE + 2)], b"\r\n") {
            return Ok(i);
        }
        if buf.len() > MAX_LINE {
            return Err(Refuse(400));
        }
        let have = buf.len();
        fill(stream, buf, have + 1, deadline)?;
    }
}

fn read_chunked(stream: &mut TcpStream, buf: &mut Vec<u8>, deadline: Instant) -> Result<Vec<u8>, Refuse> {
    let mut body = Vec::new();
    loop {
        let line_end = line(stream, buf, deadline)?;
        let size_text = String::from_utf8_lossy(&buf[..line_end]).to_string();
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| Refuse(400))?;
        buf.drain(..line_end + 2);
        if size == 0 {
            // Trailers, if any, end with an empty line.
            for _ in 0..=MAX_TRAILERS {
                let i = line(stream, buf, deadline)?;
                buf.drain(..i + 2);
                if i == 0 {
                    return Ok(body);
                }
            }
            return Err(Refuse(400));
        }
        // `body` never exceeds MAX_BODY, so this can't underflow; and `size` is checked before
        // any arithmetic on it.
        if size > MAX_BODY - body.len() {
            return Err(Refuse(413));
        }
        fill(stream, buf, size + 2, deadline)?;
        if &buf[size..size + 2] != b"\r\n" {
            return Err(Refuse(400));
        }
        body.extend_from_slice(&buf[..size]);
        buf.drain(..size + 2);
    }
}

// ---- Responses -------------------------------------------------------------------------------

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    }
}

fn head(status: u16, headers: &[(&str, String)]) -> String {
    let mut h = format!("HTTP/1.1 {status} {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n", reason(status));
    for (k, v) in headers {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h
}

fn respond(stream: &mut TcpStream, status: u16, headers: &[(&str, String)], body: &[u8], keep: bool) -> io::Result<()> {
    let mut h = head(status, headers);
    h.push_str(&format!("Content-Length: {}\r\nConnection: {}\r\n\r\n", body.len(), if keep { "keep-alive" } else { "close" }));
    let mut out = h.into_bytes();
    out.extend_from_slice(body);
    stream.write_all(&out)?;
    stream.flush()
}

fn json_body(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

/// A JSON-RPC error with no id, as the body of a refusal.
fn rpc_error(code: i64, message: &str) -> Vec<u8> {
    json_body(&json!({"jsonrpc": "2.0", "id": null, "error": {"code": code, "message": message}}))
}

// ---- Serving ---------------------------------------------------------------------------------

fn connection(mut stream: TcpStream, port: u16, ctx: &Arc<Ctx>, state: &Arc<HttpState>, stop: &AtomicBool) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
    let mut buf = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        let (req, deadline) = match read_head(&mut stream, &mut buf) {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(Refuse(status)) => {
                let _ = respond(&mut stream, status, &[], &[], false);
                return;
            }
        };
        let close = req.header("connection").is_some_and(|c| c.eq_ignore_ascii_case("close"));
        match serve(&mut stream, &mut buf, req, deadline, port, ctx, state) {
            Ok(true) if !close => continue,
            _ => return,
        }
    }
}

/// The `Host` header names this server by `localhost` or a loopback/private IP (never a DNS name).
fn local_host(host: Option<&str>, port: u16) -> bool {
    let Some(h) = host.map(|h| h.trim().to_ascii_lowercase()) else { return false };
    let name = match h.strip_suffix(&format!(":{port}")) {
        Some(n) => n,
        None if port == 80 => h.as_str(),
        None => return false,
    };
    if name == "localhost" {
        return true;
    }
    let raw = name.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(name);
    raw.parse::<IpAddr>().is_ok_and(private_or_loopback)
}

/// A web page's request: it names an origin, or the browser says where it came from.
fn from_a_browser(req: &Request) -> bool {
    req.header("origin").is_some() || req.header("sec-fetch-site").is_some_and(|s| !s.eq_ignore_ascii_case("none"))
}

/// Answers one request whose head has been read; its body is read only once it's let in.
/// Ok(true): the connection can take another.
fn serve(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
    mut req: Request,
    deadline: Instant,
    port: u16,
    ctx: &Arc<Ctx>,
    state: &Arc<HttpState>,
) -> io::Result<bool> {
    // A web page: never, whatever else it sends.
    if !local_host(req.header("host"), port) || from_a_browser(&req) {
        state.log("refused", "A request from a browser or another host".into());
        respond(stream, 403, &[], &[], false)?;
        return Ok(false);
    }
    if req.path.split('?').next() != Some(PATH) {
        // Its body is never read, so the connection can't take another request.
        respond(stream, 404, &[], &[], false)?;
        return Ok(false);
    }
    if !state.token_ok(req.header("authorization")) {
        let throttled = state.fail();
        state.log("refused", if req.header("authorization").is_some() { "A wrong token".into() } else { "No token".into() });
        thread::sleep(state.tarpit);
        if throttled {
            respond(stream, 429, &[("Retry-After", FAIL_WINDOW.as_secs().to_string())], &[], false)?;
        } else {
            respond(stream, 401, &[("WWW-Authenticate", "Bearer realm=\"oah\"".into())], &[], false)?;
        }
        return Ok(false);
    }
    req.body = match read_body(stream, buf, &req, deadline) {
        Ok(b) => b,
        Err(Refuse(status)) => {
            respond(stream, status, &[], &[], false)?;
            return Ok(false);
        }
    };
    state.purge();
    match req.method.as_str() {
        "POST" => post(stream, req, ctx, state),
        "DELETE" => {
            let gone = req.header("mcp-session-id").and_then(|id| state.sessions.lock().unwrap().remove(id));
            match gone {
                Some(s) => {
                    state.log("closed", s.client);
                    respond(stream, 204, &[], &[], true)?;
                }
                None => respond(stream, 404, &[], &[], true)?,
            }
            Ok(true)
        }
        // Nothing is sent unasked, so there's no stream to open.
        _ => {
            respond(stream, 405, &[("Allow", "POST, DELETE".into())], &[], true)?;
            Ok(true)
        }
    }
}

fn post(stream: &mut TcpStream, req: Request, ctx: &Arc<Ctx>, state: &Arc<HttpState>) -> io::Result<bool> {
    if !req.header("content-type").is_some_and(|t| t.to_ascii_lowercase().starts_with("application/json")) {
        respond(stream, 415, &[], &rpc_error(-32600, "the body must be application/json"), true)?;
        return Ok(true);
    }
    if let Some(v) = req.header("mcp-protocol-version").filter(|v| !VERSIONS.contains(v)) {
        respond(stream, 400, &[], &rpc_error(-32600, &format!("unsupported MCP-Protocol-Version {v}")), true)?;
        return Ok(true);
    }
    let msg: Value = match serde_json::from_slice(&req.body) {
        Ok(v) => v,
        Err(e) => {
            respond(stream, 400, &[], &rpc_error(-32700, &format!("parse error: {e}")), true)?;
            return Ok(true);
        }
    };
    let batch = msg.is_array();
    let msgs = match msg {
        Value::Array(a) => a,
        m => vec![m],
    };
    let method = |m: &Value| m.get("method").and_then(Value::as_str).unwrap_or("").to_string();

    // The session: a new one with `initialize`, else the one the client names.
    let (sid, new) = match msgs.iter().find(|m| method(m) == "initialize") {
        Some(init) => {
            let id = random_hex(16);
            let info = init.pointer("/params/clientInfo");
            let client = match (info.and_then(|i| i["name"].as_str()), info.and_then(|i| i["version"].as_str())) {
                (Some(n), Some(v)) => format!("{n} {v}"),
                (Some(n), None) => n.to_string(),
                _ => "An MCP client".to_string(),
            };
            state.log("connected", client.clone());
            state.sessions.lock().unwrap().insert(id.clone(), Session { client, last: Instant::now(), cancelled: HashSet::new() });
            (id, true)
        }
        None => {
            let Some(id) = req.header("mcp-session-id").map(String::from) else {
                respond(stream, 400, &[], &rpc_error(-32600, "Mcp-Session-Id is missing: initialize first"), true)?;
                return Ok(true);
            };
            match state.sessions.lock().unwrap().get_mut(&id) {
                Some(s) => s.last = Instant::now(),
                None => {
                    respond(stream, 404, &[], &rpc_error(-32600, "no such session: initialize again"), true)?;
                    return Ok(true);
                }
            }
            (id, false)
        }
    };
    let session_header = [("Mcp-Session-Id", sid.clone())];
    let headers: &[(&str, String)] = if new { &session_header } else { &[] };

    // Notifications: a cancel is remembered; the rest need nothing.
    for m in msgs.iter().filter(|m| m.get("id").is_none()) {
        if method(m) == "notifications/cancelled" {
            if let (Some(id), Some(s)) = (m.pointer("/params/requestId"), state.sessions.lock().unwrap().get_mut(&sid)) {
                s.cancelled.insert(id.to_string());
            }
        }
    }
    let requests: Vec<Value> = msgs.into_iter().filter(|m| m.get("id").is_some() && !method(m).is_empty()).collect();
    if requests.is_empty() {
        respond(stream, 202, headers, &[], true)?;
        return Ok(true);
    }
    for r in requests.iter().filter(|r| method(r) == "tools/call") {
        state.log("call", r.pointer("/params/name").and_then(Value::as_str).unwrap_or("a tool").to_string());
    }

    let streams = req.header("accept").is_some_and(|a| a.contains("text/event-stream"));
    if !batch && streams && method(&requests[0]) == "tools/call" {
        return event_stream(stream, requests.into_iter().next().unwrap_or_default(), headers, &sid, ctx, state);
    }
    // Quick ones (and clients that only take JSON): the reply as JSON. Progress has nowhere to go.
    let replies: Vec<Value> = requests
        .into_iter()
        .map(|r| {
            let id = r.get("id").cloned().unwrap_or(Value::Null);
            reply_to(ctx, &id, &method(&r), r.get("params").cloned().unwrap_or(Value::Null), &|_| {})
        })
        .collect();
    let body = if batch { Value::Array(replies) } else { replies.into_iter().next().unwrap_or_default() };
    let mut h = headers.to_vec();
    h.push(("Content-Type", "application/json".into()));
    respond(stream, 200, &h, &json_body(&body), true)?;
    Ok(true)
}

/// A tool call's reply as an event stream: progress as it goes, a comment every
/// [`KEEPALIVE`], then the reply (none if the client cancelled it). The connection closes after.
fn event_stream(
    stream: &mut TcpStream,
    request: Value,
    headers: &[(&str, String)],
    sid: &str,
    ctx: &Arc<Ctx>,
    state: &Arc<HttpState>,
) -> io::Result<bool> {
    let mut h = head(200, headers);
    h.push_str("Content-Type: text/event-stream\r\nX-Accel-Buffering: no\r\nConnection: close\r\n\r\n");
    stream.write_all(h.as_bytes())?;
    stream.flush()?;
    let out = Arc::new(Mutex::new(stream.try_clone()?));
    let event = |out: &Mutex<TcpStream>, v: &Value| {
        let mut o = out.lock().unwrap();
        let _ = write!(o, "event: message\ndata: {v}\n\n");
        let _ = o.flush();
    };
    let done = Arc::new(AtomicBool::new(false));
    {
        let (out, done) = (out.clone(), done.clone());
        thread::spawn(move || {
            let mut waited = Duration::ZERO;
            while !done.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(250));
                waited += Duration::from_millis(250);
                if waited >= KEEPALIVE {
                    waited = Duration::ZERO;
                    let mut o = out.lock().unwrap();
                    if o.write_all(b": keepalive\n\n").and_then(|_| o.flush()).is_err() {
                        return;
                    }
                }
            }
        });
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("").to_string();
    let notify = |v: &Value| event(&out, v);
    let reply = reply_to(ctx, &id, &method, request.get("params").cloned().unwrap_or(Value::Null), &notify);
    done.store(true, Ordering::SeqCst);
    let cancelled = state.sessions.lock().unwrap().get_mut(sid).is_some_and(|s| s.cancelled.remove(&id.to_string()));
    if !cancelled {
        event(&out, &reply);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fake;

    const TOKEN: &str = "oah_test-token";

    fn server() -> (Server, Arc<HttpState>) {
        let (ctx, _) = fake(true);
        let state = Arc::new(HttpState::with_tarpit(TOKEN.into(), Duration::ZERO));
        (start(0, true, ctx, state.clone()).unwrap(), state)
    }

    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl Reply {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
        }
        fn json(&self) -> Value {
            serde_json::from_str(&self.body).unwrap_or(Value::Null)
        }
        /// The `data:` of each event in an event stream.
        fn events(&self) -> Vec<Value> {
            self.body.lines().filter_map(|l| l.strip_prefix("data: ")).map(|d| serde_json::from_str(d).unwrap()).collect()
        }
    }

    /// One request on its own connection; `headers` replace the usual ones by name.
    fn send(s: &Server, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Reply {
        let port = s.addr().port();
        let host = format!("127.0.0.1:{port}");
        let auth = format!("Bearer {TOKEN}");
        let mut all: Vec<(&str, &str)> = vec![
            ("Host", &host),
            ("Authorization", &auth),
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
        ];
        for (k, v) in headers {
            all.retain(|(n, _)| !n.eq_ignore_ascii_case(k));
            if !v.is_empty() {
                all.push((k, v));
            }
        }
        let mut req = format!("{method} {path} HTTP/1.1\r\n");
        for (k, v) in &all {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()));
        let mut c = TcpStream::connect(s.addr()).unwrap();
        c.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        let (head, body) = out.split_once("\r\n\r\n").unwrap_or((&out, ""));
        let mut lines = head.lines();
        let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
        let headers = lines.filter_map(|l| l.split_once(": ")).map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Reply { status, headers, body: body.to_string() }
    }

    fn rpc(id: i64, method: &str, params: Value) -> String {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
    }

    fn initialize(s: &Server) -> String {
        let r = send(
            s,
            "POST",
            PATH,
            &[],
            &rpc(
                1,
                "initialize",
                json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test-client", "version": "9"}}),
            ),
        );
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.json()["result"]["serverInfo"]["name"], "oah");
        r.header("Mcp-Session-Id").expect("a session id").to_string()
    }

    #[test]
    fn needs_the_token() {
        let (s, state) = server();
        let body = rpc(1, "initialize", json!({}));
        let r = send(&s, "POST", PATH, &[("Authorization", "")], &body);
        assert_eq!((r.status, r.header("WWW-Authenticate")), (401, Some("Bearer realm=\"oah\"")));
        assert!(r.body.is_empty(), "nothing said about why");
        assert_eq!(send(&s, "POST", PATH, &[("Authorization", "Bearer oah_nope")], &body).status, 401);
        assert_eq!(send(&s, "POST", PATH, &[("Authorization", "Basic b2FoOnRlc3Q=")], &body).status, 401);
        let refused: Vec<_> = state.activity().into_iter().filter(|a| a.kind == "refused").collect();
        assert_eq!(refused.iter().map(|a| a.count).sum::<u32>(), 3);
    }

    #[test]
    fn refuses_browsers_and_other_hosts() {
        let (s, _) = server();
        let body = rpc(1, "initialize", json!({}));
        assert_eq!(send(&s, "POST", PATH, &[("Origin", "https://evil.example")], &body).status, 403);
        assert_eq!(send(&s, "POST", PATH, &[("Sec-Fetch-Site", "cross-site")], &body).status, 403);
        // A rebound DNS name or public IP.
        assert_eq!(send(&s, "POST", PATH, &[("Host", "evil.example:80")], &body).status, 403);
        assert_eq!(send(&s, "POST", PATH, &[("Host", "")], &body).status, 403);
        let port = s.addr().port().to_string();
        assert_eq!(send(&s, "POST", PATH, &[("Host", &format!("evil.example:{port}"))], &body).status, 403);
        assert_eq!(send(&s, "POST", PATH, &[("Host", &format!("8.8.8.8:{port}"))], &body).status, 403);
        assert_eq!(send(&s, "POST", PATH, &[("Host", &format!("localhost:{port}"))], &body).status, 200);
        assert_eq!(send(&s, "POST", PATH, &[("Host", &format!("192.168.1.20:{port}"))], &body).status, 200);
        assert_eq!(send(&s, "POST", PATH, &[("Host", &format!("100.100.1.2:{port}"))], &body).status, 200);
        assert_eq!(send(&s, "POST", "/other", &[], &body).status, 404);
        assert_eq!(send(&s, "POST", PATH, &[("Content-Type", "text/plain")], &body).status, 415);
        // Node's fetch (undici, the MCP TypeScript SDK) sends Sec-Fetch-Mode on its own.
        assert_eq!(send(&s, "POST", PATH, &[("Sec-Fetch-Mode", "cors")], &body).status, 200);
        assert_eq!(send(&s, "POST", PATH, &[("Sec-Fetch-Site", "none")], &body).status, 200);
    }

    /// Writes `raw` on a new connection and returns the status line's code (0: no answer).
    fn raw(s: &Server, raw: &[u8]) -> u16 {
        let mut c = TcpStream::connect(s.addr()).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let _ = c.write_all(raw);
        let mut out = String::new();
        let _ = c.read_to_string(&mut out);
        out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
    }

    fn chunked(s: &Server, auth: &str, body: &str) -> u16 {
        let port = s.addr().port();
        let head = format!(
            "POST {PATH} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{auth}Content-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
        );
        raw(s, format!("{head}{body}").as_bytes())
    }

    #[test]
    fn malformed_chunk_sizes_are_refused_not_a_crash() {
        let (s, _) = server();
        let auth = format!("Authorization: Bearer {TOKEN}\r\n");
        // A size that overflows `body.len() + size` (it used to panic, and abort the app).
        assert_eq!(chunked(&s, &auth, "1\r\nA\r\nffffffffffffffff\r\nB"), 413);
        assert_eq!(chunked(&s, &auth, "3\r\nABC\r\nfffffffffffffffd\r\nB"), 413);
        // A size line that never ends.
        assert_eq!(chunked(&s, &auth, &"1".repeat(MAX_LINE + 10)), 400);
        // Data longer than its size.
        assert_eq!(chunked(&s, &auth, "1\r\nABC\r\n0\r\n\r\n"), 400);
        // The server is still up.
        let init = rpc(1, "initialize", json!({}));
        let ok = format!("{:x}\r\n{init}\r\n0\r\n\r\n", init.len());
        assert_eq!(chunked(&s, &auth, &ok), 200);
    }

    #[test]
    fn nothing_of_the_body_is_read_before_the_token() {
        let (s, _) = server();
        // No token, and a body that would be refused if it were read: 401 first.
        assert_eq!(chunked(&s, "", "ffffffffffffffff\r\n"), 401);
        let port = s.addr().port();
        let big = format!("POST {PATH} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\n\r\n", MAX_BODY * 10);
        assert_eq!(raw(&s, big.as_bytes()), 401);
    }

    #[test]
    fn too_many_wrong_tokens_get_429_but_the_right_one_still_works() {
        let (s, _) = server();
        let body = rpc(1, "initialize", json!({}));
        let statuses: Vec<u16> =
            (0..FAIL_MAX + 2).map(|_| send(&s, "POST", PATH, &[("Authorization", "Bearer guess")], &body).status).collect();
        assert_eq!(statuses[0], 401);
        assert_eq!(*statuses.last().unwrap(), 429);
        assert_eq!(send(&s, "POST", PATH, &[], &body).status, 200);
    }

    #[test]
    fn sessions_from_initialize_to_delete() {
        let (s, state) = server();
        let sid = initialize(&s);
        let with = [("Mcp-Session-Id", sid.as_str()), ("MCP-Protocol-Version", "2025-06-18")];
        // Notifications are accepted with no body.
        let r = send(&s, "POST", PATH, &with, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string());
        assert_eq!((r.status, r.body.as_str()), (202, ""));
        let r = send(&s, "POST", PATH, &with, &rpc(2, "tools/list", json!({})));
        assert_eq!(r.header("Content-Type"), Some("application/json"));
        assert!(r.json()["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "create_session"));
        // No session, an unknown one, an unknown protocol version.
        assert_eq!(send(&s, "POST", PATH, &[], &rpc(3, "tools/list", json!({}))).status, 400);
        assert_eq!(send(&s, "POST", PATH, &[("Mcp-Session-Id", "nope")], &rpc(3, "tools/list", json!({}))).status, 404);
        assert_eq!(
            send(&s, "POST", PATH, &[("Mcp-Session-Id", &sid), ("MCP-Protocol-Version", "1999-01-01")], &rpc(3, "ping", json!({}))).status,
            400
        );
        // No server-sent stream.
        assert_eq!(send(&s, "GET", PATH, &with, "").status, 405);
        assert_eq!(state.sessions(), 1);
        assert_eq!(send(&s, "DELETE", PATH, &with, "").status, 204);
        assert_eq!(send(&s, "POST", PATH, &with, &rpc(4, "ping", json!({}))).status, 404);
        let kinds: Vec<&str> = state.activity().iter().map(|a| a.kind).collect();
        assert_eq!(kinds, ["closed", "connected"]);
        assert!(state.activity()[1].detail == "test-client 9");
    }

    #[test]
    fn tool_calls_stream_progress_then_the_reply() {
        let (s, state) = server();
        let sid = initialize(&s);
        let with = [("Mcp-Session-Id", sid.as_str())];
        let call = rpc(
            5,
            "tools/call",
            json!({"name": "create_session", "arguments": {"project_id": "p1", "prompt": "hello"}, "_meta": {"progressToken": "t"}}),
        );
        let r = send(&s, "POST", PATH, &with, &call);
        assert_eq!((r.status, r.header("Content-Type")), (200, Some("text/event-stream")));
        let events = r.events();
        let last = events.last().unwrap();
        assert_eq!(last["id"], 5);
        assert!(last["result"]["content"][0]["text"].as_str().unwrap().contains("You said hello"), "{last}");
        assert!(events[..events.len() - 1].iter().all(|e| e["method"] == "notifications/progress"));
        // A client that only takes JSON gets JSON.
        let r = send(
            &s,
            "POST",
            PATH,
            &[("Mcp-Session-Id", &sid), ("Accept", "application/json")],
            &rpc(6, "tools/call", json!({"name": "list_projects", "arguments": {}})),
        );
        assert!(r.json()["result"]["content"][0]["text"].as_str().unwrap().contains("\"p1\""));
        assert!(state.activity().iter().any(|a| a.kind == "call" && a.detail == "create_session"));
    }

    /// One response off a kept-alive connection: its head, then as much body as it says.
    fn read_response(c: &mut TcpStream) -> String {
        let mut got = Vec::new();
        let mut b = [0u8; 4096];
        loop {
            let n = c.read(&mut b).unwrap();
            got.extend_from_slice(&b[..n]);
            let text = String::from_utf8_lossy(&got).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let len: usize = head.lines().find_map(|l| l.strip_prefix("Content-Length: ")).unwrap().parse().unwrap();
                if body.len() >= len {
                    return text;
                }
            }
        }
    }

    #[test]
    fn keeps_a_connection_for_several_requests() {
        let (s, _) = server();
        let sid = initialize(&s);
        let mut c = TcpStream::connect(s.addr()).unwrap();
        let port = s.addr().port();
        for id in [1, 2] {
            let body = rpc(id, "ping", json!({}));
            write!(c, "POST {PATH} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nMcp-Session-Id: {sid}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            let text = read_response(&mut c);
            assert!(text.starts_with("HTTP/1.1 200") && text.contains(&format!("\"id\":{id}")), "{text}");
        }
        // A chunked body.
        let body = rpc(3, "ping", json!({}));
        write!(c, "POST {PATH} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nMcp-Session-Id: {sid}\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len()).unwrap();
        assert!(read_response(&mut c).contains("\"id\":3"));
    }

    #[test]
    fn a_new_token_ends_every_session() {
        let (s, state) = server();
        let sid = initialize(&s);
        state.set_token("oah_other".into());
        assert_eq!(state.sessions(), 0);
        assert_eq!(
            send(&s, "POST", PATH, &[("Mcp-Session-Id", &sid), ("Authorization", "Bearer oah_other")], &rpc(1, "ping", json!({}))).status,
            404
        );
        assert_eq!(send(&s, "POST", PATH, &[], &rpc(1, "initialize", json!({}))).status, 401, "the old token no longer works");
    }

    /// Serves the fake host for a real client to try (ignored): OAH_TEST_PORT, and the token
    /// OAH_TEST_TOKEN, for OAH_TEST_SECS seconds (default 60).
    #[test]
    #[ignore]
    fn serve_for_a_real_client() {
        let (ctx, _) = fake(true);
        let token = std::env::var("OAH_TEST_TOKEN").unwrap_or_else(|_| new_token());
        let state = Arc::new(HttpState::new(token.clone()));
        let port = std::env::var("OAH_TEST_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(0);
        let s = start(port, true, ctx, state.clone()).unwrap();
        eprintln!("serving http://{}{PATH} token {token}", s.addr());
        let secs = std::env::var("OAH_TEST_SECS").ok().and_then(|p| p.parse().ok()).unwrap_or(60);
        thread::sleep(Duration::from_secs(secs));
        for a in state.activity().iter().rev() {
            eprintln!("activity: {} {} x{}", a.kind, a.detail, a.count);
        }
    }

    #[test]
    fn tokens_are_long_and_random() {
        let (a, b) = (new_token(), new_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 4 + 64);
        assert!(a.starts_with("oah_") && a[4..].chars().all(|c| c.is_ascii_hexdigit()));
    }
}
