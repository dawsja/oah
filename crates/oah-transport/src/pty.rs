//! Interactive shells in a pseudo-terminal, wherever the project is: PowerShell (Windows) or the
//! user's shell (Linux) on this computer, the user's login shell in a WSL distro through
//! `wsl.exe`, or on an SSH host through `ssh -t`. A remote shell is only known once it runs, so
//! the start script names it with a private escape sequence, `ESC ] 6973 ; shell=<name> BEL`,
//! that the terminal reads and doesn't print.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oah_core::settings::Location;
use oah_core::ssh::{sh_quote, valid_host};
use oah_core::wsl::valid_distro;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

/// The OSC number of the "this is the shell" announcement.
pub const SHELL_OSC: u16 = 6973;

/// What to start in the terminal.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellSpec {
    pub program: String,
    pub args: Vec<String>,
    /// The folder to start in, on this computer (none: the home folder).
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    /// The shell's name (`pwsh`, `bash`…) when it's known before it starts; remote shells say
    /// it themselves.
    pub shell: Option<String>,
}

/// The shell for a project at `location`, starting in `cwd` (a folder there).
pub fn shell_spec(location: &Location, cwd: &str) -> Result<ShellSpec, String> {
    match location {
        Location::Local => {
            let (program, args) = local_shell();
            let dir = Some(cwd).filter(|d| !d.is_empty() && Path::new(d).is_dir()).map(String::from);
            let mut env = vec![("TERM_PROGRAM".to_string(), "oah".to_string())];
            if !cfg!(windows) {
                env.push(("TERM".into(), "xterm-256color".into()));
                env.push(("COLORTERM".into(), "truecolor".into()));
            }
            Ok(ShellSpec { shell: Some(shell_name(&program)), program, args, cwd: dir, env })
        }
        Location::Wsl { distro } => {
            if !cfg!(windows) {
                return Err("WSL projects open on Windows only".into());
            }
            if !valid_distro(distro) {
                return Err(format!("not a valid WSL distro name: {distro:?}"));
            }
            let args = ["-d", distro, "-e", "sh", "-lc", &login_script(cwd)].map(String::from).to_vec();
            Ok(ShellSpec {
                program: crate::wsl::exe().to_string_lossy().into_owned(),
                args,
                // wsl.exe's own folder must exist (see wsl::command).
                cwd: std::env::var("SystemRoot").ok(),
                env: vec![("WSL_UTF8".into(), "1".into())],
                shell: None,
            })
        }
        Location::Ssh { host } => {
            if !valid_host(host) {
                return Err(format!("not a valid SSH host: {host:?}"));
            }
            let program = which::which("ssh").map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "ssh".into());
            let line = format!("sh -lc {}", sh_quote(&login_script(cwd)));
            // Not BatchMode: this is a real terminal, so a password or host-key prompt can be answered.
            let args = ["-t", "-o", "ServerAliveInterval=30", "--", host, &line].map(String::from).to_vec();
            Ok(ShellSpec { program, args, cwd: None, env: vec![("TERM".into(), "xterm-256color".into())], shell: None })
        }
    }
}

/// The shell a terminal on this computer runs: PowerShell 7 if it's installed, else Windows
/// PowerShell; on Linux `$SHELL`, else bash, else sh.
fn local_shell() -> (String, Vec<String>) {
    let path = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
    if cfg!(windows) {
        let exe = which::which("pwsh").or_else(|_| which::which("powershell")).map(path).unwrap_or_else(|_| "powershell.exe".into());
        return (exe, vec!["-NoLogo".into()]);
    }
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty() && Path::new(s).exists());
    let shell = shell.or_else(|| ["/bin/bash", "/bin/sh"].into_iter().find(|s| Path::new(s).exists()).map(String::from));
    (shell.unwrap_or_else(|| "sh".into()), vec![])
}

/// The shell a new terminal at `location` will run, when that's known without starting it.
pub fn known_shell(location: &Location) -> Option<String> {
    matches!(location, Location::Local).then(|| shell_name(&local_shell().0))
}

