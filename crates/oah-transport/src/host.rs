//! Programs on an SSH host through `oah-host` (see that crate): one `ssh` connection per host,
//! kept open and shared by every agent and command there, that reconnects by itself. Programs
//! keep running on the host while it's down, and nothing they read or print is lost or doubled.
//!
//! The first time a host is used in a run of oah, a short `ssh` asks what the host is and which
//! helpers it has; when its build isn't there, it's copied over that same connection (no
//! download on the host). A host oah has no helper for (not Linux on x86-64 or ARM64), a build
//! without the helpers, or a host where it won't run (a `noexec` home) is used the old way: one
//! `ssh` per program ([`crate::command`]).

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use oah_core::settings::Location;
use oah_core::ssh::sh_quote;
use oah_host::proto::{self, Msg, CHUNK, MAX_SCRIPT, PROTO};

use crate::Spec;

/// How long a dropped connection is retried for; the host ends what's left after this too.
const RETRY_LIMIT: Duration = oah_host::DETACHED_LIMIT;
/// How long a new program waits for a connection (or for the host to start it).
const SPAWN_WAIT: Duration = Duration::from_secs(30);
/// A connection that hasn't answered a ping for this long is dropped and made again.
const PONG_LIMIT: Duration = Duration::from_secs(35);
const PING_EVERY: Duration = Duration::from_secs(10);
/// Stdin written but not yet acknowledged by the host, per program; a write past it waits.
const STDIN_CAP: usize = 16 << 20;
/// Output read is acknowledged in steps this big (and whenever all of it has been read), so the
/// host holds what hasn't been, and a program nobody reads waits as it would on a pipe.
const ACK_STEP: u64 = 256 << 10;
/// A host that couldn't use the helper is tried again after this.
const DIRECT_FOR: Duration = Duration::from_secs(10 * 60);
/// A host that didn't answer is tried again after this.
const UNREACHABLE_FOR: Duration = Duration::from_secs(15);

// ---- Which binaries this build has -------------------------------------------------------------

static BIN_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Where this build's helpers are (`oah-host-x86_64-linux`, `oah-host-aarch64-linux`): the app's
/// resource folder. `$OAH_HOST_DIR` wins when set.
pub fn set_binary_dir(dir: PathBuf) {
    *BIN_DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir);
}

fn binary_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("OAH_HOST_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    BIN_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// One helper build: its contents and the tag they're named by on hosts.
struct Bin {
    tag: String,
    bytes: Vec<u8>,
}

fn binary(platform: &str) -> Option<Arc<Bin>> {
    static CACHE: Mutex<Option<HashMap<String, Arc<Bin>>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(b) = cache.as_ref().and_then(|c| c.get(platform)) {
        return Some(b.clone());
    }
    let bytes = std::fs::read(binary_dir()?.join(format!("oah-host-{platform}"))).ok().filter(|b| !b.is_empty())?;
    let bin = Arc::new(Bin { tag: oah_host::tag_of(&bytes), bytes });
    cache.get_or_insert_with(HashMap::new).insert(platform.to_string(), bin.clone());
    Some(bin)
}

/// The helper's platform for `uname -sm`.
fn platform(uname: &str) -> Option<&'static str> {
    let mut w = uname.split_whitespace();
    let (os, arch) = (w.next()?, w.next()?);
    match (os, arch) {
        ("Linux", "x86_64" | "amd64") => Some("x86_64-linux"),
        ("Linux", "aarch64" | "arm64") => Some("aarch64-linux"),
        _ => None,
    }
}

// ---- Which hosts use it ----------------------------------------------------------------------

enum Route {
    Helper(Arc<Session>),
    /// One `ssh` per program, until then.
    Direct(Instant),
}

type Slot = Arc<Mutex<Option<Route>>>;

fn hosts() -> MutexGuard<'static, HashMap<String, Slot>> {
    static HOSTS: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    HOSTS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// The host's helper connection, made (and the helper put there) on first use; `None` means
