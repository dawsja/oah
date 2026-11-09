//! oah's data, with no GUI dependencies: where it lives ([`paths`]), the settings file
//! ([`settings`]) and the hosts in `~/.ssh/config` ([`ssh`]).

pub mod chats;
pub mod paths;
pub mod settings;
pub mod ssh;
pub mod wsl;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("settings file is not valid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Writes `path` whole or not at all: to a temp file beside it (named for this write, so two
/// writers never share one), flushed to disk, then renamed over it.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().ok_or_else(|| Error::Invalid(format!("{} has no file name", path.display())))?;
    let tmp =
        path.with_file_name(format!("{}.{}-{}.tmp", name.to_string_lossy(), std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(written?)
}
