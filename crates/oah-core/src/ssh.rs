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

/// The hosts in `~/.ssh/config` (empty when there's no config).
pub fn user_config_hosts() -> Vec<String> {
    let path = crate::paths::home_dir().join(".ssh").join("config");
    std::fs::read_to_string(path).map(|c| config_hosts(&c)).unwrap_or_default()
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