/// `C:\…\pwsh.exe` → `pwsh`, `/usr/bin/zsh` → `zsh`.
pub fn shell_name(program: &str) -> String {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let stem = file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file);
    stem.to_ascii_lowercase()
}

/// The start script for a remote shell: go to the folder (or home, saying so, when it's gone),
/// find the user's login shell, name it, then become it.
pub fn login_script(cwd: &str) -> String {
    let cd = crate::cd_line(cwd);
    let gone = sh_quote(&format!("oah: {cwd} isn't there; starting in the home folder"));
    format!(
        r#"{cd} 2>/dev/null || {{ printf '\033[2m%s\033[0m\r\n' {gone}; cd "$HOME"; }}
oah_sh=${{SHELL:-}}
[ -x "$oah_sh" ] || oah_sh=$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)
[ -x "$oah_sh" ] || oah_sh=/bin/sh
export TERM=xterm-256color COLORTERM=truecolor TERM_PROGRAM=oah
printf '\033]{SHELL_OSC};shell=%s\007' "${{oah_sh##*/}}"
exec "$oah_sh" -l"#
    )
}

/// What a terminal reports.
#[derive(Debug, Clone, PartialEq)]
pub enum PtyEvent {
    /// Output, as text (a character split across reads waits for the rest).
    Data(String),
    /// The shell ended (its exit code, when there is one). Nothing follows.
    Exit(Option<u32>),
}

/// A running shell. Dropping it ends the shell.
pub struct Pty {
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

impl Pty {
    /// Starts `spec` in a `cols`×`rows` terminal; `on_event` is called from reader threads.
    pub fn spawn(spec: &ShellSpec, cols: u16, rows: u16, on_event: impl Fn(PtyEvent) + Send + Sync + 'static) -> Result<Pty, String> {
        let pair = native_pty_system().openpty(size(cols, rows)).map_err(|e| format!("couldn't open a terminal: {e}"))?;
        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        match &spec.cwd {
            Some(dir) => cmd.cwd(dir),
            None => cmd.cwd(oah_core::paths::home_dir()),
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| format!("couldn't start {}: {e}", spec.program))?;
        // Only the shell holds the terminal's other end, so the output ends when it does.
        drop(pair.slave);
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let master = Arc::new(Mutex::new(Some(pair.master)));
        let on_event = Arc::new(on_event);
        let done = Arc::new(AtomicBool::new(false));
        let (drained_tx, drained) = std::sync::mpsc::channel::<()>();

        {
            let (on_event, done) = (on_event.clone(), done.clone());
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 16 * 1024];
                let mut pending = Vec::new();
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            pending.extend_from_slice(&buf[..n]);
                            let text = take_utf8(&mut pending);
                            if !text.is_empty() && !done.load(Ordering::SeqCst) {
                                on_event(PtyEvent::Data(text));
                            }
                        }
                    }
                }
                if !pending.is_empty() && !done.load(Ordering::SeqCst) {
                    on_event(PtyEvent::Data(String::from_utf8_lossy(&pending).into_owned()));
                }
                let _ = drained_tx.send(());
            });
        }
        {
            let master = master.clone();
            std::thread::spawn(move || {
                let code = child.wait().ok().map(|s| s.exit_code());
                // Windows' pseudo console keeps the output open until it's closed.
                drop(master.lock().unwrap().take());
                // The last output first; a background job still holding the terminal doesn't
                // keep the exit waiting.
                let _ = drained.recv_timeout(Duration::from_secs(1));
                done.store(true, Ordering::SeqCst);
                on_event(PtyEvent::Exit(code));
            });
        }
        Ok(Pty { master, writer: Mutex::new(writer), killer: Mutex::new(killer) })
    }

    /// Types `data` into the shell.
    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data).and_then(|_| w.flush()).map_err(|e| format!("the terminal is closed: {e}"))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        match self.master.lock().unwrap().as_ref() {
            Some(m) => m.resize(size(cols, rows)).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }

    /// Ends the shell (its exit is still reported).
    pub fn kill(&self) {
        let _ = self.killer.lock().unwrap().kill();
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.kill();
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows: rows.max(2), cols: cols.max(10), pixel_width: 0, pixel_height: 0 }
}

