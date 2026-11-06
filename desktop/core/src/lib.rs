//! Everything the oah desktop app does outside the webview, without GUI
//! dependencies: where the CLI keeps its data, the saved-session index for the
//! sidebar, desktop-only preferences, the `oah serve` engine process, and
//! running it on another machine (WSL or SSH, see [`remote`]).
//!
//! The desktop app and the terminal CLI share one data folder (see [`paths`]),
//! so sessions, settings, sign-ins, memory, skills and MCP servers are the same
//! in both.

pub mod engine;
pub mod paths;
pub mod prefs;
pub mod remote;
pub mod sessions;
