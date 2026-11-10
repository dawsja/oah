//! oah-transport's SSH path with `oah-host` in it, end to end, on this machine: a stand-in `ssh`
//! runs the remote command here with its own home folder, so setting the host up (the check, the
//! copy), the daemon, dropped connections and stopping are all the real thing.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use oah_core::settings::Location;
use oah_transport::Spec;

struct World {
    _dir: tempfile::TempDir,
    remote_home: PathBuf,
}

/// One per test binary: PATH and OAH_HOST_DIR are process-wide.
fn world() -> &'static World {
    static W: OnceLock<World> = OnceLock::new();
    W.get_or_init(|| {
        let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let remote_home = dir.path().join("remote-home");
        let bin = dir.path().join("bin");
        let helpers = dir.path().join("helpers");
        for d in [&remote_home, &bin, &helpers] {
            std::fs::create_dir_all(d).unwrap();
        }
        // What sshd does with `ssh [opts] -- host line`: the line, through a shell, at home.
        let ssh = format!(
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in --) shift; break;; -o) shift 2;; *) shift;; esac; done\nshift\nexport HOME='{}'\ncd \"$HOME\"\nexec sh -c \"$*\"\n",
            remote_home.display()
        );
        let ssh_path = bin.join("ssh");
        std::fs::write(&ssh_path, ssh).unwrap();
        std::fs::set_permissions(&ssh_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        for plat in ["x86_64-linux", "aarch64-linux"] {
            std::fs::copy(env!("CARGO_BIN_EXE_oah-host"), helpers.join(format!("oah-host-{plat}"))).unwrap();
        }
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        // SAFETY: set once, before this binary's tests start anything that reads them.
        unsafe {
            std::env::set_var("PATH", path);
            std::env::set_var("OAH_HOST_DIR", &helpers);
            std::env::set_var("OAH_HOME", dir.path().join("oah"));
        }
        // The daemon outlives this test binary; end it (and what it runs) on the way out.
        extern "C" fn stop_daemons() {
            if let Some(w) = W.get() {
                let _ = std::process::Command::new("pkill").args(["-KILL", "-f", &format!("{}/.oah/bin/oah-host-", w.remote_home.display())]).status();
            }
        }
        // SAFETY: registering a plain extern "C" function.
        unsafe { libc::atexit(stop_daemons) };
        World { _dir: dir, remote_home }
    })
}

fn loc() -> Location {
    world();
    Location::Ssh { host: "testhost".into() }
}

fn sh<'a>(loc: &'a Location, script: &str) -> Spec<'a> {
    Spec { location: loc, cwd: "~", program: "sh", args: vec!["-c".into(), script.into()], env: &[] }
}

fn helper_name(home: &Path) -> String {
    let names: Vec<String> = std::fs::read_dir(home.join(".oah/bin"))
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.into_iter().find(|n| n.starts_with("oah-host-")).expect("the helper was copied to the host")
}

/// Drops the SSH connection the way a network does: the proxy on the host just dies. Tests
/// that do this share one connection, so they take turns.
fn drop_connection(home: &Path) -> std::sync::MutexGuard<'static, ()> {
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let pattern = format!("{} proxy", helper_name(home));
    let killed =
        wait_until(Duration::from_secs(10), || std::process::Command::new("pkill").args(["-f", &pattern]).status().unwrap().success());
    assert!(killed, "a proxy was running");
    turn
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + limit;
    while Instant::now() < until {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f()
}

#[test]
fn commands_run_through_the_helper() {
    let loc = loc();
    let out = oah_transport::run(&sh(&loc, "cat; echo oops >&2; exit 3"), Some(b"hello\nworld\n"), Duration::from_secs(30)).unwrap();
    assert_eq!(out.stdout, "hello\nworld\n");
    assert_eq!(out.stderr, "oops\n");
    assert_eq!(out.code, Some(3));
    // It went through the daemon, not a plain ssh.
    assert!(helper_name(&world().remote_home).len() > "oah-host-".len());
    let run_dir = world().remote_home.join(".oah/run");
    let machine = std::fs::read_dir(&run_dir).unwrap().next().expect("a folder for this machine").unwrap().path();
    assert!(std::fs::read_dir(&machine).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".sock")));

    // Binary data and big outputs arrive whole.
    let out = oah_transport::run(&sh(&loc, "head -c 3000000 /dev/zero | tr '\\0' 'x'"), None, Duration::from_secs(30)).unwrap();
    assert_eq!(out.stdout.len(), 3_000_000);
    assert!(out.stdout.bytes().all(|b| b == b'x'));

    // In the folder it was asked for, with what it was given.
    let env = [oah_core::settings::EnvVar { key: "OAH_T".into(), value: "it's".into(), in_keychain: false }];
    let spec = Spec { location: &loc, cwd: "~/.oah", program: "sh", args: vec!["-c".into(), "pwd; echo \"$OAH_T\"".into()], env: &env };
    let out = oah_transport::run(&spec, None, Duration::from_secs(30)).unwrap();
    assert_eq!(out.stdout, format!("{}\nit's\n", world().remote_home.join(".oah").display()));
}

