//! Running a program in a project's folder, wherever the project is: on this computer, on an
//! SSH host, or in a WSL distro through `wsl.exe -e` (the program's stdio is wsl.exe's). On an
//! SSH host it runs under `oah-host` when oah can put that there ([`host`]: one connection per
//! host, and agents that outlive a dropped one), else through its own `ssh -T`. Remote commands
//! are one POSIX `sh` script with every word single-quoted; nothing from the user is spliced in
//! unquoted.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};

use oah_core::settings::{EnvVar, Location};
pub mod host;
pub mod pty;
pub mod remote;
pub mod wsl;

use oah_core::ssh::{sh_quote, valid_host};

/// What to run and where.
#[derive(Debug, Clone)]
pub struct Spec<'a> {
    pub location: &'a Location,
    /// The folder to run in (on `location`).
    pub cwd: &'a str,
    /// The program: a name to look up on PATH, or a path.
    pub program: &'a str,
    pub args: Vec<String>,
    pub env: &'a [EnvVar],
}

/// Per-user folders installers put CLIs in without always getting them onto PATH for every
/// shell (Claude Code's native installer; OpenCode's, which adds it in `.bashrc` only; Grok
/// Build's, whose PATH change a running oah on Windows doesn't see). A
/// program by name not on PATH is looked for there too; PATH still wins.
pub const USER_BIN_DIRS: [&str; 3] = [".local/bin", ".opencode/bin", ".grok/bin"];

/// A program on this computer: on PATH (so `claude` finds `claude.exe` or `claude.cmd`), else
/// in one of the [`USER_BIN_DIRS`].
pub fn find_local(program: &str) -> Option<PathBuf> {
    if let Ok(p) = which::which(program) {
        return Some(p);
    }
    if program.contains(['/', '\\']) {
        return None;
    }
    let home = oah_core::paths::home_dir();
    let names: Vec<String> =
        if cfg!(windows) { ["exe", "cmd"].iter().map(|e| format!("{program}.{e}")).collect() } else { vec![program.to_string()] };
    USER_BIN_DIRS.iter().flat_map(|d| names.iter().map(|n| home.join(d).join(n)).collect::<Vec<_>>()).find(|p| p.is_file())
}

/// Starts `spec` with piped stdio: under the host's `oah-host` when it has one, else as a
/// process here ([`command`]).
pub fn spawn(spec: &Spec) -> Result<Proc, String> {
    if let Location::Ssh { host } = spec.location {
        if valid_host(host) {
            if let Some(session) = host::session_for(host) {
                let p = session.spawn(remote_script(spec, false))?;
                return Ok(Proc {
                    stdin: Some(Stdin::Host(p.stdin())),
                    stdout: Some(Out::Host(p.stdout())),
                    stderr: Some(Out::Host(p.stderr())),
                    inner: Inner::Host(p),
                });
            }
        }
    }
    let mut child = command(spec)?.spawn().map_err(|e| e.to_string())?;
    Ok(Proc {
        stdin: child.stdin.take().map(Stdin::Child),
        stdout: child.stdout.take().map(Out::Child),
        stderr: child.stderr.take().map(Out::ChildErr),
        inner: Inner::Child(child),
    })
}

/// A running program, here or on a host. Like [`std::process::Child`]: take its stdio, then
/// wait for it or [`stop_tree`] it.
pub struct Proc {
    pub stdin: Option<Stdin>,
    pub stdout: Option<Out>,
    pub stderr: Option<Out>,
    inner: Inner,
}

enum Inner {
    Child(Child),
    Host(host::HostProc),
}

/// How a program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    code: Option<i32>,
}

impl ExitStatus {
    /// Its exit code; `None` when a signal ended it or the host lost it.
    pub fn code(&self) -> Option<i32> {
        self.code
    }

    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

impl Proc {
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        match &mut self.inner {
            Inner::Child(c) => c.wait().map(|s| ExitStatus { code: s.code() }),
            Inner::Host(h) => Ok(ExitStatus { code: h.wait().code }),
        }
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match &mut self.inner {
            Inner::Child(c) => c.try_wait().map(|s| s.map(|s| ExitStatus { code: s.code() })),
            Inner::Host(h) => Ok(h.try_wait().map(|e| ExitStatus { code: e.code })),
        }
    }