/// "use plain ssh", for now or for this host.
pub(crate) fn session_for(host: &str) -> Option<Arc<Session>> {
    binary_dir()?;
    let slot = hosts().entry(host.to_string()).or_default().clone();
    // One setup per host at a time; the others wait for its answer.
    let mut route = slot.lock().unwrap_or_else(|e| e.into_inner());
    match &*route {
        Some(Route::Helper(s)) if s.usable() => return Some(s.clone()),
        Some(Route::Direct(until)) if Instant::now() < *until => return None,
        _ => {}
    }
    match setup(host) {
        Ok(s) => {
            *route = Some(Route::Helper(s.clone()));
            Some(s)
        }
        Err(Setup::Unreachable(e)) => {
            log(host, &format!("not reachable: {e}"));
            *route = Some(Route::Direct(Instant::now() + UNREACHABLE_FOR));
            None
        }
        Err(Setup::Unsupported(e)) => {
            log(host, &format!("using plain ssh: {e}"));
            *route = Some(Route::Direct(Instant::now() + DIRECT_FOR));
            None
        }
    }
}

/// Ends every helper connection: what's still running is told to stop (stdin closed, then its
/// group killed), as when oah quits.
pub fn shutdown() {
    let slots: Vec<Slot> = hosts().values().cloned().collect();
    let sessions: Vec<Arc<Session>> = slots
        .iter()
        // A host still being set up has nothing running yet; quitting doesn't wait for it.
        .filter_map(|s| match s.try_lock().as_deref() {
            Ok(Some(Route::Helper(s))) => Some(s.clone()),
            _ => None,
        })
        .collect();
    for s in &sessions {
        s.close();
    }
}

enum Setup {
    Unreachable(String),
    Unsupported(String),
}

/// What the host is and which helpers it has.
const PROBE: &str = r#"
printf 'oah-uname\t%s\n' "$(uname -sm 2>/dev/null)"
for f in "$HOME"/.oah/bin/oah-host-*; do [ -x "$f" ] && printf 'oah-bin\t%s\n' "${f##*/}"; done
exit 0
"#;

/// The helper on stdin, saved as `~/.oah/bin/oah-host-$1` and tried; old builds go after two weeks.
const INSTALL: &str = r#"
umask 077
d="$HOME/.oah/bin"
mkdir -p "$d" || exit 1
t="$d/.oah-host-$$"
if ! { cat > "$t" && chmod 700 "$t" && mv -f "$t" "$d/oah-host-$1"; }; then rm -f "$t"; exit 1; fi
if ! "$d/oah-host-$1" --version >/dev/null 2>&1; then
  rm -f "$d/oah-host-$1"
  echo "oah-host won't run in $d (is the home folder mounted noexec?)" >&2
  exit 3
fi
find "$d" -name 'oah-host-*' ! -name "oah-host-$1" -mtime +14 -exec rm -f {} + 2>/dev/null
exit 0
"#;

fn setup(host: &str) -> Result<Arc<Session>, Setup> {
    let probe = sh(host, PROBE, &[], None, Duration::from_secs(25)).map_err(Setup::Unreachable)?;
    if probe.code == Some(255) {
        return Err(Setup::Unreachable(last_line(&probe.stderr)));
    }
    if !probe.ok() {
        return Err(Setup::Unsupported(format!("the host check failed: {}", last_line(&probe.stderr))));
    }
    let field =
        |key: &str| probe.stdout.lines().filter_map(|l| l.strip_prefix(key)?.strip_prefix('\t')).map(str::to_string).collect::<Vec<_>>();
    let uname = field("oah-uname").pop().unwrap_or_default();
    let plat = platform(&uname).ok_or_else(|| Setup::Unsupported(format!("no helper for {}", uname.trim())))?;
    let bin = binary(plat).ok_or_else(|| Setup::Unsupported(format!("this build has no helper for {plat}")))?;
    if !field("oah-bin").iter().any(|b| *b == format!("oah-host-{}", bin.tag)) {
        install(host, &bin).map_err(Setup::Unsupported)?;
    }
    Session::open(host, bin).map_err(Setup::Unsupported)
}

