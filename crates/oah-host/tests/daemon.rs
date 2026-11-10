//! The daemon through `oah-host proxy`, speaking the protocol by hand.

#![cfg(unix)]

use std::io::Write;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use oah_host::proto::{self, Msg, PROTO, STDOUT};

struct Conn {
    child: Child,
    w: ChildStdin,
    r: ChildStdout,
}

impl Conn {
    fn open(home: &std::path::Path, client: u64, instance: u64) -> Conn {
        let mut child = Command::new(env!("CARGO_BIN_EXE_oah-host"))
            .args(["proxy", "testtag"])
            .env("HOME", home)
            .env_remove("SSH_AUTH_SOCK")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let (w, mut r) = (child.stdin.take().unwrap(), child.stdout.take().unwrap());
        proto::skip_to_magic(&mut r, 1024).unwrap();
        let mut c = Conn { child, w, r };
        c.send(Msg::Hello { proto: PROTO, client, instance });
        assert!(matches!(c.next(), Msg::HelloOk { proto: PROTO, .. }));
        c
    }

    fn send(&mut self, m: Msg) {
        self.w.write_all(&m.encode()).unwrap();
    }

    fn next(&mut self) -> Msg {
        proto::read(&mut self.r).unwrap().expect("a message")
    }

    /// Output on stdout until `want` bytes, answering nothing else.
    fn stdout_until(&mut self, id: u64, want: usize) -> Vec<u8> {
        let mut got = Vec::new();
        while got.len() < want {
            if let Msg::Out { id: i, stream: STDOUT, data, .. } = self.next() {
                assert_eq!(i, id);
                got.extend(data);
            }
        }
        got
    }

    fn close(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Ends the daemon a test started (its pid is in its log), so test runs don't leave them.
fn stop_daemon(home: &std::path::Path) {
    for machine in std::fs::read_dir(home.join(".oah/run")).into_iter().flatten().flatten() {
        let log = std::fs::read_to_string(machine.path().join("testtag.log")).unwrap_or_default();
        if let Some(pid) = log.split("(pid ").nth(1).and_then(|r| r.split(')').next()).and_then(|p| p.parse::<i32>().ok()) {
            // SAFETY: a plain signal to the daemon this test started.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn reattaching_replays_from_where_oah_was_and_a_new_run_ends_the_old_ones_programs() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();

    let mut a = Conn::open(dir.path(), 7, 1);
    a.send(Msg::Spawn { id: 42, script: "echo $$; echo one; sleep 0.3; echo two; exec sleep 60".into() });
    assert_eq!(a.next(), Msg::Spawned { id: 42 });
    let first = String::from_utf8(a.stdout_until(42, 1)).unwrap();
    let pid: i32 = first.lines().next().unwrap().parse().unwrap();
    // Gone before oah acknowledged anything.
    a.close();
    std::thread::sleep(Duration::from_millis(600));

    // The same run comes back: everything from the start, nothing lost.
    let mut b = Conn::open(dir.path(), 7, 1);
    b.send(Msg::Attach { id: 42, out: 0, err: 0 });
    assert_eq!(b.next(), Msg::StdinAck { id: 42, off: 0 });
    let all = String::from_utf8(b.stdout_until(42, format!("{pid}\none\ntwo\n").len())).unwrap();
    assert_eq!(all, format!("{pid}\none\ntwo\n"));
    // From an offset: only what comes after it.
    b.send(Msg::Attach { id: 42, out: all.len() as u64 - 4, err: 0 });
    assert_eq!(b.next(), Msg::StdinAck { id: 42, off: 0 });
    assert_eq!(b.stdout_until(42, 4), b"two\n");
    // An id the host never had.
    b.send(Msg::Attach { id: 99, out: 0, err: 0 });
    assert_eq!(b.next(), Msg::Gone { id: 99 });
    b.close();

    // A new run of the same install can't come back for it, so it's ended.
    assert!(alive(pid));
    let c = Conn::open(dir.path(), 7, 2);
    let until = Instant::now() + Duration::from_secs(6);
    while alive(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(pid), "the old run's program was left running");
    c.close();
    stop_daemon(dir.path());
}

#[test]
fn another_install_leaves_it_alone() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut a = Conn::open(dir.path(), 1, 1);
    a.send(Msg::Spawn { id: 5, script: "echo $$; exec sleep 60".into() });
    assert_eq!(a.next(), Msg::Spawned { id: 5 });
    let pid: i32 = String::from_utf8(a.stdout_until(5, 2)).unwrap().trim().parse().unwrap();
    let b = Conn::open(dir.path(), 2, 1);
    std::thread::sleep(Duration::from_millis(3000));
    assert!(alive(pid), "a different install's connection ended it");
    b.close();
    a.send(Msg::Kill { id: 5 });
    let until = Instant::now() + Duration::from_secs(3);
    while alive(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(pid));
    // Forgetting is confirmed, known or not.
    a.send(Msg::Forget { id: 5 });
    a.send(Msg::Forget { id: 6 });
    // (After its Eof and Exited frames.)
    let mut gone = Vec::new();
    while gone.len() < 2 {
        if let Msg::Gone { id } = a.next() {
            gone.push(id);
        }
    }
    assert_eq!(gone, [5, 6]);
    a.close();
    stop_daemon(dir.path());
}

#[test]
fn spawn_sent_again_replays_everything() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut a = Conn::open(dir.path(), 3, 1);
    a.send(Msg::Spawn { id: 8, script: "echo hello; echo oops >&2; exit 5".into() });
    // The connection drops before oah reads anything: Spawned, the output and the exit are
    // all on the dead one.
    std::thread::sleep(Duration::from_millis(1500));
    a.close();
    let mut b = Conn::open(dir.path(), 3, 1);
    b.send(Msg::Spawn { id: 8, script: "echo hello; echo oops >&2; exit 5".into() });
    let mut got = Vec::new();
    loop {
        let m = b.next();
        let done = matches!(m, Msg::Exited { .. });
        got.push(m);
        if done {
            break;
        }
    }
    assert_eq!(got[0], Msg::Spawned { id: 8 });
    assert!(got.contains(&Msg::Out { id: 8, stream: STDOUT, off: 0, data: b"hello\n".to_vec() }), "{got:?}");
    assert!(got.contains(&Msg::Out { id: 8, stream: proto::STDERR, off: 0, data: b"oops\n".to_vec() }), "{got:?}");
    assert_eq!(got.last(), Some(&Msg::Exited { id: 8, code: Some(5), signal: None }));
    b.send(Msg::Forget { id: 8 });
    b.close();
    stop_daemon(dir.path());
}
