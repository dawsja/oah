//! Working on another machine: a WSL distro (Windows) or an SSH host. The
//! engine is the oah CLI installed there, run as `oah serve` through
//! `wsl.exe --exec` or `ssh`; its JSON lines travel over that pipe exactly as
//! they do from a local engine. Sessions, settings and sign-ins live on that
//! machine, in its own oah data folder.
//!
//! Everything here runs small POSIX `sh` scripts on the target with arguments
//! passed separately ($1, $2…), never spliced into the script.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Where the engine runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Target {
    Local,
    Wsl { distro: String },
    Ssh { host: String },
}

impl Target {
    pub fn is_local(&self) -> bool {
        matches!(self, Target::Local)
    }

    /// For messages: "WSL (Ubuntu)", "devbox".
    pub fn label(&self) -> String {
        match self {
            Target::Local => "this computer".into(),
            Target::Wsl { distro } => format!("WSL ({distro})"),
            Target::Ssh { host } => host.clone(),
        }
    }

    /// Reject names that ssh or wsl.exe would read as options, or that hold
    /// whitespace or control characters.
    pub fn validate(&self) -> Result<(), String> {
        let (what, name) = match self {
            Target::Local => return Ok(()),
            Target::Wsl { distro } => ("WSL distro", distro),
            Target::Ssh { host } => ("SSH host", host),
        };
        if name.is_empty() || name.starts_with('-') || name.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!("Not a valid {what}: {name:?}"));
        }
        Ok(())
    }
}

/// The minimum oah CLI version with `oah serve` and the session summaries the
/// sidebar reads.
pub const MIN_VERSION: &str = "0.1.2";

/// POSIX shell single-quoting.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Sets PATH to what an interactive login shell has (so the agent's tools are
/// found; `setsid` keeps that shell from stopping to wait for a terminal), then
/// finds oah: `$OAH_INSTALL`/oah home `bin/oah` (where the installer puts it),
/// else `oah` on PATH. Leaves `$OAH` empty when it isn't installed and `$H`
/// set to the oah data folder.
const PRELUDE: &str = r#"
T=; command -v timeout >/dev/null 2>&1 && T="timeout -s KILL 6"
command -v setsid >/dev/null 2>&1 && T="$T setsid"
P=$($T "${SHELL:-/bin/sh}" -ilc 'printf "\n__OAH_PATH__%s\n" "$PATH"' </dev/null 2>/dev/null | sed -n 's/^__OAH_PATH__//p' | tail -n 1)
[ -n "$P" ] && PATH=$P && export PATH
H=${OAH_HOME:-${XDG_CONFIG_HOME:-$HOME/.config}/oah}
OAH=${OAH_INSTALL:-$H}/bin/oah
[ -x "$OAH" ] || OAH=$(command -v oah 2>/dev/null) || OAH=
"#;

/// `$1` folder (empty: home), then the engine's extra arguments.
pub const ENGINE: &str = r#"
[ -n "$OAH" ] || { echo "oah isn't installed on this machine" >&2; exit 127; }
D=${1:-$HOME}; shift
cd "$D" 2>/dev/null || { echo "No such folder: $D" >&2; exit 2; }
exec "$OAH" serve --cwd "$PWD" "$@"
"#;

/// Prints the home folder, then the oah version (empty line when not installed).
pub const CHECK: &str = r#"
printf '%s\n' "$HOME"
if [ -n "$OAH" ]; then "$OAH" --version 2>/dev/null | head -n 1; else echo; fi
"#;

/// `$1` folder (empty: home). Prints the folder, then its subfolders.
pub const LIST_DIR: &str = r#"
cd "${1:-$HOME}" 2>/dev/null || { echo "No such folder: $1" >&2; exit 2; }
pwd
for d in *; do [ -d "$d" ] && printf '%s\n' "$d"; done
exit 0
"#;

/// Prints the home folder, then the newest session summaries.
pub const SESSIONS: &str = r#"
printf '%s\n' "$HOME"
cd "$H/sessions/meta" 2>/dev/null || exit 0
ls -t 2>/dev/null | grep '\.json$' | head -n 400 | while IFS= read -r f; do cat "$f"; echo; done
"#;

/// `$1` session id (checked by the caller).
pub const DELETE_SESSION: &str = r#"
rm -f "$H/sessions/$1.json" "$H/sessions/meta/$1.json"
"#;