fn install(host: &str, bin: &Bin) -> Result<(), String> {
    let out = sh(host, INSTALL, &[&bin.tag], Some(&bin.bytes), Duration::from_secs(180))?;
    if out.ok() {
        Ok(())
    } else {
        Err(format!("couldn't put the helper on the host: {}", last_line(&out.stderr)))
    }
}

/// A POSIX `sh` script on the host through plain ssh, with `args` as `$1`…
fn sh(host: &str, script: &str, args: &[&str], input: Option<&[u8]>, limit: Duration) -> Result<crate::Output, String> {
    let loc = Location::Ssh { host: host.to_string() };
    let mut a = vec!["-c".to_string(), script.to_string(), "sh".to_string()];
    a.extend(args.iter().map(|s| s.to_string()));
    crate::run_direct(&Spec { location: &loc, cwd: "~", program: "sh", args: a, env: &[] }, input, limit)
}

fn last_line(s: &str) -> String {
    s.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()
}

fn log(host: &str, msg: &str) {
    eprintln!("oah-host [{host}]: {msg}");
}

// ---- Who we are, to the host ------------------------------------------------------------------

/// This oah install (kept in the app folder) and this run of it.
fn ids() -> (u64, u64) {
    static IDS: OnceLock<(u64, u64)> = OnceLock::new();
    *IDS.get_or_init(|| {
        let file = oah_core::paths::app_dir().join("host-client-id");
        let client =
            std::fs::read_to_string(&file).ok().and_then(|s| u64::from_str_radix(s.trim(), 16).ok()).filter(|c| *c != 0).unwrap_or_else(
                || {
                    let c = random();
                    let _ = std::fs::create_dir_all(oah_core::paths::app_dir());
                    let _ = std::fs::write(&file, format!("{c:016x}\n"));
                    c
                },
            );
        (client, random())
    })
}

fn random() -> u64 {
    uuid::Uuid::new_v4().as_u64_pair().0 | 1
}

// ---- A connection ----------------------------------------------------------------------------

pub(crate) struct Session {
    host: String,
    bin: Arc<Bin>,
    st: Mutex<St>,
    cv: Condvar,
}

#[derive(Default)]
struct St {
    link: Option<Link>,
    /// Counts connections, so a dead one's threads know they're stale.
    gen: u64,
    reconnecting: bool,
    /// Spawns waiting for a connection.
    waiting: usize,
    /// Gave up reconnecting, or oah is quitting.
    closed: bool,
    procs: HashMap<u64, Remote>,
    /// Programs let go of whose `Forget` the host hasn't confirmed: said again on every
    /// connection until it does.
    forgets: HashSet<u64>,
    /// The newest connection whose reader has ended (it's dead even if not yet installed).
    ended: u64,
    last_pong: Option<Instant>,
}

struct Link {
    tx: Sender<Vec<u8>>,
    ssh: Arc<Mutex<Child>>,
}

impl St {
    fn send(&self, m: &Msg) {
        if let Some(l) = &self.link {
            let _ = l.tx.send(m.encode());
        }
    }

    /// Lets the host end `id`, now and on every connection until it says it heard.
    fn forget(&mut self, id: u64) {
        self.forgets.insert(id);
        self.send(&Msg::Forget { id });
    }

    /// Whether a dropped connection should be made again.
    fn needed(&self) -> bool {
        !self.closed && (self.waiting > 0 || !self.forgets.is_empty() || self.procs.values().any(|p| !p.finished()))
    }

    /// Acknowledges what's been read of `id`'s output, once there's enough of it (or all).
    fn ack_read(&mut self, id: u64) {
        let Some(p) = self.procs.get_mut(&id) else { return };
        let read = [p.out[0].read(), p.out[1].read()];
        let due = p.out.iter().zip(read).any(|(pipe, r)| r > pipe.acked && (r - pipe.acked >= ACK_STEP || pipe.buf.is_empty()));
        if due {
            p.out[0].acked = read[0];
            p.out[1].acked = read[1];
            self.send(&Msg::Ack { id, out: read[0], err: read[1] });
        }
    }
}

