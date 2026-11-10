//! `oah-host proxy <tag>`: what oah's SSH connection runs. It joins the tag's daemon (starting
//! it when none is running), says which agent socket its SSH session forwarded, prints
//! [`MAGIC`](crate::proto::MAGIC), and then just copies bytes both ways until either side ends.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::proto::{Msg, MAGIC};

/// Never returns on success: the process exits when the connection ends.
pub fn proxy(tag: &str) -> io::Result<()> {
    let paths = crate::paths(tag)?;
    let mut sock = match UnixStream::connect(&paths.sock) {
        Ok(s) => s,
        Err(_) => {
            start_daemon(tag, &paths)?;
            connect_within(&paths.sock, Duration::from_secs(10))?
        }
    };
    let auth_sock = std::env::var("SSH_AUTH_SOCK").unwrap_or_default();
    sock.write_all(&Msg::Proxy { auth_sock }.encode())?;
    // Unbuffered stdio: frames go out as soon as they're whole.
    // SAFETY: fds 0 and 1 are this process's stdio, owned by nothing else here.
    let mut stdout = unsafe { File::from_raw_fd(1) };
    let mut stdin = unsafe { File::from_raw_fd(0) };
    stdout.write_all(MAGIC)?;
    let mut from_daemon = sock.try_clone()?;
    std::thread::spawn(move || {
        copy(&mut from_daemon, &mut stdout);
        // The daemon hung up (it exited, or closed us).
        std::process::exit(0);
    });
    copy(&mut stdin, &mut sock);
    // oah hung up (the SSH connection ended).
    let _ = sock.shutdown(std::net::Shutdown::Both);
    std::process::exit(0);
}

fn copy(from: &mut impl Read, to: &mut impl Write) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        if to.write_all(&buf[..n]).is_err() {
            return;
        }
    }
}

/// `oah-host serve <tag>`, in its own session so the SSH session ending doesn't end it, with its
/// messages in the tag's log.
fn start_daemon(tag: &str, paths: &crate::Paths) -> io::Result<()> {
    let exe = std::env::current_exe()?;
    // A fresh log each start, so it never grows without bound.
    let log = OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&paths.log)?;
    let mut c = Command::new(exe);
    c.arg("serve").arg(tag).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        c.pre_exec(|| if libc::setsid() < 0 { Err(io::Error::last_os_error()) } else { Ok(()) });
    }
    c.spawn()?;
    Ok(())
}

fn connect_within(sock: &std::path::Path, limit: Duration) -> io::Result<UnixStream> {
    let until = Instant::now() + limit;
    loop {
        match UnixStream::connect(sock) {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() >= until => return Err(io::Error::new(e.kind(), format!("the oah-host daemon didn't start: {e}"))),
            Err(_) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}
