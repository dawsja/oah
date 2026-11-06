//! Safe Rust bindings over the OpenTUI native C ABI.
//!
//! OpenTUI's core (written in Zig) owns terminal capability detection, mode
//! setup/restoration, cell diffing, output encoding, Unicode width handling and
//! input parsing (legacy, Kitty keyboard, SGR mouse, bracketed paste). This crate
//! drives it in "immediate mode": the host draws each frame into a Context-owned
//! offscreen buffer with checked draw records, then copies it into the Session's
//! framebuffer and submits it. The host owns the TTY (raw mode) and the clock.
//!
//! All calls must happen on the thread that created the [`Terminal`].

pub mod sys;

use std::fmt;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sys::*;

// ---------------------------------------------------------------------------
// Errors

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error {
    pub status: i32,
    pub op: &'static str,
}

impl Error {
    fn name(&self) -> &'static str {
        match self.status {
            -1 => "INVALID_ARGUMENT",
            -2 => "UNSUPPORTED_VERSION",
            -3 => "OUT_OF_MEMORY",
            -4 => "WRONG_THREAD",
            -5 => "INTERNAL_ERROR",
            -6 => "CONTEXT_BUSY",
            -7 => "WRONG_CONTEXT",
            -8 => "WRONG_KIND",
            -9 => "STALE_HANDLE",
            -10 => "WRONG_SESSION",
            -11 => "OUTPUT_BACKPRESSURE",
            -12 => "SESSION_CLOSED",
            -13 => "OUTPUT_BUSY",
            -14 => "STALE_OUTPUT",
            -15 => "OUTPUT_FAILED",
            -16 => "OBJECT_LIMIT",
            -17 => "RENDERER_ATTACHED",
            -18 => "RENDERER_NOT_ATTACHED",
            -19 => "INVALID_PHASE",
            -24 => "UNSUPPORTED_RESOURCE",
            -27 => "FRAME_BUSY",
            _ => "UNKNOWN",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "opentui {} failed: {} ({})", self.op, self.name(), self.status)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn check(status: ot_status, op: &'static str) -> Result<()> {
    if status == OT_OK {
        Ok(())
    } else {
        Err(Error { status, op })
    }
}

macro_rules! versioned {
    ($t:ty) => {{
        let mut v: $t = Default::default();
        v.struct_size = std::mem::size_of::<$t>() as u32;
        v.abi_version = OT_CONTEXT_ABI_VERSION;
        v
    }};
}

// ---------------------------------------------------------------------------
// Colors and styles

/// An opaque or translucent RGBA color, 0..255 per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba(r, g, b, 255)
    }
    pub const fn hex(v: u32) -> Self {
        Rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)
    }
    pub const TRANSPARENT: Rgba = Rgba(0, 0, 0, 0);
    fn words(self) -> [u16; 4] {
        [self.0 as u16, self.1 as u16, self.2 as u16, self.3 as u16]
    }
}