/// A program on the host, as oah sees it.
struct Remote {
    script: String,
    spawned: bool,
    failed: Option<String>,
    /// Stdin not yet acknowledged, from byte `stdin_base`.
    stdin: VecDeque<u8>,
    stdin_base: u64,
    stdin_total: u64,
    stdin_closed: bool,
    out: [Pipe; 2],
    exit: Option<Exit>,
    /// The host doesn't have it any more.
    lost: bool,
    kill: bool,
}

#[derive(Default)]
struct Pipe {
    buf: VecDeque<u8>,
    /// Bytes received so far (the offset of the next one).
    recv: u64,
    eof: bool,
    /// Acknowledged to the host up to here.
    acked: u64,
    /// Nobody will read it (its reader was dropped): taken and acknowledged as it comes.
    discard: bool,
}

impl Pipe {
    /// What's been read (or discarded) so far.
    fn read(&self) -> u64 {
        self.recv - self.buf.len() as u64
    }
}

/// How a program on the host ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

impl Remote {
    fn finished(&self) -> bool {
        self.failed.is_some() || self.lost || (self.exit.is_some() && self.out.iter().all(|p| p.eof))
    }
}

impl Session {
    fn open(host: &str, bin: Arc<Bin>) -> Result<Arc<Session>, String> {
        let s = Arc::new(Session { host: host.to_string(), bin, st: Mutex::new(St::default()), cv: Condvar::new() });
        s.connect()?;
        Ok(s)
    }

