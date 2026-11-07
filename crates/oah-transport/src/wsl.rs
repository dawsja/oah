//! WSL distros on this computer: which are installed and running, and how to run a program in
//! one. Every command is `wsl.exe -d <distro> -e <program> <args…>`: `-e` (exec) hands the words
//! to Linux as they are, where `--` would run them through the distro's shell and expand `$`
//! twice. `WSL_UTF8=1` makes wsl.exe print its own messages as UTF-8 instead of UTF-16.

use std::process::{Command, Stdio};
use std::time::Duration;

use oah_core::wsl::valid_distro;
use serde::Serialize;

/// An installed distro.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Distro {
    pub name: String,
    /// `wsl.exe` without `-d` uses this one.
    pub default: bool,
    pub running: bool,
    /// 1 or 2 (0 when unknown).
    pub version: u8,
}

/// `wsl.exe` from System32, so nothing else on PATH can stand in for it.
pub fn exe() -> std::path::PathBuf {
    if let Some(root) = std::env::var_os("SystemRoot") {
        let p = std::path::Path::new(&root).join("System32").join("wsl.exe");
        if p.exists() {
            return p;
        }
    }
    "wsl.exe".into()
}

/// `wsl.exe -d <distro> -e sh -lc <script>`, run from a folder that always exists (a deleted
/// worktree as wsl.exe's own working folder would make the start fail).
pub fn command(distro: &str, script: &str) -> Result<Command, String> {
    if !cfg!(windows) {
        return Err("WSL projects open on Windows only".into());
    }
    if !valid_distro(distro) {
        return Err(format!("not a valid WSL distro name: {distro:?}"));
    }
    let mut c = Command::new(exe());
    c.args(["-d", distro, "-e", "sh", "-lc", script]).env("WSL_UTF8", "1");
    if let Some(root) = std::env::var_os("SystemRoot") {
        c.current_dir(root);
    }
    Ok(c)
}

/// Runs `wsl.exe <args>` (no distro) and returns what it printed, NULs dropped (some builds
/// ignore `WSL_UTF8` and print UTF-16).
fn wsl(args: &[&str], limit: Duration) -> Result<crate::Output, String> {
    if !cfg!(windows) {
        return Err("WSL is a Windows feature".into());
    }
    let mut c = Command::new(exe());
    c.args(args).env("WSL_UTF8", "1").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::no_window(&mut c);
    let child = c.spawn().map_err(|e| format!("couldn't run wsl.exe: {e}"))?;
    let out = wait(child, limit)?;
    Ok(crate::Output { code: out.code, stdout: out.stdout.replace('\0', ""), stderr: out.stderr.replace('\0', "") })
}

fn wait(mut child: std::process::Child, limit: Duration) -> Result<crate::Output, String> {
    use std::io::Read;
    let (mut o, mut e) = (child.stdout.take().ok_or("no stdout")?, child.stderr.take().ok_or("no stderr")?);
    let to = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = o.read_to_end(&mut b);
        b
    });
    let te = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = e.read_to_end(&mut b);
        b
    });
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("wsl.exe didn't answer in {}s", limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => return Err(e.to_string()),
        }
    };
    Ok(crate::Output {
        code: status.code(),
        stdout: String::from_utf8_lossy(&to.join().unwrap_or_default()).into_owned(),
        stderr: String::from_utf8_lossy(&te.join().unwrap_or_default()).into_owned(),
    })
}

/// The installed distros (Docker Desktop's own are left out). Empty when WSL isn't installed.
/// Names come from `--list --quiet` and running ones from `--list --running --quiet` (both
/// language-independent); the default and the version from `--list --verbose`'s `*` and last
/// column.
pub fn distros() -> Result<Vec<Distro>, String> {
    let names = wsl(&["--list", "--quiet"], Duration::from_secs(20))?;
    if !names.ok() {
        // No distros installed, or WSL itself missing: nothing to offer.
        return Ok(Vec::new());
    }
    let running = wsl(&["--list", "--running", "--quiet"], Duration::from_secs(20)).map(|o| o.stdout).unwrap_or_default();
    let verbose = wsl(&["--list", "--verbose"], Duration::from_secs(20)).map(|o| o.stdout).unwrap_or_default();
    Ok(parse_distros(&names.stdout, &running, &verbose))
}

pub fn parse_distros(quiet: &str, running: &str, verbose: &str) -> Vec<Distro> {
    let list = |s: &str| -> Vec<String> {
        s.lines().map(|l| l.trim().trim_start_matches('*').trim().to_string()).filter(|l| valid_distro(l)).collect()
    };
    let running = list(running);
    let mut default = None;
    let mut versions = std::collections::HashMap::new();
    for line in verbose.lines() {
        let line = line.trim();
        let star = line.starts_with('*');
        let mut words = line.trim_start_matches('*').split_whitespace();
        let Some(name) = words.next() else { continue };
        if star {
            default = Some(name.to_string());
        }
        if let Some(v) = line.split_whitespace().last().and_then(|v| v.parse::<u8>().ok()) {
            versions.insert(name.to_string(), v);
        }
    }
    list(quiet)
        .into_iter()
        .filter(|n| !n.to_ascii_lowercase().starts_with("docker-desktop"))
        .map(|name| Distro {
            default: default.as_deref() == Some(name.as_str()),
            running: running.iter().any(|r| r == &name),
            version: versions.get(&name).copied().unwrap_or(0),
            name,
        })
        .collect()
}

/// Whether `distro` is up (without starting it).
pub fn is_running(distro: &str) -> bool {
    wsl(&["--list", "--running", "--quiet"], Duration::from_secs(10))
        .map(|o| o.stdout.lines().any(|l| l.trim().trim_start_matches('*').trim() == distro))
        .unwrap_or(false)
}

