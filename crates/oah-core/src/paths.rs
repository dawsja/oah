//! The oah data folder: `$OAH_HOME`, else `$XDG_CONFIG_HOME/oah`, else `~/.config/oah`, on
//! every OS (Windows: `%USERPROFILE%\.config\oah`). The app keeps its own files in `app/`
//! inside it.

use std::path::PathBuf;

/// The home directory: `$HOME` when set, else the OS profile folder.
pub fn home_dir() -> PathBuf {
    env("HOME").or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// `$OAH_HOME`, else `$XDG_CONFIG_HOME/oah`, else `~/.config/oah`.
pub fn oah_home() -> PathBuf {
    if let Some(h) = env("OAH_HOME") {
        return h;
    }
    env("XDG_CONFIG_HOME").unwrap_or_else(|| home_dir().join(".config")).join("oah")
}

/// The app's own folder: `<oah home>/app`.
pub fn app_dir() -> PathBuf {
    oah_home().join("app")
}

pub fn settings_file() -> PathBuf {
    app_dir().join("settings.json")
}

/// One folder per chat: its index entry and timeline.
pub fn chats_dir() -> PathBuf {
    app_dir().join("chats")
}

/// `~/…` for display.
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
    fn resolves_the_data_folder() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let saved: Vec<_> = ["OAH_HOME", "XDG_CONFIG_HOME", "HOME"].iter().map(|k| (*k, std::env::var_os(k))).collect();

        std::env::set_var("HOME", "/home/u");
        std::env::remove_var("OAH_HOME");
        std::env::remove_var("XDG_CONFIG_HOME");
        assert_eq!(oah_home(), PathBuf::from("/home/u/.config/oah"));
        std::env::set_var("XDG_CONFIG_HOME", "");
        assert_eq!(oah_home(), PathBuf::from("/home/u/.config/oah"), "empty XDG is ignored");
        std::env::set_var("XDG_CONFIG_HOME", "/x");
        assert_eq!(oah_home(), PathBuf::from("/x/oah"));
        std::env::set_var("OAH_HOME", "/custom");
        assert_eq!(settings_file(), PathBuf::from("/custom/app/settings.json"));
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