pub mod attr {
    pub const NONE: u32 = 0;
    pub const BOLD: u32 = 1;
    pub const DIM: u32 = 2;
    pub const ITALIC: u32 = 4;
    pub const UNDERLINE: u32 = 8;
    pub const BLINK: u32 = 16;
    pub const INVERSE: u32 = 32;
    pub const HIDDEN: u32 = 64;
    pub const STRIKETHROUGH: u32 = 128;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderStyle {
    Single,
    Rounded,
    Double,
    Heavy,
}

impl BorderStyle {
    fn chars(self) -> [u32; 11] {
        let s: [char; 11] = match self {
            BorderStyle::Single => ['┌', '┐', '└', '┘', '─', '│', '┬', '┴', '├', '┤', '┼'],
            BorderStyle::Rounded => ['╭', '╮', '╰', '╯', '─', '│', '┬', '┴', '├', '┤', '┼'],
            BorderStyle::Double => ['╔', '╗', '╚', '╝', '═', '║', '╦', '╩', '╠', '╣', '╬'],
            BorderStyle::Heavy => ['┏', '┓', '┗', '┛', '━', '┃', '┳', '┻', '┣', '┫', '╋'],
        };
        s.map(|c| c as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left = 0,
    Center = 1,
    Right = 2,
}

// ---------------------------------------------------------------------------
// Input events

pub mod key {
    pub use crate::sys::{
        OT_INPUT_KEY_BACKSPACE as BACKSPACE, OT_INPUT_KEY_DELETE as DELETE, OT_INPUT_KEY_DOWN as DOWN,
        OT_INPUT_KEY_END as END, OT_INPUT_KEY_ENTER as ENTER, OT_INPUT_KEY_ESCAPE as ESCAPE,
        OT_INPUT_KEY_HOME as HOME, OT_INPUT_KEY_KP_ENTER as KP_ENTER, OT_INPUT_KEY_LEFT as LEFT,
        OT_INPUT_KEY_PAGE_DOWN as PAGE_DOWN, OT_INPUT_KEY_PAGE_UP as PAGE_UP, OT_INPUT_KEY_RIGHT as RIGHT,
        OT_INPUT_KEY_TAB as TAB, OT_INPUT_KEY_UP as UP,
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods(pub u8);

impl Mods {
    pub fn shift(self) -> bool {
        self.0 & OT_INPUT_MOD_SHIFT != 0
    }
    pub fn alt(self) -> bool {
        self.0 & OT_INPUT_MOD_ALT != 0
    }
    pub fn ctrl(self) -> bool {
        self.0 & OT_INPUT_MOD_CTRL != 0
    }
    pub fn sup(self) -> bool {
        self.0 & OT_INPUT_MOD_SUPER != 0
    }
    pub fn none(self) -> bool {
        self.0 & (OT_INPUT_MOD_SHIFT | OT_INPUT_MOD_ALT | OT_INPUT_MOD_CTRL | OT_INPUT_MOD_SUPER) == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    /// Unicode scalar or Kitty functional key code (see [`key`]).
    pub code: u32,
    pub mods: Mods,
    /// Text an editor would insert for this key.
    pub text: String,
    pub repeat: bool,
}

impl KeyEvent {
    /// True for `ctrl+<c>` in both legacy (control byte) and Kitty encodings.
    pub fn is_ctrl(&self, c: char) -> bool {
        let lower = c.to_ascii_lowercase() as u32;
        (self.mods.ctrl() && (self.code == lower || self.code == c.to_ascii_uppercase() as u32))
            || (self.code == (lower & 0x1f) && self.code != 0 && !matches!(self.code, 9 | 13 | 27))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Up,
    Move,
    Drag,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    /// 0 left, 1 middle, 2 right, 255 none.
    pub button: u32,
    pub x: u16,
    pub y: u16,
    pub mods: Mods,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Focus(bool),
    Resize(u16, u16),
    /// The [`Waker`] was triggered from another thread.
    Wake,
}

// ---------------------------------------------------------------------------
// TTY helpers (host-owned raw mode, size, wake pipe)

struct RawMode {
    fd: RawFd,
    original: libc::termios,
}

impl RawMode {
    fn enable(fd: RawFd) -> std::io::Result<Self> {
        unsafe {
            let mut original: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut original) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut raw = original;
            libc::cfmakeraw(&mut raw);
            // Keep output post-processing off (cfmakeraw) – OpenTUI emits explicit CR/LF.
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(fd, libc::TCSANOW, &raw) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(RawMode { fd, original })
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}

pub fn terminal_size() -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        for fd in [libc::STDOUT_FILENO, libc::STDIN_FILENO, libc::STDERR_FILENO] {
            if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
                return (ws.ws_col, ws.ws_row);
            }
        }
    }
    (80, 24)
}

/// Wakes a thread blocked in [`Terminal::poll`]. Cheap to clone and `Send`.
#[derive(Clone)]
pub struct Waker {
    fd: Arc<OwnedPipe>,
}

struct OwnedPipe {
    read: RawFd,
    write: RawFd,
}

impl Drop for OwnedPipe {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.read);
            libc::close(self.write);
        }
    }
}

impl Waker {
    pub fn wake(&self) {
        let b = [1u8];
        unsafe {
            libc::write(self.fd.write, b.as_ptr() as *const _, 1);
        }
    }
}

static RESIZED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigwinch(_: libc::c_int) {
    RESIZED.store(true, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Terminal

/// Options for [`Terminal::new`].
#[derive(Clone, Debug)]
pub struct TerminalOptions {
    pub alternate_screen: bool,
    pub mouse: bool,
    pub mouse_movement: bool,
    /// Kitty keyboard progressive-enhancement flags (0 disables).
    pub kitty_keyboard_flags: u32,
    pub title: Option<String>,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        TerminalOptions {
            alternate_screen: true,
            mouse: true,
            mouse_movement: false,
            kitty_keyboard_flags: 1,
            title: None,
        }
    }
}

const EVENT_CAP: usize = 64;
const PAYLOAD_CAP: usize = 16 * 1024;

/// A full-screen terminal driven by OpenTUI.
pub struct Terminal {
    ctx: *mut ot_context,
    session: ot_handle,
    buffer: ot_handle,
    width: u16,
    height: u16,
    epoch: Instant,
    raw: Option<RawMode>,
    pipe: Arc<OwnedPipe>,
    events: Vec<ot_input_event>,
    payload: Vec<u8>,
    paste: Option<Vec<u8>>,
    input_deadline: u64,
    pump_deadline: u64,
    expect_flags: u32,
    cursor: Option<(u16, u16)>,
    closed: bool,
    clips: u32,
}

impl Terminal {
    pub fn new(opts: TerminalOptions) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let (width, height) = terminal_size();
        let raw = RawMode::enable(libc::STDIN_FILENO)?;

        let mut fds = [0 as RawFd; 2];
        unsafe {
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            for fd in fds {
                let fl = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            libc::signal(libc::SIGWINCH, on_sigwinch as extern "C" fn(libc::c_int) as libc::sighandler_t);
        }
        let pipe = Arc::new(OwnedPipe { read: fds[0], write: fds[1] });

        let mut copts = versioned!(ot_context_options);
        copts.object_capacity = 4096;
        copts.render_cells_max = 4_000_000;
        let mut ctx: *mut ot_context = std::ptr::null_mut();
        check(unsafe { ot_context_create(&copts, &mut ctx) }, "context_create")?;

        let mut term = Terminal {
            ctx,
            session: ot_handle::default(),
            buffer: ot_handle::default(),
            width,
            height,
            epoch: Instant::now(),
            raw: Some(raw),
            pipe,
            events: vec![ot_input_event::default(); EVENT_CAP],
            payload: vec![0; PAYLOAD_CAP],
            paste: None,
            input_deadline: 0,
            pump_deadline: 0,
            expect_flags: 0,
            cursor: None,
            closed: false,
            clips: 0,
        };

        let mut sopts = versioned!(ot_session_options);
        sopts.chunk_size = 65_536;
        sopts.span_capacity = 128;
        sopts.max_bytes = 8_388_608;
        sopts.control_capacity = 4096;
        check(unsafe { ot_session_create(term.ctx, &sopts, &mut term.session) }, "session_create")?;

        let mut ropts = versioned!(ot_session_renderer_options);
        ropts.width = width as u32;
        ropts.height = height as u32;
        check(unsafe { ot_session_attach_renderer(term.ctx, &term.session, &ropts) }, "attach_renderer")?;

        let mut bopts = versioned!(ot_buffer_options);
        bopts.width = width as u32;
        bopts.height = height as u32;
        bopts.width_method = OT_WIDTH_METHOD_UNICODE;
        check(unsafe { ot_buffer_create(term.ctx, &bopts, &mut term.buffer) }, "buffer_create")?;

        let mut topts = versioned!(ot_session_terminal_options);
        topts.flags = OT_TERMINAL_CLEAR_ON_CLOSE | OT_TERMINAL_REPLY_SENTINEL;
        if opts.alternate_screen {
            topts.flags |= OT_TERMINAL_ALTERNATE_SCREEN;
        }
        if opts.mouse {
            topts.flags |= OT_TERMINAL_MOUSE;
        }
        if opts.mouse_movement {
            topts.flags |= OT_TERMINAL_MOUSE_MOVEMENT;
        }
        if std::env::var_os("TMUX").is_some() || std::env::var_os("STY").is_some() || std::env::var_os("ZELLIJ").is_some() {
            topts.flags |= OT_TERMINAL_HOST_MULTIPLEXER;
        }
        topts.kitty_keyboard_flags = opts.kitty_keyboard_flags;
        check(unsafe { ot_session_setup_terminal(term.ctx, &term.session, &topts) }, "setup_terminal")?;
        term.set_expect(OT_INPUT_EXPECT_SENTINEL | OT_INPUT_EXPECT_REPLIES);

        // Drive setup to completion: emit the mode/query packet and wait for ACTIVE.
        let start = Instant::now();
        loop {
            term.flush()?;
            if term.terminal_phase() == OT_TERMINAL_ACTIVE {
                break;
            }
            if start.elapsed() > Duration::from_secs(3) {
                break;
            }
            // Read replies that may already be arriving so they are not typed later.
            let _ = term.read_input(Duration::from_millis(5));
        }
        if let Some(title) = opts.title {
            term.set_title(&title);
        }
        Ok(term)
    }

    pub fn width(&self) -> u16 {
        self.width
    }
    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn waker(&self) -> Waker {
        Waker { fd: self.pipe.clone() }
    }

    fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64 + 1
    }

    fn terminal_phase(&self) -> u32 {
        let mut phase = 0;
        unsafe { ot_session_get_terminal_state(self.ctx, &self.session, &mut phase) };
        phase
    }

    fn set_expect(&mut self, flags: u32) {
        if flags != self.expect_flags {
            self.expect_flags = flags;
            unsafe { ot_session_input_expect(self.ctx, &self.session, flags) };
        }
    }

    /// Run session work and deliver queued output to stdout until idle.
    fn flush(&mut self) -> Result<()> {
        for _ in 0..10_000 {
            let mut r = versioned!(ot_session_pump_result);
            check(unsafe { ot_session_pump(self.ctx, &self.session, self.now_ns(), 64, &mut r) }, "pump")?;
            match r.status {
                OT_PUMP_IDLE => {
                    self.pump_deadline = 0;
                    return Ok(());
                }
                OT_PUMP_AGAIN => continue,
                OT_PUMP_OUTPUT_PENDING => {
                    let mut written = 0u32;
                    let st = unsafe { ot_session_drain_stdout(self.ctx, &self.session, 1 << 20, &mut written) };
                    if st == OT_UNSUPPORTED_RESOURCE {
                        self.drain_via_writer()?;
                    } else {
                        check(st, "drain_stdout")?;
                        if written == 0 {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                }
                OT_PUMP_WAIT_UNTIL => {
                    self.pump_deadline = r.deadline_ns;
                    return Ok(());
                }
                OT_PUMP_CLOSED => {
                    self.closed = true;
                    return Ok(());
                }
                _ => return Ok(()),
            }
        }
        Ok(())
    }

    fn drain_via_writer(&mut self) -> Result<()> {
        unsafe extern "C" fn write_cb(_: *mut std::os::raw::c_void, bytes: *const u8, n: u32) -> i64 {
            let r = libc::write(libc::STDOUT_FILENO, bytes as *const _, n as usize);
            if r < 0 {
                let e = *libc::__errno_location();
                if e == libc::EAGAIN || e == libc::EINTR {
                    return 0;
                }
                return -1;
            }
            r as i64
        }
        let mut written = 0u32;
        check(
            unsafe {
                ot_session_drain_output(self.ctx, &self.session, 1 << 20, &mut written, std::ptr::null_mut(), Some(write_cb))
            },
            "drain_output",
        )
    }

    fn check_resize(&mut self) -> Option<Event> {
        if !RESIZED.swap(false, Ordering::SeqCst) {
            return None;
        }
        let (w, h) = terminal_size();
        if w == self.width && h == self.height {
            return None;
        }
        let _ = self.flush();
        unsafe {
            ot_session_resize_renderer(self.ctx, &self.session, w as u32, h as u32);
            ot_buffer_resize(self.ctx, &self.buffer, w as u32, h as u32);
        }
        self.width = w;
        self.height = h;
        Some(Event::Resize(w, h))
    }

    /// Wait up to `timeout` for input, a wake-up, or a resize, and return parsed events.
    pub fn poll(&mut self, timeout: Duration) -> Vec<Event> {
        let mut out = Vec::new();
        if let Some(ev) = self.check_resize() {
            out.push(ev);
            return out;
        }
        let _ = self.flush();
        out.extend(self.read_input(timeout));
        if let Some(ev) = self.check_resize() {
            out.push(ev);
        }
        out
    }

    fn read_input(&mut self, timeout: Duration) -> Vec<Event> {
        let mut out = Vec::new();
        let now = self.now_ns();
        let mut wait_ms = timeout.as_millis() as i64;
        for d in [self.input_deadline, self.pump_deadline] {
            if d != 0 {
                wait_ms = wait_ms.min(((d.saturating_sub(now)) / 1_000_000) as i64 + 1);
            }
        }
        let mut fds = [
            libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: self.pipe.read, events: libc::POLLIN, revents: 0 },
        ];
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, wait_ms.max(0) as i32) };
        if n > 0 && fds[1].revents & libc::POLLIN != 0 {
            let mut sink = [0u8; 256];
            while unsafe { libc::read(self.pipe.read, sink.as_mut_ptr() as *mut _, sink.len()) } > 0 {}
            out.push(Event::Wake);
        }
        let mut bytes: Vec<u8> = Vec::new();
        if n > 0 && fds[0].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 8192];
            let r = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr() as *mut _, buf.len()) };
            if r > 0 {
                bytes.extend_from_slice(&buf[..r as usize]);
            }
        }
        let due = self.input_deadline != 0 && self.now_ns() >= self.input_deadline;
        if !bytes.is_empty() || due {
            self.feed(&bytes, &mut out);
        }
        out
    }

    fn feed(&mut self, mut bytes: &[u8], out: &mut Vec<Event>) {
        loop {
            let mut drain = versioned!(ot_input_drain);
            let st = unsafe {
                ot_session_input_feed(
                    self.ctx,
                    &self.session,
                    if bytes.is_empty() { std::ptr::null() } else { bytes.as_ptr() },
                    bytes.len() as u32,
                    self.now_ns(),
                    self.events.as_mut_ptr(),
                    self.events.len() as u32,
                    self.payload.as_mut_ptr(),
                    self.payload.len() as u32,
                    &mut drain,
                )
            };
            if st != OT_OK {
                return;
            }
            self.input_deadline = drain.deadline_ns;
            for i in 0..drain.count as usize {
                let ev = self.events[i];
                self.translate(ev, out);
            }
            bytes = &bytes[(drain.consumed as usize).min(bytes.len())..];
            if bytes.is_empty() || (drain.consumed == 0 && drain.count == 0) {
                break;
            }
        }
    }

    fn span(&self, off: u32, len: u32) -> &[u8] {
        let s = off as usize;
        let e = (s + len as usize).min(self.payload.len());
        &self.payload[s.min(e)..e]
    }

    fn translate(&mut self, ev: ot_input_event, out: &mut Vec<Event>) {
        match ev.kind {
            OT_INPUT_KEY => {
                if ev.action == OT_INPUT_KEY_RELEASE {
                    return;
                }
                let text = String::from_utf8_lossy(self.span(ev.text_offset, ev.text_len)).into_owned();
                out.push(Event::Key(KeyEvent {
                    code: ev.code,
                    mods: Mods(ev.modifiers),
                    text,
                    repeat: ev.action == OT_INPUT_KEY_REPEAT,
                }));
            }
            OT_INPUT_MOUSE => {
                let kind = match ev.action {
                    OT_INPUT_MOUSE_DOWN => MouseKind::Down,
                    OT_INPUT_MOUSE_UP => MouseKind::Up,
                    OT_INPUT_MOUSE_MOVE => MouseKind::Move,
                    OT_INPUT_MOUSE_DRAG => MouseKind::Drag,
                    OT_INPUT_MOUSE_SCROLL => match ev.code {
                        0 => MouseKind::ScrollUp,
                        1 => MouseKind::ScrollDown,
                        2 => MouseKind::ScrollLeft,
                        _ => MouseKind::ScrollRight,
                    },
                    _ => return,
                };
                out.push(Event::Mouse(MouseEvent { kind, button: ev.code, x: ev.x, y: ev.y, mods: Mods(ev.modifiers) }));
            }
            OT_INPUT_PASTE => {
                let chunk = self.span(ev.text_offset, ev.text_len).to_vec();
                if ev.flags & OT_INPUT_PASTE_START != 0 || self.paste.is_none() {
                    self.paste = Some(Vec::new());
                }
                if let Some(p) = self.paste.as_mut() {
                    p.extend_from_slice(&chunk);
                }
                if ev.flags & OT_INPUT_PASTE_END != 0 {
                    if let Some(p) = self.paste.take() {
                        out.push(Event::Paste(String::from_utf8_lossy(&p).into_owned()));
                    }
                }
            }
            OT_INPUT_FOCUS => out.push(Event::Focus(ev.action == 1)),
            OT_INPUT_REPLY => {
                let raw = self.span(ev.raw_offset, ev.raw_len).to_vec();
                let mut opts = versioned!(ot_session_control_options);
                opts.kind = OT_CONTROL_CAPABILITY_RESPONSE;
                if raw.len() <= 4096 {
                    unsafe { ot_session_control(self.ctx, &self.session, &opts, raw.as_ptr(), raw.len() as u32) };
                }
                // A primary device attributes reply (CSI ? ... c) ends the startup queries.
                if raw.starts_with(b"\x1b[?") && raw.ends_with(b"c") {
                    self.set_expect(0);
                }
            }
            _ => {}
        }
    }

    // ---- output helpers ---------------------------------------------------

    pub fn set_title(&mut self, title: &str) {
        let mut opts = versioned!(ot_session_control_options);
        opts.kind = OT_CONTROL_TITLE;
        let clean: String = title.chars().filter(|c| !c.is_control()).collect();
        unsafe { ot_session_control(self.ctx, &self.session, &opts, clean.as_ptr(), clean.len() as u32) };
    }

    /// Copy text to the system clipboard through OSC 52. Returns false when
    /// the terminal does not support it.
    pub fn copy_to_clipboard(&mut self, text: &str) -> bool {
        let mut written = 0u32;
        let st = unsafe {
            ot_session_clipboard(self.ctx, &self.session, 0, text.as_ptr(), text.len() as u32, &mut written)
        };
        let _ = self.flush();
        st == OT_OK && written > 0
    }

    pub fn notify(&mut self, title: &str, message: &str) {
        let mut written = 0u32;
        unsafe {
            ot_session_notification(
                self.ctx,
                &self.session,
                message.as_ptr(),
                message.len() as u32,
                title.as_ptr(),
                title.len() as u32,
                &mut written,
            )
        };
    }

    /// Place (Some) or hide (None) the hardware cursor for the next frame.
    pub fn set_cursor(&mut self, pos: Option<(u16, u16)>) {
        self.cursor = pos;
    }

    fn apply_cursor(&mut self) {
        let mut u = ot_session_cursor_update::default();
        u.fields = OT_CURSOR_POSITION | OT_CURSOR_STYLE | OT_CURSOR_BLINKING;
        match self.cursor {
            Some((x, y)) => {
                u.x = x as i32 + 1;
                u.y = y as i32 + 1;
                u.visible = 1;
            }
            None => {
                u.x = 1;
                u.y = 1;
                u.visible = 0;
            }
        }
        u.style = 1; // line
        u.blinking = 1;
        let mut opts = versioned!(ot_session_control_options);
        opts.kind = OT_CONTROL_CURSOR;
        let bytes = unsafe {
            std::slice::from_raw_parts(&u as *const _ as *const u8, std::mem::size_of::<ot_session_cursor_update>())
        };
        unsafe { ot_session_control(self.ctx, &self.session, &opts, bytes.as_ptr(), bytes.len() as u32) };
    }

    /// Start a frame: returns a painter over the offscreen buffer.
    pub fn frame(&mut self) -> Frame<'_> {
        Frame { term: self }
    }

    /// Present the offscreen buffer.
    pub fn present(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.flush()?;
        while self.clips > 0 {
            self.clips -= 1;
            unsafe {
                ot_buffer_stack(
                    self.ctx,
                    &self.buffer,
                    std::ptr::null(),
                    OT_BUFFER_STACK_POP_SCISSOR,
                    0,
                    0,
                    0,
                    0,
                    &ONE,
                    &mut 0.0,
                )
            };
        }
        let st = unsafe { ot_session_draw_buffer(self.ctx, &self.session, &self.buffer, 0, 0) };
        if st == OT_INVALID_PHASE || st == OT_OUTPUT_BUSY {
            return Ok(()); // Not yet active, or a frame is still in flight; try next tick.
        }
        check(st, "draw_buffer")?;
        self.apply_cursor();
        let mut result = 0u32;
        let st = unsafe { ot_session_render(self.ctx, &self.session, 0, &mut result) };
        if st == OT_INVALID_PHASE || st == OT_OUTPUT_BUSY {
            return Ok(());
        }
        check(st, "render")?;
        self.flush()
    }

    /// Restore the terminal. Called automatically on drop.
    pub fn shutdown(&mut self) {
        if self.ctx.is_null() {
            return;
        }
        unsafe {
            ot_session_close(self.ctx, &self.session);
        }
        let start = Instant::now();
        loop {
            let _ = self.flush();
            let mut state = 0u32;
            unsafe { ot_session_get_state(self.ctx, &self.session, &mut state) };
            if state != OT_SESSION_OPEN && state != OT_SESSION_CLOSING {
                break;
            }
            if self.closed {
                break;
            }
            if start.elapsed() > Duration::from_millis(1500) {
                unsafe { ot_session_drop_output(self.ctx, &self.session) };
                for _ in 0..1000 {
                    let mut st = 0u32;
                    if unsafe { ot_session_pump_exit(self.ctx, &self.session, &mut st) } != OT_OK {
                        break;
                    }
                    if st == OT_PUMP_CLOSED {
                        break;
                    }
                    let mut w = 0u32;
                    unsafe { ot_session_drain_stdout(self.ctx, &self.session, 1 << 20, &mut w) };
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        unsafe {
            ot_terminal_flush_input();
            ot_buffer_destroy(self.ctx, &self.buffer);
            if ot_session_destroy(self.ctx, &self.session) != OT_OK {
                ot_session_cancel(self.ctx, &self.session);
                ot_session_destroy(self.ctx, &self.session);
            }
            ot_context_destroy(self.ctx);
        }
        self.ctx = std::ptr::null_mut();
        self.raw.take();
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Immediate-mode painter. Coordinates are signed cells; everything clips to
/// the screen and to the active scissor.
pub struct Frame<'a> {
    term: &'a mut Terminal,
}

impl Frame<'_> {
    pub fn width(&self) -> u16 {
        self.term.width
    }
    pub fn height(&self) -> u16 {
        self.term.height
    }

    fn draw<T>(&mut self, rec: &T, text: &[u8], bottom: &[u8]) {
        unsafe {
            ot_buffer_draw(
                self.term.ctx,
                &self.term.buffer,
                std::ptr::null(),
                rec as *const T as *const ot_buffer_draw_header,
                std::ptr::null(),
                if text.is_empty() { std::ptr::null() } else { text.as_ptr() },
                text.len() as u32,
                if bottom.is_empty() { std::ptr::null() } else { bottom.as_ptr() },
                bottom.len() as u32,
            );
        }
    }

    pub fn clear(&mut self, bg: Rgba) {
        let mut r = ot_buffer_draw_clear::default();
        r.header = header::<ot_buffer_draw_clear>(OT_BUFFER_DRAW_CLEAR, 0);
        r.background = bg.words();
        self.draw(&r, &[], &[]);
    }

    pub fn fill(&mut self, x: i32, y: i32, w: u32, h: u32, bg: Rgba) {
        let mut r = ot_buffer_draw_fill::default();
        r.header = header::<ot_buffer_draw_fill>(OT_BUFFER_DRAW_FILL, 0);
        r.x = x;
        r.y = y;
        r.width = w;
        r.height = h;
        r.background = bg.words();
        self.draw(&r, &[], &[]);
    }

    /// Draw a single line of text. `bg = None` keeps the existing background.
    /// `max_width > 0` truncates with an ellipsis.
    pub fn text(&mut self, x: i32, y: i32, s: &str, fg: Rgba, bg: Option<Rgba>, attrs: u32, max_width: u32) {
        if s.is_empty() {
            return;
        }
        let mut flags = 0;
        if bg.is_some() {
            flags |= OT_BUFFER_DRAW_HAS_BACKGROUND;
        }
        if max_width > 0 {
            flags |= OT_BUFFER_DRAW_FIT_ELLIPSIS;
        }
        let mut r = ot_buffer_draw_text_record::default();
        r.header = header::<ot_buffer_draw_text_record>(OT_BUFFER_DRAW_TEXT, flags);
        r.x = x;
        r.y = y;
        r.attributes = attrs;
        r.foreground = fg.words();
        r.background = bg.unwrap_or(Rgba::TRANSPARENT).words();
        r.max_width = max_width;
        let bytes = s.as_bytes();
        let n = bytes.len().min(OT_BUFFER_TEXT_BYTES_MAX);
        self.draw(&r, &bytes[..n], &[]);
    }

    pub fn cell(&mut self, x: i32, y: i32, ch: char, fg: Rgba, bg: Rgba, attrs: u32) {
        let mut r = ot_buffer_draw_cell::default();
        r.header = header::<ot_buffer_draw_cell>(OT_BUFFER_DRAW_CELL, 0);
        r.x = x;
        r.y = y;
        r.character = ch as u32;
        r.attributes = attrs;
        r.foreground = fg.words();
        r.background = bg.words();
        self.draw(&r, &[], &[]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn boxed(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        style: BorderStyle,
        fg: Rgba,
        bg: Option<Rgba>,
        title: Option<(&str, Align, Rgba)>,
        bottom_title: Option<(&str, Align)>,
    ) {
        let mut r = ot_buffer_draw_box::default();
        r.header = header::<ot_buffer_draw_box>(OT_BUFFER_DRAW_BOX, 0);
        r.x = x;
        r.y = y;
        r.width = w;
        r.height = h;
        let mut packed = OT_BORDER_ALL;
        if bg.is_some() {
            packed |= 1 << 4;
        }
        if let Some((_, a, _)) = title {
            packed |= (a as u32) << 5;
        }
        if let Some((_, a)) = bottom_title {
            packed |= (a as u32) << 7;
        }
        r.packed_options = packed;
        r.foreground = fg.words();
        r.background = bg.unwrap_or(Rgba::TRANSPARENT).words();
        r.title_color = title.map(|t| t.2).unwrap_or(fg).words();
        r.border_chars = style.chars();
        let t = title.map(|t| t.0).unwrap_or("");
        let b = bottom_title.map(|t| t.0).unwrap_or("");
        self.draw(&r, t.as_bytes(), b.as_bytes());
    }

    pub fn push_clip(&mut self, x: i32, y: i32, w: u32, h: u32) {
        let mut out = 0.0f32;
        let st = unsafe {
            ot_buffer_stack(
                self.term.ctx,
                &self.term.buffer,
                std::ptr::null(),
                OT_BUFFER_STACK_PUSH_SCISSOR,
                x,
                y,
                w,
                h,
                &ONE,
                &mut out,
            )
        };
        if st == OT_OK {
            self.term.clips += 1;
        }
    }

    pub fn pop_clip(&mut self) {
        if self.term.clips == 0 {
            return;
        }
        let mut out = 0.0f32;
        unsafe {
            ot_buffer_stack(
                self.term.ctx,
                &self.term.buffer,
                std::ptr::null(),
                OT_BUFFER_STACK_POP_SCISSOR,
                0,
                0,
                0,
                0,
                &ONE,
                &mut out,
            )
        };
        self.term.clips -= 1;
    }
}

fn header<T>(operation: u32, flags: u32) -> ot_buffer_draw_header {
    ot_buffer_draw_header {
        struct_size: std::mem::size_of::<T>() as u32,
        abi_version: OT_CONTEXT_ABI_VERSION,
        operation,
        flags,
    }
}

pub const OT_BUFFER_TEXT_BYTES_MAX: usize = 65_536;
static ONE: f32 = 1.0;
