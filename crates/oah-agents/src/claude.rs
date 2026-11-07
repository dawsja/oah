//! Claude driver (not wired up yet).

use crate::driver::{Driver, DriverResult, Prompt, Sink, StartOptions};
use crate::event::Decision;

pub struct ClaudeDriver;

impl ClaudeDriver {
    pub fn start(_opts: StartOptions, _sink: Sink) -> DriverResult<ClaudeDriver> {
        Err("Claude isn't wired up yet".into())
    }
}

impl Driver for ClaudeDriver {
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
