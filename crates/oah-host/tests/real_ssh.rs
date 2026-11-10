//! Against a real SSH host, with the real `ssh`: run by hand.
//!
//!   OAH_TEST_SSH_HOST=myhost OAH_HOST_DIR=app/src-tauri/host \
//!     cargo test -p oah-host --test real_ssh -- --ignored --test-threads=1
//!
//! OAH_HOST_DIR holds the helper builds (`bun scripts/build-host.ts`). The host gets them in
//! `~/.oah/bin` like any host oah uses.

use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use oah_core::settings::Location;
use oah_transport::Spec;

fn host() -> Location {
    let host = std::env::var("OAH_TEST_SSH_HOST").expect("OAH_TEST_SSH_HOST");
    if std::env::var_os("OAH_HOME").is_none() {
        let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap().keep();
        // SAFETY: set before anything reads it (one test thread).
        unsafe { std::env::set_var("OAH_HOME", dir) };
    }
    Location::Ssh { host }
}

fn sh<'a>(loc: &'a Location, script: &str) -> Spec<'a> {
    Spec { location: loc, cwd: "~", program: "sh", args: vec!["-c".into(), script.into()], env: &[] }
}

#[test]
#[ignore]
fn runs_and_reconnects_over_real_ssh() {
    let loc = host();
    let t = std::time::Instant::now();
    let out = oah_transport::run(&sh(&loc, "echo hi; echo there >&2; exit 4"), None, Duration::from_secs(120)).unwrap();
    assert_eq!((out.stdout.as_str(), out.stderr.as_str(), out.code), ("hi\n", "there\n", Some(4)));
    eprintln!("first command (setup included): {:?}", t.elapsed());
    let t = std::time::Instant::now();
    for _ in 0..10 {
        oah_transport::run(&sh(&loc, "true"), None, Duration::from_secs(30)).unwrap();
    }
    eprintln!("ten more commands on the open connection: {:?}", t.elapsed());

    let big = oah_transport::run(
        &sh(&loc, "head -c 5000000 /dev/urandom | od -An -tx1 | tr -d ' \\n' | head -c 4000000"),
        None,
        Duration::from_secs(120),
    )
    .unwrap();
    assert_eq!(big.stdout.len(), 4_000_000);

    let mut p = oah_transport::spawn(&sh(&loc, "while read l; do echo \"got $l\"; done; echo bye")).unwrap();
    let mut stdin = p.stdin.take().unwrap();
    let mut lines = BufReader::new(p.stdout.take().unwrap()).lines();
    writeln!(stdin, "a").unwrap();
    assert_eq!(lines.next().unwrap().unwrap(), "got a");
    // This side's ssh dies, as when the network goes.
    assert!(kill_my_ssh(), "found the helper's ssh");
    writeln!(stdin, "b").unwrap();
    assert_eq!(lines.next().unwrap().unwrap(), "got b");
    drop(stdin);
    assert_eq!(lines.next().unwrap().unwrap(), "bye");
    assert_eq!(p.wait().unwrap().code(), Some(0));
}

/// Kills this process's own `ssh` children (the helper's connection), not anyone else's.
fn kill_my_ssh() -> bool {
    let me = std::process::id();
    #[cfg(unix)]
    let st = std::process::Command::new("pkill").args(["-P", &me.to_string(), "-x", "ssh"]).status();
    #[cfg(windows)]
    let st = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "$p = Get-CimInstance Win32_Process -Filter \"ParentProcessId={me} AND Name='ssh.exe'\"; if (!$p) {{ exit 1 }}; $p | % {{ taskkill /T /F /PID $_.ProcessId | Out-Null }}"
            ),
        ])
        .status();
    st.map(|s| s.success()).unwrap_or(false)
}