    fn lock(&self) -> MutexGuard<'_, St> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn usable(&self) -> bool {
        !self.lock().closed
    }

    /// Makes the connection; on success everything in flight is picked up where it was.
    fn connect(self: &Arc<Self>) -> Result<(), String> {
        let tag = &self.bin.tag;
        let line = format!("sh -c {}", sh_quote(&format!("exec \"$HOME/.oah/bin/oah-host-{tag}\" proxy {tag}")));
        let mut cmd = Command::new("ssh");
        cmd.args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=2"])
            .args(["--", &self.host, &line])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::no_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| format!("couldn't run ssh: {e}"))?;
        let (Some(mut stdin), Some(mut stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
            let _ = child.kill();
            return Err("ssh has no stdio".into());
        };
        let ssh = Arc::new(Mutex::new(child));
        let gen = {
            let mut st = self.lock();
            st.gen += 1;
            st.gen
        };

        let err_tail = Arc::new(Mutex::new(String::new()));
        {
            let tail = err_tail.clone();
            thread::spawn(move || {
                let mut s = String::new();
                for line in io::BufRead::lines(io::BufReader::new(stderr)).map_while(Result::ok) {
                    s.push_str(&line);
                    s.push('\n');
                    if s.len() > 4096 {
                        s.drain(..s.len() - 4096);
                    }
                    *tail.lock().unwrap_or_else(|e| e.into_inner()) = s.clone();
                }
            });
        }
        let (tx, rx) = channel::<Vec<u8>>();
        thread::spawn(move || {
            for frame in rx {
                if stdin.write_all(&frame).and_then(|_| stdin.flush()).is_err() {
                    return;
                }
            }
            // Every sender gone (oah is closing the connection): ssh sees stdin end.
        });
        let (client, instance) = ids();
        let _ = tx.send(Msg::Hello { proto: PROTO, client, instance }.encode());

        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        {
            let me = self.clone();
            let ssh = ssh.clone();
            thread::spawn(move || me.read_loop(gen, &mut stdout, ready_tx, &ssh));
        }
        let ready = ready_rx.recv_timeout(SPAWN_WAIT).unwrap_or_else(|_| Err("no answer from the helper".into()));
        if let Err(e) = ready {
            let _ = ssh.lock().unwrap_or_else(|e| e.into_inner()).kill();
            thread::sleep(Duration::from_millis(100));
            let tail = last_line(&err_tail.lock().unwrap_or_else(|e| e.into_inner()));
            return Err(if tail.is_empty() { e } else { tail });
        }

        let mut st = self.lock();
        if st.gen != gen || st.closed || st.ended >= gen {
            drop(st);
            let _ = ssh.lock().unwrap_or_else(|e| e.into_inner()).kill();
            return Err("the connection dropped as it was made".into());
        }
        st.link = Some(Link { tx, ssh: ssh.clone() });
        st.last_pong = Some(Instant::now());
        // Pick up where everything was.
        let mut resend = Vec::new();
        for (&id, p) in &st.procs {
            if p.failed.is_some() || p.lost {
                continue;
            }
            if !p.spawned {
                // It may have started (and printed) with its `Spawned` lost; the host then
                // sends all of it again.
                resend.push(Msg::Spawn { id, script: p.script.clone() });
            }
            resend.push(Msg::Attach { id, out: p.out[0].recv, err: p.out[1].recv });
            let data: Vec<u8> = p.stdin.iter().copied().collect();
            for (i, chunk) in data.chunks(CHUNK).enumerate() {
                resend.push(Msg::Stdin { id, off: p.stdin_base + (i * CHUNK) as u64, data: chunk.to_vec() });
            }
            if p.stdin_closed {
                resend.push(Msg::CloseStdin { id, off: p.stdin_total });
            }
            if p.kill && p.exit.is_none() {
                resend.push(Msg::Kill { id });
            }
        }
        resend.extend(st.forgets.iter().map(|&id| Msg::Forget { id }));
        for m in &resend {
            st.send(m);
        }
        drop(st);
        self.cv.notify_all();
        {
            let me = self.clone();
            thread::spawn(move || me.ping_loop(gen, ssh));
        }
        Ok(())
    }

    fn read_loop(self: Arc<Self>, gen: u64, r: &mut impl Read, ready: Sender<Result<(), String>>, ssh: &Arc<Mutex<Child>>) {
        let mut r = io::BufReader::with_capacity(128 * 1024, r);
        let greeted = match proto::skip_to_magic(&mut r, 256 * 1024) {
            Ok(_) => match proto::read(&mut r) {
                Ok(Some(Msg::HelloOk { proto: PROTO, .. })) => Ok(()),
                Ok(Some(Msg::HelloOk { proto, .. })) => Err(format!("the helper speaks version {proto}, not {PROTO}")),
                Ok(_) => Err("the helper didn't say hello".into()),
                Err(e) => Err(e.to_string()),
            },
            Err(e) => Err(format!("the helper didn't start: {}", last_line(&e.to_string()))),
        };
        let ok = greeted.is_ok();
        let _ = ready.send(greeted);
        if ok {
            while let Ok(Some(m)) = proto::read(&mut r) {
                let mut st = self.lock();
                if st.gen != gen {
                    break;
                }
                // Anything from the host shows the connection works (a pong can wait behind a
                // lot of output on a slow link).
                st.last_pong = Some(Instant::now());
                apply(&mut st, m);
                drop(st);
                self.cv.notify_all();
            }
        }
        let _ = ssh.lock().unwrap_or_else(|e| e.into_inner()).kill();
        let _ = ssh.lock().unwrap_or_else(|e| e.into_inner()).wait();
        let mut st = self.lock();
        st.ended = st.ended.max(gen);
        if st.gen == gen && st.link.is_some() {
            st.link = None;
            log(&self.host, "connection dropped");
        }
        drop(st);
        self.cv.notify_all();
        if ok {
            self.reconnect();
        }
    }

    fn ping_loop(self: Arc<Self>, gen: u64, ssh: Arc<Mutex<Child>>) {
        let mut n = 0;
        loop {
            thread::sleep(PING_EVERY);
            let st = self.lock();
            if st.gen != gen || st.link.is_none() {
                return;
            }
            // Asleep, or a network that went away without a word: start over.
            if st.last_pong.is_some_and(|t| t.elapsed() > PONG_LIMIT) {
                drop(st);
                log(&self.host, "no answer; reconnecting");
                let _ = ssh.lock().unwrap_or_else(|e| e.into_inner()).kill();
                return;
            }
            n += 1;
            st.send(&Msg::Ping { n });
        }
    }

    /// Makes the connection again, in the background, while something needs it.
    fn reconnect(self: &Arc<Self>) {
        {
            let mut st = self.lock();
            if st.reconnecting || st.link.is_some() || !st.needed() {
                return;
            }
            st.reconnecting = true;
        }
        let me = self.clone();
        thread::spawn(move || {
            let started = Instant::now();
            let mut delay = Duration::from_millis(500);
            let mut reinstalled = false;
            loop {
                match me.connect() {
                    Ok(()) => break,
                    Err(e) => {
                        log(&me.host, &format!("reconnecting: {e}"));
                        // The helper went missing from the host (a cleaned home folder).
                        if !reinstalled && (e.contains("not found") || e.contains("No such file")) {
                            reinstalled = true;
                            let _ = install(&me.host, &me.bin);
                            continue;
                        }
                    }
                }
                let mut st = me.lock();
                if !st.needed() {
                    st.reconnecting = false;
                    return;
                }
                if started.elapsed() > RETRY_LIMIT {
                    st.closed = true;
                    st.reconnecting = false;
                    for p in st.procs.values_mut() {
                        p.lost = true;
                    }
                    drop(st);
                    me.cv.notify_all();
                    return;
                }
                drop(st);
                thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_secs(15));
            }
            let mut st = me.lock();
            st.reconnecting = false;
            // It dropped again before this thread was done.
            let again = st.link.is_none() && st.needed();
            drop(st);
            me.cv.notify_all();
            if again {
                me.reconnect();
            }
        });
    }

    /// Starts `sh -lc <script>` on the host.
    pub(crate) fn spawn(self: &Arc<Self>, script: String) -> Result<HostProc, String> {
        if script.len() > MAX_SCRIPT {
            return Err(format!("the command is too long to start on a host ({} KB)", script.len() / 1024));
        }
        let id = random();
        let mut st = self.lock();
        if st.closed {
            return Err(format!("the connection to {} is closed", self.host));
        }
        st.procs.insert(
            id,
            Remote {
                script: script.clone(),
                spawned: false,
                failed: None,
                stdin: VecDeque::new(),
                stdin_base: 0,
                stdin_total: 0,
                stdin_closed: false,
                out: Default::default(),
                exit: None,
                lost: false,
                kill: false,
            },
        );
        let waited = st.link.is_none();
        if waited {
            st.waiting += 1;
            drop(st);
            self.reconnect();
            st = self.lock();
        } else {
            st.send(&Msg::Spawn { id, script });
        }
        let until = Instant::now() + SPAWN_WAIT;
        let result = loop {
            let p = &st.procs[&id];
            if p.spawned {
                break Ok(());
            }
            if let Some(e) = &p.failed {
                break Err(e.clone());
            }
            if p.lost || st.closed {
                break Err(format!("lost the connection to {}", self.host));
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break Err(format!("couldn't reach {}", self.host));
            }
            st = self.cv.wait_timeout(st, left).unwrap_or_else(|e| e.into_inner()).0;
        };
        if waited {
            st.waiting -= 1;
        }
        match result {
            Ok(()) => Ok(HostProc { sess: self.clone(), id, killed: AtomicBool::new(false) }),
            Err(e) => {
                st.procs.remove(&id);
                // In case the host started it after all.
                st.forget(id);
                Err(e)
            }
        }
    }

    /// Lets go of every program and the connection, waiting a moment for the host to hear it.
    fn close(&self) {
        let mut st = self.lock();
        st.closed = true;
        let ids: Vec<u64> = st.procs.keys().chain(st.forgets.iter()).copied().collect();
        for id in ids {
            st.send(&Msg::Forget { id });
        }
        let link = st.link.take();
        drop(st);
        self.cv.notify_all();
        if let Some(Link { tx, ssh }) = link {
            // The writer sends what's queued, then closes ssh's stdin.
            drop(tx);
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                if matches!(ssh.lock().unwrap_or_else(|e| e.into_inner()).try_wait(), Ok(Some(_))) {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            let _ = ssh.lock().unwrap_or_else(|e| e.into_inner()).kill();
        }
    }
}

