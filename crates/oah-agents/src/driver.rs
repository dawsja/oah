//! A running agent CLI behind one chat. [`start`] picks the driver for the agent; every driver
//! reports through a [`Sink`] and takes prompts, interrupts and approval answers.

use std::sync::Arc;

use oah_core::settings::{AgentKind, EnvVar, Location, PermissionMode};
use serde::Deserialize;

use crate::event::{ChatEvent, Decision};

/// Where a driver's events go. Called from the driver's reader thread.
pub type Sink = Arc<dyn Fn(ChatEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct StartOptions {
    pub kind: AgentKind,
    /// The folder the agent works in (on `location`).
    pub cwd: String,
    pub location: Location,
    /// The CLI; empty is the usual command.
    pub binary: String,
    pub model: Option<String>,
    pub permission: PermissionMode,
    /// The CLI's session id to continue, from an earlier `Started`.
    pub resume: Option<String>,
    pub env: Vec<EnvVar>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    /// `image/png`…
    pub mime: String,
    /// Base64 bytes.
    pub data: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub images: Vec<Image>,
}

pub type DriverResult<T> = Result<T, String>;

pub trait Driver: Send {
    /// Sends a user message; the reply streams to the sink.
    fn send(&mut self, prompt: Prompt) -> DriverResult<()>;
    /// Stops the current turn (the CLI keeps running).
    fn interrupt(&mut self) -> DriverResult<()>;
    /// Answers an [`crate::event::ApprovalRequest`].
    fn answer(&mut self, request_id: &str, decision: Decision) -> DriverResult<()>;
    /// Changes the permission mode for the next tool calls, when the CLI can.
    fn set_permission(&mut self, _mode: PermissionMode) -> DriverResult<()> {
        Ok(())
    }
    /// Answers a [`crate::event::QuestionRequest`]: answers by question id.
    fn answer_question(&mut self, _request_id: &str, _answers: std::collections::BTreeMap<String, Vec<String>>) -> DriverResult<()> {
        Err("this agent doesn't ask questions".into())
    }
    /// Switches the model for the next turns.
    fn set_model(&mut self, _model: &str) -> DriverResult<()> {
        Ok(())
    }
    /// Ends the CLI process.
    fn stop(&mut self);
}

/// Starts the agent for `opts`. `OAH_MOCK_AGENT=1` swaps every agent for the scripted mock.
pub fn start(opts: StartOptions, sink: Sink) -> DriverResult<Box<dyn Driver>> {
    if std::env::var("OAH_MOCK_AGENT").is_ok_and(|v| v == "1") {
        return Ok(Box::new(crate::mock::MockDriver::start(opts, sink)));
    }
    match opts.kind {
        AgentKind::Claude => crate::claude::ClaudeDriver::start(opts, sink).map(|d| Box::new(d) as Box<dyn Driver>),
        AgentKind::Codex => crate::codex::CodexDriver::start(opts, sink).map(|d| Box::new(d) as Box<dyn Driver>),
    }
}