#[test]
fn a_program_lives_through_a_dropped_connection() {
    let loc = loc();
    let mut p = oah_transport::spawn(&sh(&loc, "while read l; do echo \"got $l\"; done; echo bye")).unwrap();
    let mut stdin = p.stdin.take().unwrap();
    let mut lines = BufReader::new(p.stdout.take().unwrap()).lines();
    writeln!(stdin, "a").unwrap();
    assert_eq!(lines.next().unwrap().unwrap(), "got a");

    let _turn = drop_connection(&world().remote_home);
    // Written while the connection is down: it arrives once it's back, once.
    writeln!(stdin, "b").unwrap();
    writeln!(stdin, "c").unwrap();
    assert_eq!(lines.next().unwrap().unwrap(), "got b");
    assert_eq!(lines.next().unwrap().unwrap(), "got c");

    drop(stdin);
    assert_eq!(lines.next().unwrap().unwrap(), "bye");
    assert!(lines.next().is_none());
    assert_eq!(p.wait().unwrap().code(), Some(0));
}

#[test]
fn output_while_disconnected_is_kept() {
    let loc = loc();
    let mut p = oah_transport::spawn(&sh(&loc, "echo first; sleep 1; echo second; exit 7")).unwrap();
    let mut lines = BufReader::new(p.stdout.take().unwrap()).lines();
    assert_eq!(lines.next().unwrap().unwrap(), "first");
    let _turn = drop_connection(&world().remote_home);
    assert_eq!(lines.next().unwrap().unwrap(), "second");
    assert!(lines.next().is_none());
    assert_eq!(p.wait().unwrap().code(), Some(7));
}

#[test]
fn stopping_ends_its_group_on_the_host() {
    let loc = loc();
    let mut p = oah_transport::spawn(&sh(&loc, "sleep 60 & echo $!; wait")).unwrap();
    let mut lines = BufReader::new(p.stdout.take().unwrap()).lines();
    let pid: i32 = lines.next().unwrap().unwrap().trim().parse().unwrap();
    assert!(alive(pid));
    drop(p.stdin.take());
    oah_transport::stop_tree(&mut p, Duration::from_millis(100));
    assert!(wait_until(Duration::from_secs(5), || !alive(pid)), "the background sleep was left running");
}

#[test]
fn letting_go_ends_it() {
    let loc = loc();
    let mut p = oah_transport::spawn(&sh(&loc, "echo $$; exec sleep 60")).unwrap();
    let mut lines = BufReader::new(p.stdout.take().unwrap()).lines();
    let pid: i32 = lines.next().unwrap().unwrap().trim().parse().unwrap();
    drop(lines);
    drop(p);
    assert!(wait_until(Duration::from_secs(6), || !alive(pid)), "a program nobody holds was left running");
}

#[test]
fn big_input_waits_for_the_host_instead_of_failing() {
    let loc = loc();
    let input = vec![b'y'; 40 << 20];
    let out = oah_transport::run(&sh(&loc, "wc -c"), Some(&input), Duration::from_secs(120)).unwrap();
    assert_eq!(out.stdout.trim(), (40u64 << 20).to_string());
}

#[test]
fn many_at_once_share_the_connection() {
    let loc = loc();
    let handles: Vec<_> = (0..12)
        .map(|i| {
            let loc = loc.clone();
            std::thread::spawn(move || oah_transport::run(&sh(&loc, &format!("echo {i}")), None, Duration::from_secs(30)).unwrap().stdout)
        })
        .collect();
    for (i, h) in handles.into_iter().enumerate() {
        assert_eq!(h.join().unwrap(), format!("{i}\n"));
    }
}
