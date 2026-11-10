//! `oah-host proxy <tag>` (what oah's SSH connection runs) and `oah-host serve <tag>` (the
//! daemon the proxy starts). See the library's docs.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (cmd, tag) = (args.get(1).map(String::as_str), args.get(2).map(String::as_str).unwrap_or(""));
    if cmd == Some("--version") {
        println!("oah-host {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    #[cfg(unix)]
    let result = match cmd {
        Some("proxy") => oah_host::proxy::proxy(tag),
        Some("serve") => oah_host::daemon::serve(tag),
        _ => {
            eprintln!("usage: oah-host proxy|serve <tag>");
            std::process::exit(2);
        }
    };
    #[cfg(not(unix))]
    let result: std::io::Result<()> = {
        let _ = (cmd, tag);
        Err(std::io::Error::other("oah-host runs on Linux and macOS hosts"))
    };
    if let Err(e) = result {
        eprintln!("oah-host: {e}");
        std::process::exit(1);
    }
}
