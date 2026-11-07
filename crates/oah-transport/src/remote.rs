//! What oah needs to know about an SSH host: can we log in, and which tools are there. Plus a
//! folder browser. Each runs one small POSIX `sh` script with arguments passed as `$1`, `$2`…

use std::time::Duration;

use oah_core::settings::Location;
use serde::Serialize;

use crate::{run, Spec};

const CHECK: &str = r#"
printf 'home\t%s\n' "$HOME"
printf 'os\t%s\n' "$(uname -sm 2>/dev/null)"
for t in git gh claude codex; do
  p=$(command -v "$t" 2>/dev/null) || p=
  v=
  [ -n "$p" ] && v=$("$t" --version 2>/dev/null | head -n 1)
  printf '%s\t%s\t%s\n' "$t" "$p" "$v"
done
"#;

const LIST_DIR: &str = r#"
cd "${1:-$HOME}" 2>/dev/null || { echo "No such folder: $1" >&2; exit 2; }
pwd
for d in * .[!.]*; do [ -d "$d" ] && printf '%s\n' "$d"; done
exit 0
"#;

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HostCheck {
    pub ok: bool,
    pub error: Option<String>,
    pub home: Option<String>,
    pub os: Option<String>,
    pub git: Tool,
    pub gh: Tool,
    pub claude: Tool,
    pub codex: Tool,
}

fn sh(host: &str, script: &str, args: &[&str], limit: Duration) -> Result<crate::Output, String> {
    let loc = Location::Ssh { host: host.into() };
    let mut a = vec!["-c".to_string(), script.to_string(), "oah".to_string()];
    a.extend(args.iter().map(|s| s.to_string()));
    run(&Spec { location: &loc, cwd: "~", program: "sh", args: a, env: &[] }, None, limit)
}

/// Logs in and lists the tools oah uses there.
pub fn check(host: &str) -> HostCheck {
    match sh(host, CHECK, &[], Duration::from_secs(25)) {
        Err(e) => HostCheck { error: Some(e), ..Default::default() },
        Ok(out) if !out.ok() => HostCheck { error: Some(ssh_error(&out.stderr)), ..Default::default() },
        Ok(out) => parse_check(&out.stdout),
    }
}

pub fn parse_check(out: &str) -> HostCheck {
    let mut c = HostCheck { ok: true, ..Default::default() };
    for line in out.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let tool = || Tool {
            path: f.get(1).filter(|p| !p.is_empty()).map(|p| p.to_string()),
            version: f.get(2).filter(|v| !v.is_empty()).map(|v| v.to_string()),
        };
        match f[0] {
            "home" => c.home = f.get(1).map(|s| s.to_string()),
            "os" => c.os = f.get(1).filter(|s| !s.is_empty()).map(|s| s.to_string()),
            "git" => c.git = tool(),
            "gh" => c.gh = tool(),
            "claude" => c.claude = tool(),
            "codex" => c.codex = tool(),
            _ => {}
        }
    }
    c
}

/// ssh's own complaint, made short: the last line that says something.
pub fn ssh_error(stderr: &str) -> String {
    let line = stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("ssh failed");
    if line.contains("Permission denied") {
        format!("{line}. oah logs in with keys only: add your key with ssh-copy-id first.")
    } else {
        line.to_string()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub path: String,
    pub dirs: Vec<String>,
}

/// A remote folder and its subfolders (empty `path`: the home folder).
pub fn list_dir(host: &str, path: &str) -> Result<Listing, String> {
    let out = sh(host, LIST_DIR, &[path], Duration::from_secs(20))?;
    if !out.ok() {
        return Err(ssh_error(&out.stderr));
    }
    Ok(parse_listing(&out.stdout))
}

pub fn parse_listing(out: &str) -> Listing {
    let mut lines = out.lines().filter(|l| !l.is_empty());
    let path = lines.next().unwrap_or("/").to_string();
    let mut dirs: Vec<String> = lines.map(String::from).collect();
    dirs.sort_by_key(|d| (d.starts_with('.'), d.to_lowercase()));
    Listing { path, dirs }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_check() {
        let c = parse_check("home\t/home/daws\nos\tLinux x86_64\ngit\t/usr/bin/git\tgit version 2.43.0\ngh\t\t\nclaude\t/home/daws/.local/bin/claude\t2.1.319 (Claude Code)\ncodex\t\t\n");
        assert!(c.ok);
        assert_eq!(c.home.as_deref(), Some("/home/daws"));
        assert_eq!(c.git.version.as_deref(), Some("git version 2.43.0"));
        assert!(c.gh.path.is_none());
        assert!(c.claude.path.is_some());
        let l = parse_listing("/home/daws\n.config\ncode\nApps\n");
        assert_eq!(l.dirs, ["Apps", "code", ".config"]);
    }
}