/// Runs the oah installer.
pub const INSTALL: &str = r#"
if command -v curl >/dev/null 2>&1; then curl -fsSL https://oah.dev/install.sh | sh
elif command -v wget >/dev/null 2>&1; then wget -qO- https://oah.dev/install.sh | sh
else echo "curl or wget is needed to install oah" >&2; exit 1; fi
"#;

/// A command that runs `script` (after [`PRELUDE`]) with `sh` on the target,
/// with `args` as `$1`, `$2`…
pub fn command(target: &Target, script: &str, args: &[&str]) -> Command {
    let full = format!("{PRELUDE}{script}");
    #[allow(unused_mut)] // set on Windows below
    let mut cmd = match target {
        Target::Local => {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&full).arg("oah").args(args);
            c
        }
        Target::Wsl { distro } => {
            let mut c = Command::new("wsl.exe");
            c.args(["-d", distro, "--cd", "~", "--exec", "sh", "-c"]).arg(&full).arg("oah").args(args);
            c
        }
        Target::Ssh { host } => {
            // ssh hands the remote shell one string, so quote every word.
            let mut remote = format!("sh -c {} oah", sh_quote(&full));
            for a in args {
                remote.push(' ');
                remote.push_str(&sh_quote(a));
            }
            let mut c = Command::new("ssh");
            c.args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=4"])
                .arg(host)
                .arg(remote);
            c
        }
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Run a script on the target and return its stdout. Fails with the end of
/// stderr when it exits non-zero, or when it takes longer than `timeout`.
pub fn run(target: &Target, script: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    target.validate()?;
    let mut child = command(target, script, args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match target {
            Target::Wsl { .. } => format!("Couldn't run wsl.exe: {e}"),
            Target::Ssh { .. } => format!("Couldn't run ssh: {e}. Install the OpenSSH client."),
            Target::Local => format!("Couldn't run sh: {e}"),
        })?;
    let mut out = child.stdout.take().expect("piped stdout");
    let mut err = child.stderr.take().expect("piped stderr");
    let reader = thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        s
    });
    let err_reader = thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err.read_to_end(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} didn't answer within {}s", target.label(), timeout.as_secs()));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let stdout = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    if status.success() {
        Ok(stdout)
    } else {
        let tail: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = tail[tail.len().saturating_sub(6)..].join("\n");
        Err(if tail.is_empty() { format!("{} exited with {status}", target.label()) } else { tail })
    }
}

/// What's on the target.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub home: String,
    /// The installed oah version, if any.
    pub version: Option<String>,
    /// Installed and new enough for the desktop app.
    pub ok: bool,
    pub min_version: &'static str,
}

pub fn parse_check(out: &str) -> Check {
    let mut lines = out.lines().map(str::trim).filter(|l| !l.starts_with("__OAH_PATH__"));
    let home = lines.next().unwrap_or_default().to_string();
    // `oah --version` prints "oah 0.1.2" (or just the number).
    let version = lines.next().and_then(|l| l.split_whitespace().last()).map(|v| v.trim_start_matches('v').to_string()).filter(|v| !v.is_empty());
    let ok = version.as_deref().is_some_and(|v| version_at_least(v, MIN_VERSION));
    Check { home, version, ok, min_version: MIN_VERSION }
}

/// `a >= b` for dotted numeric versions (pre-release suffixes ignored).
pub fn version_at_least(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let (a, b) = (parse(a), parse(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    true
}

/// A folder and its subfolders.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Listing {
    pub path: String,
    pub dirs: Vec<String>,
}

pub fn parse_listing(out: &str) -> Listing {
    let mut lines = out.lines().filter(|l| !l.is_empty());
    let path = lines.next().unwrap_or("/").to_string();
    let mut dirs: Vec<String> = lines.map(String::from).collect();
    dirs.sort_by_key(|d| d.to_lowercase());
    Listing { path, dirs }
}

/// A session id as the CLI makes them (hex), safe to put in a file path.
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `~/…` for paths under `home`.
pub fn short_with_home(p: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    if home.is_empty() {
        return p.to_string();
    }
    if p == home {
        return "~".into();
    }
    match p.strip_prefix(home) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => p.to_string(),
    }
}

/// WSL distributions installed on this Windows machine (none elsewhere).
pub fn wsl_distros() -> Vec<String> {
    if !cfg!(windows) {
        return vec![];
    }
    let mut cmd = Command::new("wsl.exe");
    cmd.args(["-l", "-q"]).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let Ok(out) = cmd.output() else { return vec![] };
    parse_wsl_list(&out.stdout)
}