/// A message from the host, applied to what we know.
fn apply(st: &mut St, m: Msg) {
    match m {
        Msg::Spawned { id } => {
            if let Some(p) = st.procs.get_mut(&id) {
                p.spawned = true;
            }
        }
        Msg::SpawnFailed { id, msg } => {
            if let Some(p) = st.procs.get_mut(&id) {
                p.failed = Some(msg);
            }
        }
        Msg::Out { id, stream, off, data } => {
            let Some(p) = st.procs.get_mut(&id) else { return };
            let Some(pipe) = p.out.get_mut(stream as usize - 1) else { return };
            let end = off + data.len() as u64;
            if off <= pipe.recv && end > pipe.recv {
                if !pipe.discard {
                    pipe.buf.extend(&data[(pipe.recv - off) as usize..]);
                }
                pipe.recv = end;
            }
            // Acknowledged as it's read (see HostOut), or now when nobody will read it.
            if pipe.discard {
                st.ack_read(id);
            }
        }
        Msg::Eof { id, stream, off } => {
            if let Some(pipe) = st.procs.get_mut(&id).and_then(|p| p.out.get_mut(stream as usize - 1)) {
                if off == pipe.recv {
                    pipe.eof = true;
                }
            }
        }
        Msg::StdinAck { id, off } => {
            if let Some(p) = st.procs.get_mut(&id) {
                let n = off.saturating_sub(p.stdin_base).min(p.stdin.len() as u64);
                p.stdin.drain(..n as usize);
                p.stdin_base += n;
            }
        }
        Msg::Exited { id, code, signal } => {
            if let Some(p) = st.procs.get_mut(&id) {
                p.exit = Some(Exit { code, signal });
            }
        }
        Msg::Gone { id } => {
            st.forgets.remove(&id);
            if let Some(p) = st.procs.get_mut(&id) {
                p.lost = true;
            }
        }
        Msg::Pong { .. } => st.last_pong = Some(Instant::now()),
        _ => {}
    }
}

