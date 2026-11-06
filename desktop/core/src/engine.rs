//! The engine is the oah CLI itself, run as `oah serve`: the same agent, tools,
//! sessions and commands as the terminal app, speaking JSON lines over stdio
//! (protocol: `src/serve.ts`). The desktop app spawns one per open workspace.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Protocol major version this app speaks (`PROTOCOL` in `src/serve.ts`).
pub const PROTOCOL: u32 = 1;

/// How to start the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineCommand {
    pub program: PathBuf,
    /// Arguments before `serve` (e.g. the script path when run with Bun).
    pub args: Vec<OsString>,
    /// For the UI: "bundled engine", "oah CLI at …", "source (bun) at …".
    pub describe: String,
    /// Shipped inside the app: it updates with the app, not in place.
    pub bundled: bool,
}

impl EngineCommand {
    fn binary(program: PathBuf, describe: impl Into<String>) -> Self {
        Self { program, args: vec![], describe: describe.into(), bundled: false }
    }
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Look `name` up on `path` (a PATH-style list).
pub fn which(name: &str, path: Option<&OsString>) -> Option<PathBuf> {
    let path = path.cloned().or_else(|| std::env::var_os("PATH"))?;
    std::env::split_paths(&path)
        .map(|d| d.join(exe(name)))
        .find(|p| p.is_file())
}

fn bun(path: Option<&OsString>) -> Option<PathBuf> {
    which("bun", path).or_else(|| {
        let p = crate::paths::home_dir().join(".bun").join("bin").join(exe("bun"));
        p.is_file().then_some(p)
    })
}

/// Find the engine, in order:
/// 1. `$OAH_ENGINE` — an `oah` binary, or a `.ts`/`.tsx` entry point run with Bun;
/// 2. the engine bundled next to the app (`oah-engine`);
/// 3. `dev_source` (the repo's `src/index.tsx`, debug builds) run with Bun;
/// 4. the installed CLI (`<oah home>/bin/oah`), then `oah` on PATH.
pub fn locate(bundled_dir: Option<&Path>, dev_source: Option<&Path>, path: Option<&OsString>) -> Option<EngineCommand> {
    if let Some(e) = std::env::var_os("OAH_ENGINE").filter(|e| !e.is_empty()) {
        let p = PathBuf::from(&e);
        let script = matches!(p.extension().and_then(|x| x.to_str()), Some("ts" | "tsx" | "js"));
        return Some(if script {
            EngineCommand { program: bun(path)?, args: vec![p.into_os_string()], describe: format!("source at {}", p_display(&e)), bundled: false }
        } else {
            EngineCommand::binary(p, format!("$OAH_ENGINE ({})", p_display(&e)))
        });
    }
    if let Some(dir) = bundled_dir {
        let p = dir.join(exe("oah-engine"));
        if p.is_file() {
            return Some(EngineCommand { bundled: true, ..EngineCommand::binary(p, "bundled engine") });
        }
    }
    if let Some(src) = dev_source.filter(|s| s.is_file()) {
        if let Some(b) = bun(path) {
            return Some(EngineCommand { program: b, args: vec![src.as_os_str().to_owned()], describe: format!("source at {}", src.display()), bundled: false });
        }
    }
    let installed = crate::paths::installed_cli();
    if installed.is_file() {
        let d = format!("oah CLI at {}", crate::paths::short_path(&installed.to_string_lossy()));
        return Some(EngineCommand::binary(installed, d));
    }
    which("oah", path).map(|p| {
        let d = format!("oah CLI at {}", p.display());
        EngineCommand::binary(p, d)
    })
}

fn p_display(s: &OsString) -> String {
    s.to_string_lossy().into_owned()
}

/// The PATH a login shell would have. Apps started from the Dock, Finder or a
/// desktop launcher get a minimal PATH, so the engine's tools (git, rg, node,
/// cargo…) wouldn't be found. Unix only; `None` if the shell doesn't answer
/// within `timeout`.
pub fn login_shell_path(timeout: Duration) -> Option<OsString> {
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    let mut child = Command::new(shell)
        .args(["-ilc", "printf '\\n__OAH_PATH__%s\\n' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    let line = out.lines().rev().find_map(|l| l.strip_prefix("__OAH_PATH__"))?;
    (!line.trim().is_empty()).then(|| OsString::from(line.trim()))
}

/// What happened to an engine that stopped.
#[derive(Debug, Clone)]
pub struct Exit {
    pub code: Option<i32>,
    /// The last lines it wrote to stderr (for error reports).
    pub stderr: String,
}

/// A running `oah serve`.
pub struct Engine {
    child: Arc<Mutex<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    pub command: EngineCommand,
    pub cwd: PathBuf,
}

const STDERR_LINES: usize = 60;

impl Engine {
    /// Start `oah serve --cwd <cwd> [--resume <id>]`. Every stdout line is passed
    /// to `on_line` as is (one JSON event); `on_exit` runs once when it stops.
    pub fn spawn(
        command: &EngineCommand,
        cwd: &Path,
        resume: Option<&str>,
        env: &[(OsString, OsString)],
        mut on_line: impl FnMut(String) + Send + 'static,
        on_exit: impl FnOnce(Exit) + Send + 'static,
    ) -> io::Result<Engine> {
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args).arg("serve").arg("--cwd").arg(cwd);
        if let Some(id) = resume {
            cmd.arg("--resume").arg(id);
        }
        cmd.current_dir(cwd).envs(env.iter().map(|(k, v)| (k, v)));
        if command.bundled {
            cmd.env("OAH_BUNDLED_ENGINE", "1");
        }
        cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let child = Arc::new(Mutex::new(child));

        let tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
        let err_thread = {
            let tail = tail.clone();
            thread::Builder::new().name("oah-engine-stderr".into()).spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                    if t.len() == STDERR_LINES {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            })?
        };
        {
            let child = child.clone();
            thread::Builder::new().name("oah-engine-stdout".into()).spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let l = line.trim_end();
                            if !l.is_empty() {
                                on_line(l.to_string());
                            }
                        }
                    }
                }
                // stdout closed: wait for the exit status without holding the lock.
                let code = loop {
                    match child.lock().unwrap_or_else(|e| e.into_inner()).try_wait() {
                        Ok(Some(s)) => break s.code(),
                        Ok(None) => {}
                        Err(_) => break None,
                    }
                    thread::sleep(Duration::from_millis(25));
                };
                let _ = err_thread.join();
                let stderr = tail.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect::<Vec<_>>().join("\n");
                on_exit(Exit { code, stderr });
            })?;
        }
        Ok(Engine { child, stdin: Mutex::new(stdin), command: command.clone(), cwd: cwd.to_path_buf() })
    }

    /// Send one op (a JSON object, without the newline).
    pub fn send(&self, op: &str) -> io::Result<()> {
        let mut guard = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        let stdin = guard.as_mut().ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "engine stopped"))?;
        stdin.write_all(op.trim_end().as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()
    }

    /// Ask it to save and exit; kill it if it hasn't within `grace`.
    pub fn shutdown(&self, grace: Duration) {
        let _ = self.send(r#"{"op":"shutdown"}"#);
        // Closing stdin also tells it to stop.
        self.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
        let start = Instant::now();
        loop {
            let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
            match child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) if start.elapsed() >= grace => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                Ok(None) => {}
            }
            drop(child);
            thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.child.lock().unwrap_or_else(|e| e.into_inner()).try_wait(), Ok(None))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if self.running() {
            self.shutdown(Duration::from_secs(3));
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn fake(script: &str) -> EngineCommand {
        // `sh -c <script>` gets "serve --cwd <dir>" as $0, $1, $2.
        EngineCommand { program: "sh".into(), args: vec!["-c".into(), script.into()], describe: "fake".into(), bundled: false }
    }

    #[test]
    fn relays_lines_and_shuts_down() {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let cwd = tempfile::tempdir().unwrap();
        let script = r#"echo '{"t":"hello","args":"'"$0 $1"'"}'; echo oops >&2; while read l; do case "$l" in *shutdown*) exit 0;; esac; echo "$l"; done"#;
        let e = Engine::spawn(&fake(script), cwd.path(), None, &[], move |l| tx.send(l).unwrap(), move |x| etx.send(x).unwrap()).unwrap();
        let hello = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(hello, r#"{"t":"hello","args":"serve --cwd"}"#);
        e.send(r#"{"op":"submit","text":"hi"}"#).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), r#"{"op":"submit","text":"hi"}"#);
        e.shutdown(Duration::from_secs(5));
        let exit = erx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(exit.code, Some(0));
        assert_eq!(exit.stderr, "oops");
        assert!(e.send("{}").is_err());
    }

    #[test]
    fn kills_an_engine_that_ignores_shutdown() {
        let cwd = tempfile::tempdir().unwrap();
        let (etx, erx) = mpsc::channel();
        let e = Engine::spawn(&fake("trap '' TERM; exec sleep 30"), cwd.path(), Some("abc"), &[], |_| {}, move |x| etx.send(x).unwrap()).unwrap();
        let t = Instant::now();
        e.shutdown(Duration::from_millis(300));
        assert!(t.elapsed() < Duration::from_secs(5));
        assert!(!e.running());
        assert!(erx.recv_timeout(Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn locates_the_bundled_engine_first() {
        let _g = crate::paths::tests::ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("OAH_ENGINE");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("oah-engine"), "").unwrap();
        let none = OsString::from("/nonexistent");
        let c = locate(Some(dir.path()), None, Some(&none)).unwrap();
        assert_eq!(c.program, dir.path().join("oah-engine"));
        assert_eq!(c.describe, "bundled engine");
        assert!(c.bundled);
    }

    /// The real thing: the repo's engine run from source with Bun, when available.
    #[test]
    fn talks_to_the_real_engine() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/index.tsx");
        let Some(bun) = bun(None) else { return };
        if !src.is_file() {
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let cmd = EngineCommand { program: bun, args: vec![src.into_os_string()], describe: "src".into(), bundled: false };
        let env = [("OAH_HOME".into(), home.path().as_os_str().to_owned()), ("OAH_NO_UPDATE_CHECK".into(), "1".into())];
        let (tx, rx) = mpsc::channel::<String>();
        let e = Engine::spawn(&cmd, cwd.path(), None, &env, move |l| { let _ = tx.send(l); }, |_| {}).unwrap();
        let wait = |pred: &dyn Fn(&serde_json::Value) -> bool| loop {
            let l = rx.recv_timeout(Duration::from_secs(20)).expect("engine event");
            let v: serde_json::Value = serde_json::from_str(&l).unwrap();
            if pred(&v) {
                return v;
            }
        };
        let hello = wait(&|v| v["t"] == "hello");
        assert_eq!(hello["protocol"], PROTOCOL);
        e.send(r#"{"op":"mode","value":"plan"}"#).unwrap();
        wait(&|v| v["t"] == "state" && v["s"]["settings"]["mode"] == "plan");
        e.shutdown(Duration::from_secs(10));
        assert!(home.path().join("settings.json").is_file(), "settings are the CLI's");
    }
}
