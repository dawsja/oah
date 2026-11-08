//! `oah mcp`: what an MCP client runs. It passes stdio through to the running app's local
//! socket, starting oah first when it isn't open (and MCP is on).

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use oah_core::{paths, settings::Settings};

use crate::server::{self, OFF};

/// Runs the bridge until either side hangs up; the process exit code.
pub fn run() -> i32 {
    let path = paths::mcp_socket();
    let conn = match server::connect(&path) {
        Ok(c) => c,
        Err(_) => {
            let enabled = Settings::load(&paths::settings_file()).map(|s| s.mcp.enabled).unwrap_or(false);
            if !enabled {
                eprintln!("oah: {OFF}");
                return 1;
            }
            if let Err(e) = start_app() {
                eprintln!("oah: couldn't start oah: {e}");
                return 1;
            }
            match wait_for(&path, Duration::from_secs(30)) {
                Some(c) => c,
                None => {
                    eprintln!("oah: oah didn't open its MCP socket ({path}). Is it running?");
                    return 1;
                }
            }
        }
    };
    let mut from_app = server::Shared(std::sync::Arc::new(conn));
    let mut to_app = from_app.clone();
    // App → stdout; when the app hangs up, the bridge is done.
    let out = std::thread::spawn(move || {
        let mut stdout = std::io::stdout();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match from_app.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if stdout.write_all(&buf[..n]).and_then(|_| stdout.flush()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    // stdin → app, until the client closes stdin.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut to_app);
        std::process::exit(0);
    });
    let _ = out.join();
    0
}

fn wait_for(path: &str, timeout: Duration) -> Option<interprocess::local_socket::Stream> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(c) = server::connect(path) {
            return Some(c);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    None
}

/// Starts this same executable as the app, detached from the client's stdio.
fn start_app() -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map(|_| ())
}