// ---- A program -------------------------------------------------------------------------------

/// A program on the host. Dropping it lets the host end it (stdin closed, then its group killed).
pub struct HostProc {
    sess: Arc<Session>,
    id: u64,
    killed: AtomicBool,
}

impl HostProc {
    pub fn stdin(&self) -> HostStdin {
        HostStdin { sess: self.sess.clone(), id: self.id }
    }

    pub fn stdout(&self) -> HostOut {
        HostOut { sess: self.sess.clone(), id: self.id, stream: 0 }
    }

    pub fn stderr(&self) -> HostOut {
        HostOut { sess: self.sess.clone(), id: self.id, stream: 1 }
    }

    pub fn try_wait(&self) -> Option<Exit> {
        let st = self.sess.lock();
        status(&st, self.id)
    }

    /// Waits for it to exit. Once killed, only briefly: the kill may wait for a connection, and
    /// a killed program counts as ended (as a local one would be).
    pub fn wait(&self) -> Exit {
        let limit = self.killed.load(Ordering::Relaxed).then_some(Duration::from_secs(3));
        self.wait_for(limit).unwrap_or(Exit { code: None, signal: None })
    }

    fn wait_for(&self, limit: Option<Duration>) -> Option<Exit> {
        let until = limit.map(|l| Instant::now() + l);
        let mut st = self.sess.lock();
        loop {
            if let Some(e) = status(&st, self.id) {
                return Some(e);
            }
            match until {
                None => st = self.sess.cv.wait(st).unwrap_or_else(|e| e.into_inner()),
                Some(u) => {
                    let left = u.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return None;
                    }
                    st = self.sess.cv.wait_timeout(st, left).unwrap_or_else(|e| e.into_inner()).0;
                }
            }
        }
    }

    /// SIGKILL to its whole process group on the host.
    pub fn kill(&self) {
        self.killed.store(true, Ordering::Relaxed);
        let mut st = self.sess.lock();
        let Some(p) = st.procs.get_mut(&self.id) else { return };
        p.kill = true;
        st.send(&Msg::Kill { id: self.id });
    }

    /// Gives it `grace` to exit (its stdin should be closed by now), then ends it and what it
    /// started.
    pub fn stop(&self, grace: Duration) {
        if self.sess.lock().link.is_none() {
            // Nothing can be heard now: the kill goes when the connection is back (and the host
            // ends it anyway once forgotten).
            self.kill();
            return;
        }
        if self.wait_for(Some(grace)).is_none() {
            self.kill();
            let _ = self.wait_for(Some(Duration::from_secs(2)));
        } else {
            // Exited: what it left running in its group goes too.
            self.kill();
        }
    }
}