/// Starts `distro` if it's stopped (a few seconds the first time) and checks it answers.
pub fn start(distro: &str) -> Result<(), String> {
    if !valid_distro(distro) {
        return Err(format!("not a valid WSL distro name: {distro:?}"));
    }
    let out = wsl(&["-d", distro, "-e", "true"], Duration::from_secs(90))?;
    if out.ok() {
        Ok(())
    } else {
        Err(error_text(distro, &out.stdout, &out.stderr))
    }
}

/// wsl.exe's complaint (on stdout, in the user's language), made short; the common ones in
/// plain words. Error codes like `WSL_E_DISTRO_NOT_FOUND` aren't translated, so they're matched.
pub fn error_text(distro: &str, stdout: &str, stderr: &str) -> String {
    let all = format!("{stdout}\n{stderr}").replace('\0', "");
    if all.contains("WSL_E_DISTRO_NOT_FOUND") || all.contains("no distribution with the supplied name") {
        return format!("There's no WSL distro named {distro} on this computer.");
    }
    if all.contains("WSL_E_WSL_OPTIONAL_COMPONENT_REQUIRED") || all.contains("not installed") && all.contains("Subsystem") {
        return "WSL isn't installed on this computer (run `wsl --install`).".into();
    }
    let line = stderr.lines().chain(stdout.lines()).map(str::trim).rfind(|l| !l.is_empty() && !l.starts_with("Error code:"));
    line.map(String::from).unwrap_or_else(|| format!("{distro} didn't start"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_distros() {
        let quiet = "Ubuntu\r\ndocker-desktop\r\nDebian\r\n";
        let running = "Ubuntu\r\n";
        let verbose = "  NAME              STATE           VERSION\r\n* Ubuntu            Running         2\r\n  docker-desktop    Stopped         2\r\n  Debian            Wird angehalten 1\r\n";
        let d = parse_distros(quiet, running, verbose);
        assert_eq!(
            d,
            vec![
                Distro { name: "Ubuntu".into(), default: true, running: true, version: 2 },
                Distro { name: "Debian".into(), default: false, running: false, version: 1 },
            ]
        );
        assert!(parse_distros("", "", "").is_empty());
    }

    #[test]
    fn explains_failures() {
        let nf = "There is no distribution with the supplied name.\r\nError code: Wsl/Service/WSL_E_DISTRO_NOT_FOUND\r\n";
        assert_eq!(error_text("Arch", nf, ""), "There's no WSL distro named Arch on this computer.");
        assert_eq!(error_text("Ubuntu", "", "sh: 1: cd: can't cd to /nope\n"), "sh: 1: cd: can't cd to /nope");
    }

    #[test]
    fn refuses_bad_names() {
        assert!(command("-d", "true").is_err());
    }

    /// Against a real distro: `OAH_TEST_DISTRO=Ubuntu cargo test -p oah-transport -- --ignored`.
    #[test]
    #[ignore]
    fn real_distro() {
        use oah_core::settings::Location;
        let distro = std::env::var("OAH_TEST_DISTRO").unwrap_or_else(|_| "Ubuntu".into());
        assert!(distros().unwrap().iter().any(|d| d.name == distro), "{distro} is installed");
        start(&distro).unwrap();
        assert!(is_running(&distro));
        let loc = Location::Wsl { distro: distro.clone() };
        // Quoting survives wsl.exe: spaces, quotes, $ and a missing program.
        let spec = crate::Spec {
            location: &loc,
            cwd: "/",
            program: "printf",
            args: vec!["%s|".into(), "a b".into(), "it's $HOME".into()],
            env: &[],
        };
        let out = crate::run(&spec, None, Duration::from_secs(30)).unwrap();
        assert_eq!(out.stdout, "a b|it's $HOME|");
        let spec = crate::Spec { location: &loc, cwd: "~", program: "oah-no-such-tool", args: vec![], env: &[] };
        let out = crate::run(&spec, None, Duration::from_secs(30)).unwrap();
        assert_eq!(out.code, Some(127));
        assert!(out.stderr.contains("isn't installed in this distro"), "{}", out.stderr);
        // Environment reaches the program.
        let env = [oah_core::settings::EnvVar { key: "OAH_X".into(), value: "a'b c".into(), in_keychain: false }];
        let spec = crate::Spec {
            location: &loc,
            cwd: "/tmp",
            program: "sh",
            args: vec!["-c".into(), "printf %s \"$OAH_X\"; pwd".into()],
            env: &env,
        };
        assert_eq!(crate::run(&spec, None, Duration::from_secs(30)).unwrap().stdout, "a'b c/tmp\n");
        // The host check and the folder browser.
        let c = crate::remote::check(&loc);
        assert!(c.ok, "{:?}", c.error);
        println!("{c:#?}");
        let root = crate::remote::list_dir(&loc, "/").unwrap();
        assert_eq!(root.path, "/");
        for d in ["etc", "home", "usr"] {
            assert!(root.dirs.iter().any(|x| x == d), "{d} in {:?}", root.dirs);
        }
        let home = crate::remote::list_dir(&loc, "").unwrap();
        assert!(home.path.starts_with("/home/") || home.path == "/root", "{}", home.path);
        assert!(crate::remote::list_dir(&loc, "/nope-oah").is_err());
        let missing = Location::Wsl { distro: "NoSuchDistroOah".into() };
        let e = crate::remote::check(&missing).error.unwrap();
        assert!(e.contains("no WSL distro named NoSuchDistroOah"), "{e}");
    }
}
