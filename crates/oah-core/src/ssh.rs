//! SSH helpers that need no connection: quoting for the remote shell and the hosts named in
//! the user's OpenSSH config.

/// POSIX shell single-quoting.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A host name ssh won't read as an option, with no whitespace or control characters.
pub fn valid_host(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('-') && !name.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// `Host` names from an OpenSSH client config (patterns skipped).
pub fn config_hosts(config: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for line in config.lines() {
        let mut words = line.split_whitespace();
        let Some(key) = words.next() else { continue };
        if !key.eq_ignore_ascii_case("host") {
            continue;
        }
        for h in words {
            if h.starts_with('#') {
                break;
            }
            if !h.contains(['*', '?', '!']) && !hosts.iter().any(|x| x == h) {
                hosts.push(h.to_string());
            }
        }
    }
    hosts
}

/// The hosts in `~/.ssh/config` and the files it `Include`s (empty when there's no config).
pub fn user_config_hosts() -> Vec<String> {
    let dir = crate::paths::home_dir().join(".ssh");
    let mut hosts = Vec::new();
    collect(&dir, &dir.join("config"), &mut hosts, 0);
    hosts
}

fn collect(ssh_dir: &std::path::Path, file: &std::path::Path, hosts: &mut Vec<String>, depth: u8) {
    let Ok(text) = std::fs::read_to_string(file) else { return };
    for h in config_hosts(&text) {
        if !hosts.contains(&h) {
            hosts.push(h);
        }
    }
    if depth > 4 {
        return;
    }
    for inc in includes(&text) {
        let inc = inc.strip_prefix("~/").map(|r| crate::paths::home_dir().join(r)).unwrap_or_else(|| ssh_dir.join(&inc));
        // `Include dir/*` (a trailing wildcard) reads every file in the folder.
        let name = inc.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.contains('*') {
            let prefix = name.split('*').next().unwrap_or("").to_string();
            if let Some(parent) = inc.parent() {
                let mut files: Vec<_> = std::fs::read_dir(parent).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix))).collect();
                files.sort();
                for f in files {
                    collect(ssh_dir, &f, hosts, depth + 1);
                }
            }
        } else {
            collect(ssh_dir, &inc, hosts, depth + 1);
        }
    }
}

/// `Include` targets in a config.
pub fn includes(config: &str) -> Vec<String> {
    config
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let k = w.next()?;
            k.eq_ignore_ascii_case("include").then(|| w.take_while(|x| !x.starts_with('#')).map(String::from).collect::<Vec<_>>())
        })
        .flatten()
        .collect()
}

/// `~/…` for paths under `home` (a remote home, `/`-separated).
pub fn short_with_home(p: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    if home.is_empty() {
        return p.to_string();
    }
    if p == home {
        return "~".into();
    }
    match p.strip_prefix(home) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hosts_from_a_config() {
        let cfg = "Host devbox box2 # two\n  HostName 10.0.0.2\nHost *.corp !x\nhost   gpu\nMatch all\nHost devbox\n";
        assert_eq!(config_hosts(cfg), ["devbox", "box2", "gpu"]);
    }

    #[test]
    fn follows_includes() {
        assert_eq!(includes("# x
Include gond/config
  include a b # c
Host x"), ["gond/config", "a", "b"]);
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("config"), "Include sub/*
Host top
").unwrap();
        std::fs::write(dir.path().join("sub").join("one"), "Host ditto
").unwrap();
        let mut hosts = Vec::new();
        collect(dir.path(), &dir.path().join("config"), &mut hosts, 0);
        assert_eq!(hosts, ["top", "ditto"]);
    }

    #[test]
    fn quotes_and_validates() {
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert!(valid_host("dev-box.local"));
        assert!(!valid_host("-oProxyCommand=x"));
        assert!(!valid_host("a b"));
        assert_eq!(short_with_home("/home/u/x", "/home/u/"), "~/x");
        assert_eq!(short_with_home("/home/u", "/home/u"), "~");
        assert_eq!(short_with_home("/home/uu", "/home/u"), "/home/uu");
    }
}