/// Exited, or gone (an unknown status).
fn status(st: &St, id: u64) -> Option<Exit> {
    match st.procs.get(&id) {
        None => Some(Exit { code: None, signal: None }),
        Some(p) if p.lost || p.failed.is_some() || st.closed => Some(p.exit.unwrap_or(Exit { code: None, signal: None })),
        Some(p) => p.exit,
    }
}

impl Drop for HostProc {
    fn drop(&mut self) {
        let mut st = self.sess.lock();
        if st.procs.remove(&self.id).is_some() {
            st.forget(self.id);
        }
        let offline = st.link.is_none();
        drop(st);
        self.sess.cv.notify_all();
        if offline {
            self.sess.reconnect();
        }
    }
}

/// A host program's stdin. Dropping it closes it, once everything written has arrived.
pub struct HostStdin {
    sess: Arc<Session>,
    id: u64,
}

impl Write for HostStdin {
    /// Takes up to one chunk, waiting while the host is behind (as a full pipe would).
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        let chunk = &data[..data.len().min(CHUNK)];
        let mut st = self.sess.lock();
        loop {
            let closed = st.closed;
            let Some(p) = st.procs.get_mut(&self.id) else { return Err(io::ErrorKind::BrokenPipe.into()) };
            if closed || p.stdin_closed || p.exit.is_some() || p.lost {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            if p.stdin.is_empty() || p.stdin.len() + chunk.len() <= STDIN_CAP {
                let m = Msg::Stdin { id: self.id, off: p.stdin_total, data: chunk.to_vec() };
                p.stdin.extend(chunk);
                p.stdin_total += chunk.len() as u64;
                st.send(&m);
                return Ok(chunk.len());
            }
            st = self.sess.cv.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for HostStdin {
    fn drop(&mut self) {
        let mut st = self.sess.lock();
        let Some(p) = st.procs.get_mut(&self.id) else { return };
        if !p.stdin_closed {
            p.stdin_closed = true;
            let off = p.stdin_total;
            st.send(&Msg::CloseStdin { id: self.id, off });
        }
    }
}

/// A host program's stdout or stderr.
pub struct HostOut {
    sess: Arc<Session>,
    id: u64,
    stream: usize,
}

impl Read for HostOut {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut st = self.sess.lock();
        loop {
            let closed = st.closed;
            let Some(p) = st.procs.get_mut(&self.id) else { return Ok(0) };
            let lost = p.lost || p.failed.is_some();
            let pipe = &mut p.out[self.stream];
            if !pipe.buf.is_empty() {
                let n = buf.len().min(pipe.buf.len());
                for (d, s) in buf.iter_mut().zip(pipe.buf.drain(..n)) {
                    *d = s;
                }
                st.ack_read(self.id);
                return Ok(n);
            }
            if pipe.eof || lost || closed {
                return Ok(0);
            }
            st = self.sess.cv.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Drop for HostOut {
    fn drop(&mut self) {
        let mut st = self.sess.lock();
        let Some(p) = st.procs.get_mut(&self.id) else { return };
        let pipe = &mut p.out[self.stream];
        pipe.discard = true;
        pipe.buf.clear();
        st.ack_read(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platforms() {
        assert_eq!(platform("Linux x86_64"), Some("x86_64-linux"));
        assert_eq!(platform("Linux aarch64\n"), Some("aarch64-linux"));
        assert_eq!(platform("Darwin arm64"), None);
        assert_eq!(platform(""), None);
    }
}
