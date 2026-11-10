//! `oah-host serve <tag>`: the per-user daemon. One per tag, held by a lock file; it listens on
//! the tag's socket for `oah-host proxy` connections (one per SSH connection from oah).
//!
//! Every program it starts gets its own session (`setsid`), so its process group is everything
//! it started too. Output waits in memory until oah acknowledges it, up to [`BUFFER_CAP`] per
//! stream; past that the program blocks on its next write until oah catches up.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::proto::{self, Msg, CHUNK, PROTO};
use crate::Paths;

use crate::DETACHED_LIMIT;
/// With no programs and no connections for this long, the daemon exits.
pub const IDLE_EXIT: Duration = Duration::from_secs(5 * 60);
/// Unacknowledged output kept per stream.
pub const BUFFER_CAP: usize = 32 << 20;
/// What a forgotten program gets to exit by itself (stdin closed) before its group is killed.
const FORGET_GRACE: Duration = Duration::from_secs(2);
/// After exiting, how long to wait for the last output before reporting the exit.
const DRAIN_GRACE: Duration = Duration::from_secs(1);
/// Roughly the most one write to a connection carries, and the most of it one program gets (so
/// a noisy one can't hold up the others).
const BATCH: usize = 1 << 20;
const BATCH_EACH: usize = 256 << 10;
/// A program that has exited is kept this long without a connection (for its exit status and
/// last output), not the whole [`DETACHED_LIMIT`].
const EXITED_LIMIT: Duration = Duration::from_secs(3600);

struct Shared {
    st: Mutex<State>,
    cv: Condvar,
    paths: Paths,
    daemon: u64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Default)]
struct State {
    conns: HashMap<u64, Conn>,
    next_conn: u64,
    procs: HashMap<u64, Proc>,
    /// Set once a connection brought a forwarded SSH agent.
    agent_link: Option<PathBuf>,
}

struct Conn {
    /// Encoded replies waiting for the writer.
    queue: Vec<u8>,
    /// The oah install and run on the other end (from its `Hello`).
    owner: (u64, u64),
    /// Its SSH session's forwarded agent (empty: none).
    auth_sock: String,
}

struct Proc {
    pgid: i32,
    /// The oah install and run that started it.
    owner: (u64, u64),
    conn: Option<u64>,
    detached_at: Instant,
    /// To the thread writing its stdin; dropping it closes stdin.
    stdin: Option<Sender<Vec<u8>>>,
    stdin_recv: u64,
    close_at: Option<u64>,
    streams: [Buf; 2],
    exit: Option<(Option<i32>, Option<i32>)>,
    exit_sent: bool,
    /// Forgotten: its group is killed at this time.
    kill_at: Option<Instant>,
}

#[derive(Default)]
struct Buf {
    /// Output from `base` on that oah hasn't acknowledged.
    data: Vec<u8>,
    base: u64,
    /// How far this connection has been sent.
    sent: u64,
    eof: bool,
    eof_sent: bool,
}

impl Buf {
    fn end(&self) -> u64 {
        self.base + self.data.len() as u64
    }
}

/// Serves until idle. Returns at once (`Ok`) when another daemon holds the tag's lock.
pub fn serve(tag: &str) -> io::Result<()> {
    let paths = crate::paths(tag)?;
    let lock = OpenOptions::new().create(true).truncate(false).write(true).mode(0o600).open(&paths.lock)?;
    // SAFETY: flock on a file we hold open.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(());
    }
    // We hold the lock, so a socket file left behind is a dead daemon's.
    let _ = std::fs::remove_file(&paths.sock);
    let listener = UnixListener::bind(&paths.sock)?;
    std::fs::set_permissions(&paths.sock, std::fs::Permissions::from_mode(0o600))?;
    let daemon = nonce();
    log(&format!("serving {} (pid {})", paths.sock.display(), std::process::id()));
    let sh = Arc::new(Shared { st: Mutex::new(State::default()), cv: Condvar::new(), paths, daemon });
    {
        let sh = sh.clone();
        thread::spawn(move || reaper(&sh));
    }
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let sh = sh.clone();
                thread::spawn(move || connection(&sh, s));
            }
            Err(e) => log(&format!("accept: {e}")),
        }
    }
    drop(lock);
    Ok(())
}

