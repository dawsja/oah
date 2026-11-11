//! The agent CLIs oah drives. [`driver::start`] runs one behind a chat; each driver turns its
//! CLI's protocol into [`event::ChatEvent`]s. [`probe`] finds a CLI and its version; [`cli`] is
//! what opens a chat's session in the CLI's own terminal interface.

pub mod account;
pub mod acp;
pub mod antigravity;
pub mod claude;
pub mod cli;
pub mod codex;
pub mod driver;
pub mod event;
pub mod grok;
pub mod jsonrpc;
pub mod mock;
pub mod oneshot;
pub mod probe;