    /// Kills the program (on a host: its whole process group).
    pub fn kill(&mut self) -> io::Result<()> {
        match &mut self.inner {
            Inner::Child(c) => c.kill(),
            Inner::Host(h) => {
                h.kill();
                Ok(())
            }
        }
    }
}

/// A [`Proc`]'s stdin. Dropping it closes it.
pub enum Stdin {
    Child(ChildStdin),
    Host(host::HostStdin),
}

impl Write for Stdin {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stdin::Child(w) => w.write(buf),
            Stdin::Host(w) => w.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stdin::Child(w) => w.flush(),
            Stdin::Host(w) => w.flush(),
        }
    }
}

/// A [`Proc`]'s stdout or stderr.
pub enum Out {
    Child(ChildStdout),
    ChildErr(ChildStderr),
    Host(host::HostOut),
}

impl Read for Out {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Out::Child(r) => r.read(buf),
            Out::ChildErr(r) => r.read(buf),
            Out::Host(r) => r.read(buf),
        }
    }
}

/// A `Command` ready to spawn, with piped stdio and no console window, for running `spec` as a
/// process here (on an SSH host: through its own `ssh -T`). Local programs are found with
/// [`find_local`]. [`spawn`] is what to use to start a program.
pub fn command(spec: &Spec) -> Result<Command, String> {
    let mut cmd = match spec.location {
        Location::Local => {
            let exe: PathBuf = find_local(spec.program).ok_or_else(|| format!("{} isn't installed or isn't on PATH", spec.program))?;
            let mut c = Command::new(exe);
            c.args(&spec.args).current_dir(spec.cwd);
            for v in spec.env.iter().filter(|v| !v.key.is_empty()) {
                c.env(&v.key, &v.value);
            }
            c
        }
        Location::Ssh { host } => {
            if !valid_host(host) {
                return Err(format!("not a valid SSH host: {host:?}"));
            }
            let mut c = Command::new("ssh");
            // A host that doesn't answer fails in seconds, not after the command's whole limit.
            c.args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=30", "--", host])
                .arg(remote_line(spec));
            c
        }
        Location::Wsl { distro } => wsl::command(distro, &remote_script(spec, true))?,
    };
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    no_window(&mut cmd);
    // Its own process group, so [`stop_tree`] can end what it starts too (an agent's shell
    // commands, dev servers, MCP servers), not just the program itself.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    Ok(cmd)
}

/// Ends `proc` and what it started: its process group on Unix (see [`command`]) and on a host,
/// its process tree on Windows. A program that exits by itself once its stdin is closed (close
/// it first) gets `grace` to do so, and to finish writing its session.
pub fn stop_tree(proc: &mut Proc, grace: std::time::Duration) {
    match &mut proc.inner {
        Inner::Child(c) => stop_child(c, grace),
        Inner::Host(h) => h.stop(grace),
    }
}