fn connection(sh: &Arc<Shared>, stream: UnixStream) {
    let Ok(mut rd) = stream.try_clone() else { return };
    // The proxy speaks first, with what it knows of its SSH session.
    let auth_sock = match proto::read(&mut rd) {
        Ok(Some(Msg::Proxy { auth_sock })) => auth_sock,
        _ => return,
    };
    let cid = {
        let mut st = sh.lock();
        st.next_conn += 1;
        let cid = st.next_conn;
        if !auth_sock.is_empty() && link_agent(&sh.paths, &auth_sock) {
            st.agent_link = Some(sh.paths.agent.clone());
        }
        st.conns.insert(cid, Conn { queue: Vec::new(), owner: (0, 0), auth_sock });
        cid
    };
    {
        let sh = sh.clone();
        thread::spawn(move || writer(&sh, cid, stream));
    }
    loop {
        match proto::read(&mut rd) {
            Ok(Some(m)) => handle(sh, cid, m),
            Ok(None) => break,
            Err(e) => {
                log(&format!("connection {cid}: {e}"));
                break;
            }
        }
    }
    let mut st = sh.lock();
    let gone = st.conns.remove(&cid);
    // Its agent socket goes with its session; a path left dangling could be made again by
    // someone else. Point the link at another connection's, or remove it.
    if gone.is_some_and(|c| !c.auth_sock.is_empty()) {
        let next = st.conns.values().map(|c| c.auth_sock.clone()).find(|a| !a.is_empty());
        if !next.is_some_and(|a| link_agent(&sh.paths, &a)) {
            let _ = std::fs::remove_file(&sh.paths.agent);
        }
    }
    let now = Instant::now();
    for p in st.procs.values_mut().filter(|p| p.conn == Some(cid)) {
        p.conn = None;
        p.detached_at = now;
    }
    sh.cv.notify_all();
    drop(st);
    let _ = rd.shutdown(std::net::Shutdown::Both);
}

