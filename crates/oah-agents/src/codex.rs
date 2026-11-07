//! Codex driver (not wired up yet).

use crate::driver::{Driver, DriverResult, Prompt, Sink, StartOptions};
use crate::event::Decision;

pub struct CodexDriver;

impl CodexDriver {
    pub fn start(_opts: StartOptions, _sink: Sink) -> DriverResult<CodexDriver> {
        Err("Codex isn't wired up yet".into())
    }
}

impl Driver for CodexDriver {
    fn send(&mut self, _prompt: Prompt) -> DriverResult<()> {
        Ok(())
    }
    fn interrupt(&mut self) -> DriverResult<()> {
        Ok(())
    }
    fn answer(&mut self, _request_id: &str, _decision: Decision) -> DriverResult<()> {
        Ok(())
    }
    fn stop(&mut self) {}
}