/// `wsl.exe -l -q` writes UTF-16LE.
pub fn parse_wsl_list(bytes: &[u8]) -> Vec<String> {
    let text = if bytes.len() >= 2 && bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count() * 2 >= bytes.len() / 2 {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0').to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("docker-desktop"))
        .collect()
}

/// `Host` names from an OpenSSH client config (patterns skipped).
pub fn ssh_config_hosts(config: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let mut words = line.split_whitespace();
        let Some(key) = words.next() else { continue };
        if !key.eq_ignore_ascii_case("host") {
            continue;
        }
        for h in words {
            if h.starts_with('#') {
                break;
            }
            if !h.contains(['*', '?', '!']) && !hosts.iter().any(|x| x == h) {
                hosts.push(h.to_string());
            }
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_a_shell() {
        let tricky = "it's a \"folder\" $HOME `x` \\ ;|&";
        let out = Command::new("sh").arg("-c").arg(format!("printf %s {}", sh_quote(tricky))).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), tricky);
    }

    #[test]
    fn targets_are_validated() {
        assert!(Target::Ssh { host: "dev@box".into() }.validate().is_ok());
        assert!(Target::Ssh { host: "-oProxyCommand=x".into() }.validate().is_err());
        assert!(Target::Ssh { host: "a b".into() }.validate().is_err());
        assert!(Target::Wsl { distro: "Ubuntu-24.04".into() }.validate().is_ok());
        let t: Target = serde_json::from_str(r#"{"kind":"ssh","host":"devbox"}"#).unwrap();
        assert_eq!(t, Target::Ssh { host: "devbox".into() });
    }

    #[test]
    fn versions_and_checks() {
        assert!(version_at_least("0.1.2", "0.1.2"));
        assert!(version_at_least("0.2.0", "0.1.9"));
        assert!(!version_at_least("0.1.1", "0.1.2"));
        assert!(version_at_least("1.0.0-beta", "0.9"));
        let c = parse_check("/home/me\noah 0.1.3\n");
        assert_eq!((c.home.as_str(), c.version.as_deref(), c.ok), ("/home/me", Some("0.1.3"), true));
        let c = parse_check("/home/me\n\n");
        assert_eq!((c.version, c.ok), (None, false));
        assert!(!parse_check("/root\n0.1.1\n").ok);
    }

    #[test]
    fn listings_paths_and_ids() {
        let l = parse_listing("/home/me\nsrc\nDocs\napp\n");
        assert_eq!(l.path, "/home/me");
        assert_eq!(l.dirs, ["app", "Docs", "src"]);
        assert_eq!(short_with_home("/home/me/src/x", "/home/me"), "~/src/x");
        assert_eq!(short_with_home("/home/me", "/home/me/"), "~");
        assert_eq!(short_with_home("/home/meow", "/home/me"), "/home/meow");
        assert!(valid_session_id("a1b2c3d4e5f6"));
        assert!(!valid_session_id("../x"));
        assert!(!valid_session_id(""));
    }

    #[test]
    fn wsl_list_and_ssh_config() {
        let utf16: Vec<u8> = "Ubuntu\r\ndocker-desktop\r\nDebian\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(parse_wsl_list(&utf16), ["Ubuntu", "Debian"]);
        assert_eq!(parse_wsl_list(b"Ubuntu\n"), ["Ubuntu"]);
        let cfg = "Host devbox gpu\n  HostName 10.0.0.2\nHost *.internal\nhost pi # garage\nHost devbox\n";
        assert_eq!(ssh_config_hosts(cfg), ["devbox", "gpu", "pi"]);
    }

    /// What ssh sends is one string for the remote shell: it must come out as
    /// the same script and arguments.
    #[cfg(unix)]
    #[test]
    fn ssh_command_string_round_trips() {
        let cmd = command(&Target::Ssh { host: "devbox".into() }, "printf '%s|' \"$1\" \"$2\"", &["/home/me/it's here", "--resume"]);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args[args.len() - 2], "devbox");
        let remote = args.last().unwrap();
        let out = Command::new("sh").arg("-c").arg(remote).env("SHELL", "/bin/sh").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "/home/me/it's here|--resume|");
    }

    /// End to end against a real target (not run by default):
    /// `OAH_TEST_TARGET='{"kind":"ssh","host":"devbox"}' cargo test -p oah-desktop-core -- --ignored over_a_real_target`
    /// (or `{"kind":"wsl","distro":"Ubuntu"}`). It needs key logins (SSH) and oah >= MIN_VERSION there;
    /// `OAH_TEST_DIR` picks the folder (default: home).
    #[test]
    #[ignore]
    fn over_a_real_target() {
        let t: Target = serde_json::from_str(&std::env::var("OAH_TEST_TARGET").expect("OAH_TEST_TARGET")).unwrap();
        let d = Duration::from_secs(60);
        let check = parse_check(&run(&t, CHECK, &[], d).unwrap());
        assert!(check.ok, "{check:?}");
        let dir = std::env::var("OAH_TEST_DIR").unwrap_or_else(|_| check.home.clone());
        let l = parse_listing(&run(&t, LIST_DIR, &[&dir], d).unwrap());
        assert_eq!(l.path, dir);
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let e = crate::engine::Engine::spawn_remote(&t, &dir, None, move |l| { let _ = tx.send(l); }, |_| {}).unwrap();
        let mut seen = 0;
        let hello = loop {
            let line = rx.recv_timeout(Duration::from_secs(60)).expect("engine event from the target");
            let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("not JSON: {line}"));
            seen += 1;
            if v["t"] == "hello" {
                break v;
            }
        };
        assert_eq!(hello["cwd"], dir.as_str());
        e.send(r#"{"op":"submit","text":"/plan"}"#).unwrap();
        loop {
            let v: serde_json::Value = serde_json::from_str(&rx.recv_timeout(Duration::from_secs(30)).unwrap()).unwrap();
            if v["t"] == "state" && v["s"]["settings"]["mode"] == "plan" {
                break;
            }
        }
        e.shutdown(Duration::from_secs(10));
        let sessions = crate::sessions::from_remote(&run(&t, SESSIONS, &[], d).unwrap());
        eprintln!("{}: {seen} events before hello, {} sessions listed", t.label(), sessions.len());
        if let Some(s) = sessions.first() {
            run(&t, DELETE_SESSION, &[&s.id], d).unwrap();
            assert!(!crate::sessions::from_remote(&run(&t, SESSIONS, &[], d).unwrap()).iter().any(|x| x.id == s.id));
        }
    }

    /// The scripts run under a real `sh`, with a fake oah install.
    #[cfg(unix)]
    #[test]
    fn scripts_run_on_a_target() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("oahhome");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::create_dir_all(home.join("sessions/meta")).unwrap();
        let bin = home.join("bin/oah");
        std::fs::write(&bin, "#!/bin/sh\n[ \"$1\" = --version ] && echo 'oah 0.1.4' && exit 0\necho \"args:$*\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(home.join("sessions/meta/abc.json"), r#"{"id":"abc","title":"t","cwd":"/x","updated":1,"mtime":1,"size":1}"#).unwrap();
        std::fs::write(home.join("sessions/abc.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.path().join("work/sub dir")).unwrap();
        let env = |c: &mut Command| {
            c.env("OAH_HOME", &home).env_remove("OAH_INSTALL").env("SHELL", "/bin/sh");
        };
        let go = |script: &str, args: &[&str]| {
            let mut c = command(&Target::Local, script, args);
            env(&mut c);
            let o = c.output().unwrap();
            (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
        };
        let (ok, out, _) = go(CHECK, &[]);
        assert!(ok);
        assert_eq!(parse_check(&out).version.as_deref(), Some("0.1.4"));
        let work = dir.path().join("work");
        let (ok, out, _) = go(LIST_DIR, &[work.to_str().unwrap()]);
        assert!(ok);
        assert_eq!(parse_listing(&out).dirs, ["sub dir"]);
        let (ok, out, _) = go(ENGINE, &[work.join("sub dir").to_str().unwrap(), "--resume", "abc"]);
        assert!(ok);
        assert!(out.contains("args:serve --cwd ") && out.contains("sub dir --resume abc"), "{out}");
        let (ok, _, err) = go(ENGINE, &["/no/such/folder"]);
        assert!(!ok && err.contains("No such folder"));
        let (ok, out, _) = go(SESSIONS, &[]);
        assert!(ok && out.contains(r#""id":"abc""#));
        let (ok, _, _) = go(DELETE_SESSION, &["abc"]);
        assert!(ok && !home.join("sessions/abc.json").exists() && !home.join("sessions/meta/abc.json").exists());
    }
}