/// Points the tag's agent link at `target` (replacing it in one step).
fn link_agent(paths: &Paths, target: &str) -> bool {
    if !std::path::Path::new(target).exists() {
        return false;
    }
    let tmp = paths.dir.join(format!(".agent-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(target, &tmp).is_ok() && std::fs::rename(&tmp, &paths.agent).is_ok()
}

fn handle(sh: &Arc<Shared>, cid: u64, m: Msg) {
    let mut st = sh.lock();
    let st = &mut *st;
    let mut replies = Vec::new();
    match m {
        Msg::Hello { proto: _, client, instance } => {
            // A newer run of the same oah: what the older one left it can't come back for.
            for p in st.procs.values_mut().filter(|p| p.owner.0 == client && p.owner.1 != instance) {
                forget(p);
            }
            if let Some(c) = st.conns.get_mut(&cid) {
                c.owner = (client, instance);
            }
            replies.push(Msg::HelloOk { proto: PROTO, daemon: sh.daemon });
        }
        Msg::Spawn { id, script } => {
            if let Some(p) = st.procs.get_mut(&id) {
                // Asked again after a dropped connection: it's already running, and oah has
                // none of what it sent on the dead one.
                attach(p, cid, 0, 0);
                replies.push(Msg::Spawned { id });
                replies.push(Msg::StdinAck { id, off: p.stdin_recv });
            } else {
                let owner = st.conns.get(&cid).map_or((0, 0), |c| c.owner);
                match start(st.agent_link.as_deref(), &script) {
                    Ok(child) => {
                        st.procs.insert(id, run(sh, id, owner, cid, child));
                        replies.push(Msg::Spawned { id });
                    }
                    Err(e) => replies.push(Msg::SpawnFailed { id, msg: e.to_string() }),
                }
            }
        }
        Msg::Stdin { id, off, data } => match st.procs.get_mut(&id) {
            None => replies.push(Msg::Gone { id }),
            Some(p) => {
                let end = off + data.len() as u64;
                if off <= p.stdin_recv && end > p.stdin_recv {
                    let new = data[(p.stdin_recv - off) as usize..].to_vec();
                    if let Some(tx) = &p.stdin {
                        let _ = tx.send(new);
                    }
                    p.stdin_recv = end;
                }
                close_if_done(p);
                replies.push(Msg::StdinAck { id, off: p.stdin_recv });
            }
        },
        Msg::CloseStdin { id, off } => match st.procs.get_mut(&id) {
            None => replies.push(Msg::Gone { id }),
            Some(p) => {
                p.close_at = Some(off);
                close_if_done(p);
            }
        },
        Msg::Attach { id, out, err } => match st.procs.get_mut(&id) {
            None => replies.push(Msg::Gone { id }),
            Some(p) => {
                attach(p, cid, out, err);
                replies.push(Msg::StdinAck { id, off: p.stdin_recv });
            }
        },
        Msg::Ack { id, out, err } => {
            if let Some(p) = st.procs.get_mut(&id) {
                for (b, off) in p.streams.iter_mut().zip([out, err]) {
                    let n = off.saturating_sub(b.base).min(b.data.len() as u64) as usize;
                    b.data.drain(..n);
                    b.base += n as u64;
                }
            }
        }
        Msg::Kill { id } => {
            if let Some(p) = st.procs.get(&id) {
                kill_group(p.pgid);
            }
        }
        Msg::Forget { id } => {
            if let Some(p) = st.procs.get_mut(&id) {
                forget(p);
            }
            // So oah knows it was heard (it says it again on every connection until then).
            replies.push(Msg::Gone { id });
        }
        Msg::Ping { n } => replies.push(Msg::Pong { n }),
        _ => {}
    }
    if let Some(c) = st.conns.get_mut(&cid) {
        for r in replies {
            c.queue.extend(r.encode());
        }
    }
    sh.cv.notify_all();
}

/// Sends this connection the output from these offsets on, and the exit status again.
fn attach(p: &mut Proc, cid: u64, out: u64, err: u64) {
    p.conn = Some(cid);
    for (b, off) in p.streams.iter_mut().zip([out, err]) {
        b.sent = off.clamp(b.base, b.end());
        b.eof_sent = false;
    }
    p.exit_sent = false;
}

fn close_if_done(p: &mut Proc) {
    if p.close_at.is_some_and(|at| p.stdin_recv >= at) {
        p.stdin = None;
    }
}

/// Stdin closed now, the group killed after [`FORGET_GRACE`] (by the reaper).
fn forget(p: &mut Proc) {
    p.stdin = None;
    p.conn = None;
    if p.kill_at.is_none() {
        p.kill_at = Some(Instant::now() + FORGET_GRACE);
    }
}

/// `sh -lc <script>` in a new session, from the home folder.
fn start(agent_link: Option<&std::path::Path>, script: &str) -> io::Result<Child> {
    let mut c = Command::new("/bin/sh");
    c.arg("-lc").arg(script).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(home) = std::env::var_os("HOME") {
        c.current_dir(home);
    }
    // The first connection's agent socket is gone once it disconnects; the link follows the
    // newest one.
    c.env_remove("SSH_AUTH_SOCK");
    if let Some(l) = agent_link {
        c.env("SSH_AUTH_SOCK", l);
    }
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        c.pre_exec(|| if libc::setsid() < 0 { Err(io::Error::last_os_error()) } else { Ok(()) });
    }
    c.spawn()
}

/// Starts the threads that feed, read and wait for a new child.
fn run(sh: &Arc<Shared>, id: u64, owner: (u64, u64), cid: u64, mut child: Child) -> Proc {
    let pgid = child.id() as i32;
    let (tx, rx) = channel();
    if let Some(stdin) = child.stdin.take() {
        thread::spawn(move || feed(rx, stdin));
    }
    if let Some(out) = child.stdout.take() {
        let sh = sh.clone();
        thread::spawn(move || pump(&sh, id, 0, out));
    }
    if let Some(err) = child.stderr.take() {
        let sh = sh.clone();
        thread::spawn(move || pump(&sh, id, 1, err));
    }
    {
        let sh = sh.clone();
        thread::spawn(move || reap(&sh, id, child));
    }
    Proc {
        pgid,
        owner,
        conn: Some(cid),
        detached_at: Instant::now(),
        stdin: Some(tx),
        stdin_recv: 0,
        close_at: None,
        streams: [Buf::default(), Buf::default()],
        exit: None,
        exit_sent: false,
        kill_at: None,
    }
}

fn feed(rx: Receiver<Vec<u8>>, mut stdin: ChildStdin) {
    for data in rx {
        if stdin.write_all(&data).and_then(|_| stdin.flush()).is_err() {
            // It stopped reading (exited); later sends just fail.
            return;
        }
    }
}

fn pump(sh: &Shared, id: u64, s: usize, mut r: impl Read) {
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = match r.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => 0,
        };
        let mut st = sh.lock();
        // Full: wait for oah to catch up (the program blocks on its writes meanwhile).
        while st.procs.get(&id).is_some_and(|p| p.streams[s].data.len() >= BUFFER_CAP && p.kill_at.is_none()) {
            st = sh.cv.wait(st).unwrap_or_else(|e| e.into_inner());
        }
        if let Some(p) = st.procs.get_mut(&id) {
            if n == 0 {
                p.streams[s].eof = true;
            } else if p.kill_at.is_none() {
                p.streams[s].data.extend_from_slice(&buf[..n]);
            }
        }
        sh.cv.notify_all();
        if n == 0 {
            return;
        }
    }
}