fn stop_child(child: &mut Child, grace: std::time::Duration) {
    let until = std::time::Instant::now() + grace;
    while std::time::Instant::now() < until {
        match child.try_wait() {
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => break,
        }
    }
    #[cfg(unix)]
    {
        // The group outlives its leader while anything in it runs.
        let group = child.id() as i32;
        if group > 0 {
            // SAFETY: a plain signal to a process group oah created.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        if matches!(child.try_wait(), Ok(None)) {
            let mut kill = Command::new("taskkill");
            kill.args(["/T", "/F", "/PID", &child.id().to_string()]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            no_window(&mut kill);
            let _ = kill.status();
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The remote shell line for ssh: a login shell (`sh -lc`, so tools installed for the user are
/// on PATH) running [`remote_script`].
pub fn remote_line(spec: &Spec) -> String {
    format!("sh -lc {}", sh_quote(&remote_script(spec, false)))
}

/// `cd` to the folder (`~` expanded), then `exec env K=V program args…`. A program given by
/// name is looked up on PATH, then in the [`USER_BIN_DIRS`]. With `skip_windows` (WSL), the
/// lookup leaves out the Windows drives WSL adds (`/mnt/c/…`), so a Windows copy is never run
/// on Linux files; the program itself still gets the whole PATH.
pub fn remote_script(spec: &Spec, skip_windows: bool) -> String {
    let cd = cd_line(spec.cwd);
    let mut words = vec!["exec".to_string(), "env".to_string()];
    for v in spec.env.iter().filter(|v| valid_env_key(&v.key)) {
        words.push(sh_quote(&format!("{}={}", v.key, v.value)));
    }
    let by_name = !spec.program.contains('/');
    let mut lines = vec![cd + " || exit 1"];
    let user_bins: Vec<String> = USER_BIN_DIRS.iter().map(|d| format!("$HOME/{d}")).collect();
    if skip_windows && by_name {
        lines.push(LINUX_PATH.trim().to_string());
        lines.push(format!(
            "oah_prog=$(PATH=\"$oah_path:{bins}\" command -v {p}) || {{ printf '%s\\n' {msg} >&2; exit 127; }}",
            bins = user_bins.join(":"),
            p = sh_quote(spec.program),
            msg = sh_quote(&format!("{} isn't installed in this distro (a Windows copy doesn't count)", spec.program)),
        ));
        words.push("\"$oah_prog\"".into());
    } else if by_name {
        // Not found: run it by name anyway, so the shell says so (exit 127).
        lines.push(format!(
            "oah_prog=$(PATH=\"$PATH:{bins}\" command -v {p}) || oah_prog={p}",
            bins = user_bins.join(":"),
            p = sh_quote(spec.program),
        ));
        words.push("\"$oah_prog\"".into());
    } else {
        words.push(sh_quote(spec.program));
    }
    words.extend(spec.args.iter().map(|a| sh_quote(a)));
    lines.push(words.join(" "));
    lines.join("\n")
}

/// `cd <folder>`, with a leading `~` meaning the home folder.
pub(crate) fn cd_line(cwd: &str) -> String {
    match cwd.strip_prefix("~/") {
        Some(rest) => format!("cd \"$HOME\"/{}", sh_quote(rest)),
        None if cwd == "~" || cwd.is_empty() => "cd \"$HOME\"".into(),
        None => format!("cd {}", sh_quote(cwd)),
    }
}

/// `$oah_path`: `$PATH` without WSL's Windows drives.
const LINUX_PATH: &str = r#"
set -f; oah_path=; oah_ifs=$IFS; IFS=:; for oah_d in $PATH; do case $oah_d in /mnt/[a-z]|/mnt/[a-z]/*) ;; *) oah_path=${oah_path:+$oah_path:}$oah_d;; esac; done; IFS=$oah_ifs; set +f
"#;

fn valid_env_key(k: &str) -> bool {
    !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit())
}

/// No console window flashing up on Windows.
pub fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let _ = cmd;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_quoted_remote_line() {
        let loc = Location::Ssh { host: "devbox".into() };
        let env = [
            EnvVar { key: "API_KEY".into(), value: "a'b".into(), in_keychain: false },
            EnvVar { key: "bad key".into(), value: "x".into(), in_keychain: false },
        ];
        let spec = Spec { location: &loc, cwd: "~/code/my app", program: "claude", args: vec!["-p".into(), "it's".into()], env: &env };
        let script = remote_script(&spec, false);
        assert_eq!(
            script,
            "cd \"$HOME\"/'code/my app' || exit 1\noah_prog=$(PATH=\"$PATH:$HOME/.local/bin:$HOME/.opencode/bin:$HOME/.grok/bin\" command -v 'claude') || oah_prog='claude'\nexec env 'API_KEY=a'\\''b' \"$oah_prog\" '-p' 'it'\\''s'"
        );
        assert_eq!(remote_line(&spec), format!("sh -lc {}", sh_quote(&script)));
    }

    #[test]
    #[cfg(unix)]
    fn wsl_scripts_say_when_a_program_is_missing() {
        let loc = Location::Wsl { distro: "Ubuntu".into() };
        let spec = Spec { location: &loc, cwd: "/", program: "oah-no-such-tool", args: vec![], env: &[] };
        let out = Command::new("sh").arg("-c").arg(remote_script(&spec, true)).output().unwrap();
        assert_eq!(out.status.code(), Some(127));
        assert!(String::from_utf8_lossy(&out.stderr).contains("isn't installed in this distro"));
        let spec = Spec { location: &loc, cwd: "/", program: "printf", args: vec!["%s|".into(), "a b".into()], env: &[] };
        let out = Command::new("sh").arg("-c").arg(remote_script(&spec, true)).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a b|");
    }

    #[test]
    #[cfg(unix)]
    fn the_remote_line_survives_a_shell() {
        let loc = Location::Ssh { host: "x".into() };
        let spec =
            Spec { location: &loc, cwd: "/", program: "printf", args: vec!["%s|".into(), "a b".into(), "it's $HOME".into()], env: &[] };
        let out = Command::new("sh").arg("-c").arg(remote_line(&spec)).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a b|it's $HOME|");
    }

    #[test]
    #[cfg(unix)]
    fn finds_programs_in_user_bin_folders() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".opencode/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("oah-test-tool"), "#!/bin/sh\necho found-it\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(bin.join("oah-test-tool"), std::fs::Permissions::from_mode(0o755)).unwrap();
        for skip_windows in [false, true] {
            let loc = Location::Wsl { distro: "x".into() };
            let spec = Spec { location: &loc, cwd: "/", program: "oah-test-tool", args: vec![], env: &[] };
            let out = Command::new("sh").arg("-c").arg(remote_script(&spec, skip_windows)).env("HOME", home.path()).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), "found-it\n", "skip_windows={skip_windows}");
        }
    }

    #[test]
    fn rejects_option_like_hosts() {
        let loc = Location::Ssh { host: "-oProxyCommand=calc".into() };
        let spec = Spec { location: &loc, cwd: "/", program: "ls", args: vec![], env: &[] };
        assert!(command(&spec).is_err());
    }
}

/// What a finished command printed.
#[derive(Debug, Clone)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// Runs `spec` to completion with `input` on stdin (none: closed), giving up after `limit`.
pub fn run(spec: &Spec, input: Option<&[u8]>, limit: std::time::Duration) -> Result<Output, String> {
    let child = spawn(spec).map_err(|e| format!("couldn't run {}: {e}", spec.program))?;
    finish(child, spec, input, limit)
}

/// [`run`] as a process here, never under `oah-host` (setting a host up uses this).
pub(crate) fn run_direct(spec: &Spec, input: Option<&[u8]>, limit: std::time::Duration) -> Result<Output, String> {
    let mut child = command(spec)?.spawn().map_err(|e| format!("couldn't run {}: {e}", spec.program))?;
    let proc = Proc {
        stdin: child.stdin.take().map(Stdin::Child),
        stdout: child.stdout.take().map(Out::Child),
        stderr: child.stderr.take().map(Out::ChildErr),
        inner: Inner::Child(child),
    };
    finish(proc, spec, input, limit)
}

fn finish(mut child: Proc, spec: &Spec, input: Option<&[u8]>, limit: std::time::Duration) -> Result<Output, String> {
    if let Some(mut stdin) = child.stdin.take() {
        if let Some(data) = input {
            let _ = stdin.write_all(data);
        }
    }
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let mut err = child.stderr.take().ok_or("no stderr")?;
    let t_out = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        b
    });
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > limit => {
                // A hook it started (a pre-commit, a test runner) goes with it.
                stop_tree(&mut child, std::time::Duration::ZERO);
                return Err(format!("{} didn't finish in {}s", spec.program, limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(15)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let stdout = String::from_utf8_lossy(&t_out.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&t_err.join().unwrap_or_default()).into_owned();
    Ok(Output { code: status.code(), stdout, stderr })
}

#[cfg(all(test, unix))]
mod stop_tests {
    use super::*;

    /// What a program started in the background goes when it's stopped.
    #[test]
    fn stopping_ends_what_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let script = format!("sleep 60 & echo $! > '{}'; wait", pidfile.display());
        let spec = Spec {
            location: &Location::Local,
            cwd: dir.path().to_str().unwrap(),
            program: "sh",
            args: vec!["-c".into(), script],
            env: &[],
        };
        let mut child = spawn(&spec).unwrap();
        let mut pid = String::new();
        for _ in 0..100 {
            pid = std::fs::read_to_string(&pidfile).unwrap_or_default().trim().to_string();
            if !pid.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let pid: i32 = pid.parse().expect("the background pid");
        stop_tree(&mut child, std::time::Duration::ZERO);
        std::thread::sleep(std::time::Duration::from_millis(100));
        // SAFETY: signal 0 only checks that the process exists.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        assert!(!alive, "the background sleep was left running");
    }
}
