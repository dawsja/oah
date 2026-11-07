//! WSL (Windows Subsystem for Linux) names and paths. A WSL project is a distro plus a Linux
//! path; the Windows folder picker hands back `\\wsl.localhost\<distro>\…` (or `\\wsl$\…`),
//! which is turned into that pair here.

/// A distro name oah will pass to `wsl.exe -d`: letters, digits, `.`, `_`, `-`, not starting
/// with `-` (so it can't be read as an option).
pub fn valid_distro(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `\\wsl.localhost\Ubuntu\home\u\code` (or `\\wsl$\…`, either slash) → `("Ubuntu", "/home/u/code")`.
pub fn from_unc(path: &str) -> Option<(String, String)> {
    let p = path.replace('\\', "/");
    let rest = p.strip_prefix("//")?;
    let (server, rest) = rest.split_once('/')?;
    if !server.eq_ignore_ascii_case("wsl.localhost") && server != "wsl$" {
        return None;
    }
    let (distro, linux) = match rest.split_once('/') {
        Some((d, l)) => (d, format!("/{}", l.trim_end_matches('/'))),
        None => (rest, "/".to_string()),
    };
    let linux = if linux.is_empty() { "/".to_string() } else { linux };
    valid_distro(distro).then(|| (distro.to_string(), linux))
}

/// The Windows drive a Linux path is on, if it's under WSL's automount (`/mnt/c/…` → `C:\…`).
pub fn windows_drive_path(linux: &str) -> Option<String> {
    let rest = linux.strip_prefix("/mnt/")?;
    let mut chars = rest.chars();
    let drive = chars.next().filter(|c| c.is_ascii_lowercase())?;
    let after = chars.as_str();
    if !after.is_empty() && !after.starts_with('/') {
        return None;
    }
    Some(format!("{}:\\{}", drive.to_ascii_uppercase(), after.trim_start_matches('/').replace('/', "\\")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_distro("Ubuntu"));
        assert!(valid_distro("Ubuntu-24.04"));
        assert!(valid_distro("my_distro"));
        assert!(!valid_distro("-d"));
        assert!(!valid_distro("a b"));
        assert!(!valid_distro("a;b"));
        assert!(!valid_distro(""));
    }

    #[test]
    fn unc_paths() {
        assert_eq!(from_unc(r"\\wsl.localhost\Ubuntu\home\daws\github\oah"), Some(("Ubuntu".into(), "/home/daws/github/oah".into())));
        assert_eq!(from_unc(r"\\wsl$\Debian\home\u\"), Some(("Debian".into(), "/home/u".into())));
        assert_eq!(from_unc(r"\\WSL.LOCALHOST\Ubuntu"), Some(("Ubuntu".into(), "/".into())));
        assert_eq!(from_unc("//wsl.localhost/Ubuntu/etc"), Some(("Ubuntu".into(), "/etc".into())));
        assert_eq!(from_unc(r"\\server\share\x"), None);
        assert_eq!(from_unc(r"C:\Users\u"), None);
    }

    #[test]
    fn windows_drives() {
        assert_eq!(windows_drive_path("/mnt/c/Users/u/code").as_deref(), Some(r"C:\Users\u\code"));
        assert_eq!(windows_drive_path("/mnt/d").as_deref(), Some(r"D:\"));
        assert_eq!(windows_drive_path("/mnt/wsl/x"), None);
        assert_eq!(windows_drive_path("/home/u"), None);
    }
}