fn reap(sh: &Shared, id: u64, mut child: Child) {
    let status = child.wait();
    let (code, signal) = match &status {
        Ok(s) => (s.code(), s.signal()),
        Err(_) => (None, None),
    };
    let mut st = sh.lock();
    // Its last output may still be in the pipes.
    let until = Instant::now() + DRAIN_GRACE;
    while st.procs.get(&id).is_some_and(|p| !(p.streams[0].eof && p.streams[1].eof)) {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        st = sh.cv.wait_timeout(st, left).unwrap_or_else(|e| e.into_inner()).0;
    }
    if let Some(p) = st.procs.get_mut(&id) {
        p.exit = Some((code, signal));
        p.stdin = None;
    }
    sh.cv.notify_all();
}

/// Sends a connection its replies and its programs' output, as they come.
fn writer(sh: &Shared, cid: u64, mut w: UnixStream) {
    loop {
        let batch = {
            let mut st = sh.lock();
            loop {
                if !st.conns.contains_key(&cid) {
                    return;
                }
                let b = gather(&mut st, cid);
                if !b.is_empty() {
                    break b;
                }
                st = sh.cv.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };
        if w.write_all(&batch).is_err() {
            let _ = w.shutdown(std::net::Shutdown::Both);
            return;
        }
    }
}

fn gather(st: &mut State, cid: u64) -> Vec<u8> {
    let State { conns, procs, .. } = st;
    let mut out = conns.get_mut(&cid).map(|c| std::mem::take(&mut c.queue)).unwrap_or_default();
    for (&id, p) in procs.iter_mut().filter(|(_, p)| p.conn == Some(cid)) {
        let mine = out.len() + BATCH_EACH;
        for (i, b) in p.streams.iter_mut().enumerate() {
            while b.sent < b.end() && out.len() < BATCH && out.len() < mine {
                let from = (b.sent - b.base) as usize;
                let to = (from + CHUNK).min(b.data.len());
                out.extend(Msg::Out { id, stream: i as u8 + 1, off: b.sent, data: b.data[from..to].to_vec() }.encode());
                b.sent += (to - from) as u64;
            }
            if b.eof && !b.eof_sent && b.sent == b.end() {
                out.extend(Msg::Eof { id, stream: i as u8 + 1, off: b.sent }.encode());
                b.eof_sent = true;
            }
        }
        if let Some((code, signal)) = p.exit {
            if !p.exit_sent && p.streams.iter().all(|b| b.sent == b.end()) {
                out.extend(Msg::Exited { id, code, signal }.encode());
                p.exit_sent = true;
            }
        }
    }
    out
}

/// Ends forgotten programs and ones left too long without a connection; exits when idle.
fn reaper(sh: &Shared) {
    let mut busy = Instant::now();
    loop {
        thread::sleep(Duration::from_secs(1));
        let mut st = sh.lock();
        let now = Instant::now();
        let done: Vec<u64> = st
            .procs
            .iter()
            .filter(|(_, p)| match p.kill_at {
                Some(at) => now >= at,
                None => {
                    let limit = if p.exit.is_some() { EXITED_LIMIT } else { DETACHED_LIMIT };
                    p.conn.is_none() && now.duration_since(p.detached_at) > limit
                }
            })
            .map(|(id, _)| *id)
            .collect();
        for id in done {
            if let Some(p) = st.procs.remove(&id) {
                kill_group(p.pgid);
            }
        }
        if !st.procs.is_empty() || !st.conns.is_empty() {
            busy = now;
        } else if now.duration_since(busy) > IDLE_EXIT {
            let _ = std::fs::remove_file(&sh.paths.sock);
            log("idle; exiting");
            std::process::exit(0);
        }
        sh.cv.notify_all();
    }
}

/// SIGKILL to a group that still has members (a group with none may have had its id reused).
fn kill_group(pgid: i32) {
    if pgid <= 1 {
        return;
    }
    // SAFETY: plain signals to a process group this daemon created.
    unsafe {
        if libc::kill(-pgid, 0) == 0 {
            libc::kill(-pgid, libc::SIGKILL);
        }
    }
}

fn nonce() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    t ^ ((std::process::id() as u64) << 32)
}

fn log(line: &str) {
    eprintln!("oah-host: {line}");
}