/// The text at the front of `buf`, leaving an unfinished character for the next read (bytes
/// that can never be UTF-8 become U+FFFD).
fn take_utf8(buf: &mut Vec<u8>) -> String {
    let mut out = String::new();
    loop {
        match std::str::from_utf8(buf) {
            Ok(s) => {
                out.push_str(s);
                buf.clear();
                return out;
            }
            Err(e) => {
                let good = e.valid_up_to();
                out.push_str(std::str::from_utf8(&buf[..good]).unwrap_or_default());
                match e.error_len() {
                    None => {
                        buf.drain(..good);
                        return out;
                    }
                    Some(bad) => {
                        out.push('\u{FFFD}');
                        buf.drain(..good + bad);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn keeps_split_characters_for_the_next_read() {
        let mut b = "aé".as_bytes()[..2].to_vec();
        assert_eq!(take_utf8(&mut b), "a");
        b.extend_from_slice(&"é".as_bytes()[1..]);
        b.extend_from_slice(b"\xffz");
        assert_eq!(take_utf8(&mut b), "é\u{FFFD}z");
        assert!(b.is_empty());
    }

    #[test]
    fn names_shells() {
        assert_eq!(shell_name(r"C:\Program Files\PowerShell\7\pwsh.exe"), "pwsh");
        assert_eq!(shell_name(r"C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.EXE"), "powershell");
        assert_eq!(shell_name("/usr/bin/zsh"), "zsh");
        assert_eq!(known_shell(&Location::Ssh { host: "box".into() }), None);
    }

    #[test]
    fn ssh_terminals_run_the_start_script() {
        let s = shell_spec(&Location::Ssh { host: "devbox".into() }, "~/code/it's").unwrap();
        assert_eq!(&s.args[..5], ["-t", "-o", "ServerAliveInterval=30", "--", "devbox"]);
        assert_eq!(s.args[5], format!("sh -lc {}", sh_quote(&login_script("~/code/it's"))));
        assert!(shell_spec(&Location::Ssh { host: "-oProxyCommand=x".into() }, "/").is_err());
    }

    /// The start script under a real `sh`, with `echo` standing in for the login shell.
    #[test]
    #[cfg(unix)]
    fn the_start_script_names_the_shell_and_falls_back_home() {
        let run = |cwd: &str| {
            let out = std::process::Command::new("sh").arg("-c").arg(login_script(cwd)).env("SHELL", "/bin/echo").output().unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        assert_eq!(run("/"), "\x1b]6973;shell=echo\x07-l\n");
        let gone = run("/no/such/oah folder");
        assert!(gone.contains("oah: /no/such/oah folder isn't there"), "{gone}");
        assert!(gone.ends_with("\x1b]6973;shell=echo\x07-l\n"), "{gone}");
    }

    #[test]
    #[cfg(unix)]
    fn runs_a_shell_in_a_pty() {
        let spec = ShellSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "stty -echo; [ -t 0 ] && printf 'tty '; read x; printf 'got:%s' \"$x\"; exit 3".into()],
            cwd: Some("/".into()),
            env: vec![],
            shell: None,
        };
        let (tx, rx) = mpsc::channel();
        let pty = Pty::spawn(&spec, 80, 24, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        pty.resize(100, 30).unwrap();
        pty.write(b"h\xc3\xa9llo\n").unwrap();
        let mut text = String::new();
        let code = loop {
            match rx.recv_timeout(Duration::from_secs(10)).expect("the shell answers") {
                PtyEvent::Data(d) => text.push_str(&d),
                PtyEvent::Exit(c) => break c,
            }
        };
        assert!(text.contains("tty got:héllo"), "{text:?}");
        assert_eq!(code, Some(3));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "nothing after the exit");
    }

    /// Output until `needle` shows up (or the shell ends: `Err` with what it printed).
    #[cfg(windows)]
    fn until(rx: &mpsc::Receiver<PtyEvent>, needle: &str) -> Result<String, String> {
        let mut text = String::new();
        let end = std::time::Instant::now() + Duration::from_secs(60);
        while !text.contains(needle) {
            match rx.recv_timeout(end.saturating_duration_since(std::time::Instant::now())) {
                Ok(PtyEvent::Data(d)) => text.push_str(&d),
                Ok(PtyEvent::Exit(c)) => return Err(format!("exited ({c:?}) before {needle:?}: {text:?}")),
                Err(_) => return Err(format!("no {needle:?} in {text:?}")),
            }
        }
        Ok(text)
    }

    #[cfg(windows)]
    fn exit_code(rx: &mpsc::Receiver<PtyEvent>) -> Option<u32> {
        loop {
            match rx.recv_timeout(Duration::from_secs(30)).expect("the shell ends") {
                PtyEvent::Exit(c) => return c,
                PtyEvent::Data(_) => {}
            }
        }
    }

    /// PowerShell in a Windows pseudo console, and a distro's login shell through wsl.exe:
    /// `OAH_TEST_DISTRO=Ubuntu cargo test -p oah-transport -- --ignored` (on Windows).
    #[test]
    #[cfg(windows)]
    #[ignore]
    fn real_windows_shells() {
        let home = oah_core::paths::home_dir().to_string_lossy().into_owned();
        let spec = shell_spec(&Location::Local, &home).unwrap();
        assert!(matches!(spec.shell.as_deref(), Some("pwsh" | "powershell")), "{spec:?}");
        let (tx, rx) = mpsc::channel();
        let ps = Pty::spawn(&spec, 120, 30, move |e| drop(tx.send(e))).unwrap();
        // The pseudo console asks where the cursor is first (xterm.js answers in the app).
        until(&rx, "\x1b[6n").unwrap();
        ps.write(b"\x1b[1;1R").unwrap();
        ps.write(b"Write-Output ('oah' + 'ok')\r").unwrap();
        until(&rx, "oahok").unwrap();
        ps.write(b"exit 4\r").unwrap();
        assert_eq!(exit_code(&rx), Some(4));

        let distro = std::env::var("OAH_TEST_DISTRO").unwrap_or_else(|_| "Ubuntu".into());
        let spec = shell_spec(&Location::Wsl { distro }, "/tmp").unwrap();
        let (tx, rx) = mpsc::channel();
        let sh = Pty::spawn(&spec, 120, 30, move |e| drop(tx.send(e))).unwrap();
        until(&rx, "\x1b[6n").unwrap();
        sh.write(b"\x1b[1;1R").unwrap();
        let hello = until(&rx, "\x1b]6973;shell=").unwrap();
        println!("{hello:?}");
        sh.write(b"echo \"$((6*7)) $(pwd) $TERM\"\r").unwrap();
        until(&rx, "42 /tmp xterm-256color").unwrap();
        sh.resize(90, 20).unwrap();
        sh.write(b"stty size\r").unwrap();
        until(&rx, "20 90").unwrap();
        sh.write(b"exit\r").unwrap();
        assert_eq!(exit_code(&rx), Some(0));
    }

    #[test]
    #[cfg(unix)]
    fn killing_reports_the_exit() {
        let spec = ShellSpec { program: "/bin/sh".into(), args: vec![], cwd: None, env: vec![], shell: None };
        let (tx, rx) = mpsc::channel();
        let pty = Pty::spawn(&spec, 80, 24, move |e| {
            if let PtyEvent::Exit(_) = e {
                let _ = tx.send(());
            }
        })
        .unwrap();
        drop(pty);
        rx.recv_timeout(Duration::from_secs(10)).expect("an exit");
    }
}
