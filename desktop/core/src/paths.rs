//! The oah data folder, resolved exactly like the CLI (`src/config.ts`):
//! `$OAH_HOME`, else `$XDG_CONFIG_HOME/oah`, else `~/.config/oah` — on every OS,
//! Windows included (`%USERPROFILE%\.config\oah`).

use std::path::PathBuf;

/// The user's home directory as Node/Bun's `os.homedir()` sees it: `$HOME`
/// when set (libuv checks it first on every platform), else the OS profile dir.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// `$OAH_HOME`, else `$XDG_CONFIG_HOME/oah`, else `~/.config/oah`.
pub fn oah_home() -> PathBuf {
    if let Some(h) = env("OAH_HOME") {
        return h;
    }
    env("XDG_CONFIG_HOME")
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("oah")
}

pub fn sessions_dir() -> PathBuf {
    oah_home().join("sessions")
}

pub fn settings_file() -> PathBuf {
    oah_home().join("settings.json")
}

/// Desktop-only preferences (window layout, last workspace). The CLI ignores it.
pub fn desktop_prefs_file() -> PathBuf {
    oah_home().join("desktop.json")
}

/// Where `install.sh` / `install.ps1` put the CLI: `$OAH_INSTALL/bin`, else `<oah home>/bin`.
pub fn installed_cli() -> PathBuf {
    let root = env("OAH_INSTALL").unwrap_or_else(|| {
        env("XDG_CONFIG_HOME")
            .unwrap_or_else(|| home_dir().join(".config"))
            .join("oah")
    });
    root.join("bin").join(if cfg!(windows) { "oah.exe" } else { "oah" })
}

/// `~/…` for display, like the CLI's `shortPath`.
pub fn short_path(p: &str) -> String {
    let home = home_dir();
    let h = home.to_string_lossy();
    match p.strip_prefix(h.as_ref()) {
        Some(rest) if !h.is_empty() => format!("~{rest}"),
        _ => p.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Tests that touch process env vars run one at a time.
    pub static ENV: Mutex<()> = Mutex::new(());

    #[test]
    fn resolves_like_the_cli() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let saved: Vec<_> = ["OAH_HOME", "XDG_CONFIG_HOME", "HOME"]
            .iter()
            .map(|k| (*k, std::env::var_os(k)))
            .collect();

        std::env::set_var("HOME", "/home/u");
        std::env::remove_var("OAH_HOME");
        std::env::remove_var("XDG_CONFIG_HOME");
        assert_eq!(oah_home(), PathBuf::from("/home/u/.config/oah"));

        std::env::set_var("XDG_CONFIG_HOME", "");
        assert_eq!(oah_home(), PathBuf::from("/home/u/.config/oah"), "empty XDG is ignored");

        std::env::set_var("XDG_CONFIG_HOME", "/x");
        assert_eq!(oah_home(), PathBuf::from("/x/oah"));

        std::env::set_var("OAH_HOME", "/custom");
        assert_eq!(oah_home(), PathBuf::from("/custom"));
        assert_eq!(sessions_dir(), PathBuf::from("/custom/sessions"));

        assert_eq!(short_path("/home/u/code/x"), "~/code/x");
        assert_eq!(short_path("/srv/x"), "/srv/x");

        for (k, v) in saved {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}
