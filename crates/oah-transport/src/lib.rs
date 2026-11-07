//! Running a program in a project's folder, wherever the project is: on this computer, or on an
//! SSH host through `ssh -T` (the program's stdio is the ssh session's). Remote commands are one
//! POSIX `sh` line with every word single-quoted; nothing from the user is spliced in unquoted.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use oah_core::settings::{EnvVar, Location};
pub mod remote;

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

/// A `Command` ready to spawn, with piped stdio and no console window. Local programs are
/// resolved on PATH first (so `claude` finds `claude.exe` or `claude.cmd`).
pub fn command(spec: &Spec) -> Result<Command, String> {
    let mut cmd = match spec.location {
        Location::Local => {
            let exe: PathBuf = which::which(spec.program).map_err(|_| format!("{} isn't installed or isn't on PATH", spec.program))?;
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
            c.args(["-T", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=30", "--", host]).arg(remote_line(spec));
            c
        }
    };
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    no_window(&mut cmd);
    Ok(cmd)
}

/// The remote shell line: a login shell's PATH (so tools installed for the user are found),
/// `cd` to the folder (`~` expanded), then `exec env K=V program args…`.
pub fn remote_line(spec: &Spec) -> String {
    let mut words = vec!["exec".to_string(), "env".to_string()];
    for v in spec.env.iter().filter(|v| valid_env_key(&v.key)) {
        words.push(sh_quote(&format!("{}={}", v.key, v.value)));
    }
    words.push(sh_quote(spec.program));
    words.extend(spec.args.iter().map(|a| sh_quote(a)));
    let cd = match spec.cwd.strip_prefix("~/") {
        Some(rest) => format!("cd \"$HOME\"/{}", sh_quote(rest)),
        None if spec.cwd == "~" || spec.cwd.is_empty() => "cd \"$HOME\"".into(),
        None => format!("cd {}", sh_quote(spec.cwd)),
    };
    // `sh -lc` reads the login profile; `bash -lc` would too, but sh is always there.
    let inner = format!("{cd} && {}", words.join(" "));
    format!("sh -lc {}", sh_quote(&inner))
}

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
        let env = [EnvVar { key: "API_KEY".into(), value: "a'b".into() }, EnvVar { key: "bad key".into(), value: "x".into() }];
        let spec = Spec { location: &loc, cwd: "~/code/my app", program: "claude", args: vec!["-p".into(), "it's".into()], env: &env };
        let line = remote_line(&spec);
        assert_eq!(
            line,
            r#"sh -lc 'cd "$HOME"/'\''code/my app'\'' && exec env '\''API_KEY=a'\''\'\'''\''b'\'' '\''claude'\'' '\''-p'\'' '\''it'\''\'\'''\''s'\'''"#
        );
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
    use std::io::{Read, Write};
    let mut child = command(spec)?.spawn().map_err(|e| format!("couldn't run {}: {e}", spec.program))?;
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
                let _ = child.kill();
                let _ = child.wait();
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
