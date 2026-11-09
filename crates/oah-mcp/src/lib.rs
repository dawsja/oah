//! MCP for oah: another agent (Claude Code, Codex, anything that speaks MCP) lists projects,
//! starts and drives chats, answers their approvals and questions, and sees models and efforts.
//!
//! The app serves it on a local socket ([`server::listen`]) over its own live chats, through
//! [`host::Host`]; [`hub::Hub`] follows what each chat is doing from its events. MCP clients run
//! `oah mcp` ([`bridge::run`]), which passes stdio through to that socket.

pub mod bridge;
pub mod host;
pub mod hub;
pub mod search;
pub mod server;
pub mod tools;
pub mod transcript;

pub use host::Host;
pub use hub::Hub;
pub use tools::Ctx;
