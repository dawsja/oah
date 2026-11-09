//! What oah needs to know about a remote location (an SSH host or a WSL distro): can we reach
//! it, and which tools are there. Plus a folder browser. Each runs one small POSIX `sh` script
//! with arguments passed as `$1`, `$2`…

use std::time::Duration;

use oah_core::settings::Location;
use serde::Serialize;

use crate::{run, Spec};

const CHECK: &str = r#"
printf 'home\t%s\n' "$HOME"
printf 'os\t%s\n' "$(uname -sm 2>/dev/null)"
set -f; lin=; oifs=$IFS; IFS=:; for d in $PATH; do case $d in /mnt/[a-z]|/mnt/[a-z]/*) ;; *) lin=${lin:+$lin:}$d;; esac; done; IFS=$oifs; set +f
for t in git gh claude codex opencode opencode2 agy; do
  p=$(PATH="$lin:$HOME/.local/bin:$HOME/.opencode/bin" command -v "$t" 2>/dev/null) || p=
  v=
  [ -n "$p" ] && v=$("$p" --version 2>/dev/null | head -n 1)
  w=
  [ -z "$p" ] && w=$(command -v "$t" 2>/dev/null)
  printf '%s\t%s\t%s\t%s\n' "$t" "$p" "$v" "$w"
done
"#;

/// `$1` on the location's PATH, leaving out WSL's Windows folders (a Windows copy can't run there
/// as a Linux agent): prints its path, or a marker when it isn't there (an exit code could be
/// ssh's or wsl.exe's own).
const INSTALLED: &str = r#"
set -f; lin=; oifs=$IFS; IFS=:; for d in $PATH; do case $d in /mnt/[a-z]|/mnt/[a-z]/*) ;; *) lin=${lin:+$lin:}$d;; esac; done; IFS=$oifs; set +f
PATH="$lin:$HOME/.local/bin:$HOME/.opencode/bin" command -v "$1" || echo "oah:missing"
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
    /// Not installed there, but a Windows copy is on PATH (WSL): where.
    pub windows: Option<String>,
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
    pub opencode: Tool,
    pub opencode2: Tool,
    /// Antigravity's `agy`.
    pub antigravity: Tool,
}

fn sh(location: &Location, script: &str, args: &[&str], limit: Duration) -> Result<crate::Output, String> {
    let mut a = vec!["-c".to_string(), script.to_string(), "oah".to_string()];
    a.extend(args.iter().map(|s| s.to_string()));
    run(&Spec { location, cwd: "~", program: "sh", args: a, env: &[] }, None, limit)
}

/// Why a command on `location` failed: ssh's complaint, or wsl.exe's (which it prints on stdout).
pub fn failure(location: &Location, out: &crate::Output) -> String {
    match location {
        Location::Wsl { distro } => crate::wsl::error_text(distro, &out.stdout, &out.stderr),
        _ => ssh_error(&out.stderr),
    }
}

/// Reaches `location` (logs in, or starts the distro) and lists the tools oah uses there.
pub fn check(location: &Location) -> HostCheck {
    // A stopped distro takes a few seconds to start.
    match sh(location, CHECK, &[], Duration::from_secs(60)) {
        Err(e) => HostCheck { error: Some(e), ..Default::default() },
        Ok(out) if !out.ok() => HostCheck { error: Some(failure(location, &out)), ..Default::default() },
        Ok(out) => parse_check(&out.stdout),
    }
}

/// Whether `program` is installed on an SSH host or in a WSL distro; `None` when the location
/// couldn't be asked (unreachable, or the lookup itself failed).
pub fn installed(location: &Location, program: &str) -> Option<bool> {
    let out = sh(location, INSTALLED, &[program], Duration::from_secs(20)).ok()?;
    parse_installed(&out)
}

fn parse_installed(out: &crate::Output) -> Option<bool> {
    let last = out.stdout.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    match (out.ok(), last) {
        (true, "oah:missing") => Some(false),
        (true, _) => Some(true),
        _ => None,
    }
}

pub fn parse_check(out: &str) -> HostCheck {
    let mut c = HostCheck { ok: true, ..Default::default() };
    for line in out.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let field = |i: usize| f.get(i).filter(|p| !p.is_empty()).map(|p| p.to_string());
        let tool = || Tool { path: field(1), version: field(2), windows: field(3) };
        match f[0] {
            "home" => c.home = f.get(1).map(|s| s.to_string()),
            "os" => c.os = f.get(1).filter(|s| !s.is_empty()).map(|s| s.to_string()),
            "git" => c.git = tool(),
            "gh" => c.gh = tool(),
            "claude" => c.claude = tool(),
            "codex" => c.codex = tool(),
            "opencode" => c.opencode = tool(),
            "opencode2" => c.opencode2 = tool(),
            "agy" => c.antigravity = tool(),
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
pub fn list_dir(location: &Location, path: &str) -> Result<Listing, String> {
    let out = sh(location, LIST_DIR, &[path], Duration::from_secs(60))?;
    if !out.ok() {
        return Err(failure(location, &out));
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
    fn reads_whether_a_program_is_installed() {
        let out = |code, stdout: &str| crate::Output { code, stdout: stdout.into(), stderr: String::new() };
        assert_eq!(parse_installed(&out(Some(0), "/home/u/.local/bin/codex\n")), Some(true));
        assert_eq!(parse_installed(&out(Some(0), "oah:missing\n")), Some(false));
        // ssh couldn't log in, or the distro didn't start: unknown, not "missing".
        assert_eq!(parse_installed(&out(Some(255), "")), None);
        assert_eq!(parse_installed(&out(Some(1), "oah:missing\n")), None);
        assert_eq!(parse_installed(&out(Some(0), "")), None);
    }

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
