//! A fake host for the transports' tests: one project, and an "agent" that answers every
//! message in the hub straight away (or asks first, when the message says "ask").

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use oah_agents::driver::Prompt;
use oah_agents::event::{ChatEvent, Decision, ModelOption, SlashCommand};
use oah_core::chats::ChatMeta;
use oah_core::settings::{AgentKind, Location, PermissionMode, Project, Settings};
use serde_json::{json, Value};

use crate::host::*;
use crate::hub::Hub;
use crate::tools::Ctx;

/// A host with one project whose "agent" answers every message in the hub straight away.
pub struct Fake {
    pub hub: Arc<Hub>,
    pub settings: Mutex<Settings>,
    pub chats: Mutex<Vec<ChatMeta>>,
    pub events: Mutex<Vec<Value>>,
    pub sent: Mutex<Vec<(String, Option<PermissionMode>)>>,
}

impl Host for Fake {
    fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
    fn worktrees(&self, _: &str) -> HostResult<Vec<WorktreeInfo>> {
        Ok(vec![WorktreeInfo { path: "/p".into(), branch: Some("main".into()), main: true }])
    }
    fn create_worktree(&self, _: &str, branch: &str) -> HostResult<WorktreeInfo> {
        Ok(WorktreeInfo { path: format!("/wt/{branch}"), branch: Some(branch.into()), main: false })
    }
    fn check_agent(&self, _: AgentKind, _: &Location) -> AgentCheck {
        AgentCheck { installed: true, signed_in: Some(true), ..Default::default() }
    }
    fn catalog(&self, _: AgentKind, _: Option<&str>) -> HostResult<(Vec<ModelOption>, Vec<SlashCommand>)> {
        Ok((
            vec![ModelOption {
                id: "opus".into(),
                name: "Opus".into(),
                efforts: vec!["low".into(), "high".into()],
                default_effort: Some("high".into()),
                ..Default::default()
            }],
            vec![],
        ))
    }
    fn chats(&self) -> Vec<ChatMeta> {
        self.chats.lock().unwrap().clone()
    }
    fn running(&self) -> Vec<String> {
        self.chats.lock().unwrap().iter().map(|c| c.id.clone()).collect()
    }
    fn create(&self, c: NewChat) -> HostResult<ChatMeta> {
        let m = ChatMeta {
            id: format!("c{}", self.chats.lock().unwrap().len()),
            project_id: c.project_id,
            agent: c.agent.unwrap_or(AgentKind::Claude),
            cwd: c.cwd.unwrap_or_else(|| "/p".into()),
            ..Default::default()
        };
        self.chats.lock().unwrap().push(m.clone());
        Ok(m)
    }
    fn send(&self, id: &str, p: Prompt, perm: Option<PermissionMode>) -> HostResult<()> {
        self.sent.lock().unwrap().push((p.text.clone(), perm));
        let evs = vec![
            ChatEvent::UserMessage { id: p.id, text: p.text.clone(), images: vec![], files: vec![] },
            ChatEvent::Text { item_id: "a".into(), text: format!("You said {}", p.text) },
        ];
        for e in &evs {
            self.hub.push(id, e);
            self.events.lock().unwrap().push(serde_json::to_value(e).unwrap());
        }
        let (hub, id, ask) = (self.hub.clone(), id.to_string(), p.text.contains("ask"));
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if ask {
                hub.push(&id, &serde_json::from_value(json!({"type":"approval","request":{"requestId":"r1","itemId":null,"kind":"command","title":"Run a command","detail":"ls","diff":null,"canRemember":true}})).unwrap());
            } else {
                hub.push(
                    &id,
                    &ChatEvent::TurnFinished { turn_id: "t".into(), usage: None, cost_usd: None, duration_ms: Some(5), interrupted: false },
                );
            }
        });
        Ok(())
    }
    fn interrupt(&self, _: &str) -> HostResult<()> {
        Ok(())
    }
    fn answer(&self, id: &str, r: &str, d: Decision) -> HostResult<()> {
        self.hub.push(id, &ChatEvent::ApprovalResolved { request_id: r.into(), decision: d });
        Ok(())
    }
    fn answer_question(&self, _: &str, _: &str, _: BTreeMap<String, Vec<String>>) -> HostResult<()> {
        Ok(())
    }
    fn set_model(&self, _: &str, _: &str) -> HostResult<()> {
        Ok(())
    }
    fn set_effort(&self, _: &str, _: &str) -> HostResult<()> {
        Ok(())
    }
    fn set_permission(&self, _: &str, _: PermissionMode) -> HostResult<()> {
        Ok(())
    }
    fn compact(&self, _: &str, _: &str) -> HostResult<()> {
        Ok(())
    }
    fn clear(&self, _: &str) -> HostResult<()> {
        Ok(())
    }
    fn rename(&self, id: &str, title: &str) -> HostResult<ChatMeta> {
        let mut chats = self.chats.lock().unwrap();
        let m = chats.iter_mut().find(|c| c.id == id).ok_or("no such chat")?;
        m.title = title.into();
        Ok(m.clone())
    }
    fn close(&self, _: &str) {}
    fn delete(&self, _: &str) -> HostResult<()> {
        Ok(())
    }
    fn events(&self, _: &str) -> HostResult<Vec<Value>> {
        Ok(self.events.lock().unwrap().clone())
    }
    fn git_status(&self, _: &str, cwd: Option<&str>) -> HostResult<Value> {
        Ok(json!({"cwd": cwd}))
    }
    fn git_diff(&self, _: &str, _: Option<&str>, _: &str) -> HostResult<String> {
        Ok(String::new())
    }
}

pub fn fake(enabled: bool) -> (Arc<Ctx>, Arc<Fake>) {
    let hub = Arc::new(Hub::new());
    let mut settings = Settings::default();
    settings.mcp.enabled = enabled;
    settings.projects.push(Project { id: "p1".into(), name: "demo".into(), path: "/p".into(), ..Default::default() });
    let host = Arc::new(Fake {
        hub: hub.clone(),
        settings: Mutex::new(settings),
        chats: Mutex::default(),
        events: Mutex::default(),
        sent: Mutex::default(),
    });
    (Arc::new(Ctx::new(host.clone(), hub)), host)
}
