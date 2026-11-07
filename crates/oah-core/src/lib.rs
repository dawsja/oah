//! oah's data, with no GUI dependencies: where it lives ([`paths`]), the settings file
//! ([`settings`]) and the hosts in `~/.ssh/config` ([`ssh`]).

pub mod paths;
pub mod settings;
pub mod ssh;

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
