//! The agent CLIs oah drives. [`driver::start`] runs one behind a chat; each driver turns its
//! CLI's protocol into [`event::ChatEvent`]s. [`probe`] finds a CLI and its version.

pub mod account;
pub mod acp;
pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod driver;
pub mod event;
pub mod jsonrpc;
pub mod mock;
pub mod oneshot;
pub mod probe;
