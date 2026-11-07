//! `settings.json`: everything the Settings page edits. Every field has a default, so an older
//! or hand-edited file still loads; the file is written whole, atomically.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub providers: Providers,
    pub voice: Voice,
    pub appearance: Appearance,
    pub ssh_hosts: Vec<SshHost>,
    pub projects: Vec<Project>,
}

// ---- Providers -------------------------------------------------------------------------------

/// The agent CLIs oah drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    /// OpenCode, over ACP (`opencode acp`).
    Opencode,
    /// An agent the user added, over the Agent Client Protocol (see `Providers::acp`).
    Acp,
}

impl AgentKind {
    /// The built-in agents (each has its own Provider settings).
    pub const ALL: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::Opencode];

    /// The command when no path is set.
    pub fn default_binary(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Opencode => "opencode",
            AgentKind::Acp => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Providers {
    pub claude: Provider,
    pub codex: Provider,
    pub opencode: Provider,
    /// Other agents that speak ACP, added by the user.
    pub acp: Vec<AcpAgent>,
    /// The agent a new chat starts with when its project names none.
    pub default_agent: AgentKind,
}

impl Default for Providers {
    fn default() -> Self {
        Providers {
            claude: Provider::default(),
            codex: Provider::default(),
            opencode: Provider::default(),
            acp: Vec::new(),
            default_agent: AgentKind::Claude,
        }
    }
}

/// An ACP agent the user added: a command that speaks the Agent Client Protocol on stdio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AcpAgent {
    pub id: String,
    pub name: String,
    /// Arguments after the command (`provider.binary`), e.g. `["--experimental-acp"]`.
    pub args: Vec<String>,
    /// The command, model, permissions and environment, as for the built-in agents.
    #[serde(flatten)]
    pub provider: Provider,
}

static NO_PROVIDER: std::sync::LazyLock<Provider> = std::sync::LazyLock::new(Provider::default);

impl Providers {
    /// A built-in agent's settings (`Acp` agents are looked up with [`Providers::acp_agent`]).
    pub fn get(&self, kind: AgentKind) -> &Provider {
        match kind {
            AgentKind::Claude => &self.claude,
            AgentKind::Codex => &self.codex,
            AgentKind::Opencode => &self.opencode,
            AgentKind::Acp => &NO_PROVIDER,
        }
    }

    pub fn acp_agent(&self, id: &str) -> Option<&AcpAgent> {
        self.acp.iter().find(|a| a.id == id)
    }

    /// The settings a chat's agent runs with.
    pub fn for_chat(&self, kind: AgentKind, acp_id: Option<&str>) -> Option<&Provider> {
        match kind {
            AgentKind::Acp => acp_id.and_then(|id| self.acp_agent(id)).map(|a| &a.provider),
            k => Some(self.get(k)),
        }
    }

    pub fn get_mut(&mut self, kind: AgentKind) -> Option<&mut Provider> {
        match kind {
            AgentKind::Claude => Some(&mut self.claude),
            AgentKind::Codex => Some(&mut self.codex),
            AgentKind::Opencode => Some(&mut self.opencode),
            AgentKind::Acp => None,
        }
    }
}

/// How much an agent may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Ask before edits and commands.
    #[default]
    Ask,
    /// Edits in the project folder go through; commands still ask.
    AcceptEdits,
    /// Plan only: read, no changes.
    Plan,
    /// Never ask.
    FullAccess,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Provider {
    pub enabled: bool,
    /// A path to the CLI; empty means the one on PATH.
    pub binary: String,
    /// Empty means the CLI's own default.
    pub default_model: String,
    pub permission_mode: PermissionMode,
    /// Extra environment for the CLI process.
    pub env: Vec<EnvVar>,
}

impl Provider {
    /// The executable to run on `location`. A Windows path (`C:\\…`, `\\…`) set here is for this
    /// computer only; on an SSH host or in WSL the CLI is looked up on that system's PATH. A
    /// bare name or a Linux path is used everywhere.
    pub fn binary_on(&self, location: &Location) -> String {
        let b = self.binary.trim();
        let windows_path = b.contains('\\') || b.as_bytes().get(1) == Some(&b':');
        if location.is_remote() && windows_path {
            String::new()
        } else {
            b.to_string()
        }
    }
}

impl Default for Provider {
    fn default() -> Self {
        Provider {
            enabled: true,
            binary: String::new(),
            default_model: String::new(),
            permission_mode: PermissionMode::default(),
            env: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EnvVar {
    pub key: String,
    /// Empty once the value is in the OS keychain (`in_keychain`).
    pub value: String,
    /// The value lives in the OS keychain, not in this file.
    #[serde(rename = "inKeychain")]
    pub in_keychain: bool,
}

// ---- Voice -----------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Voice {
    /// Speech to text is on (the model is downloaded).
    pub enabled: bool,
    /// The microphone's device id; empty is the system default.
    pub microphone: String,
    /// Hold this key to dictate into the composer.
    pub hold_key: String,
    /// Stop recording after this much silence (seconds, 0 = never).
    pub silence_stop_secs: u32,
}

impl Default for Voice {
    fn default() -> Self {
        Voice { enabled: false, microphone: String::new(), hold_key: "Ctrl+Space".into(), silence_stop_secs: 0 }
    }
}

// ---- Appearance ------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Appearance {
    pub theme: Theme,
    /// Base UI size in px.
    pub ui_font_size: u32,
    /// Chat text size in px.
    pub chat_font_size: u32,
    pub sans_font: String,
    pub mono_font: String,
    pub density: Density,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            theme: Theme::System,
            ui_font_size: 13,
            chat_font_size: 14,
            sans_font: "Inter Variable".into(),
            mono_font: "JetBrains Mono Variable".into(),
            density: Density::Comfortable,
        }
    }
}

// ---- SSH hosts and projects ------------------------------------------------------------------

/// A host added by hand. Hosts from `~/.ssh/config` are listed live, not stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SshHost {
    /// What `ssh` is given: an alias or `user@host`.
    pub host: String,
    /// `-p`; 0 is ssh's default.
    pub port: u16,
    /// `-i`; empty is ssh's default.
    pub identity_file: String,
    /// Shown instead of `host` when set.
    pub label: String,
}

/// Where a project's folder is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[derive(Default)]
pub enum Location {
    #[default]
    Local,
    Ssh {
        host: String,
    },
    /// A WSL distro on this computer (Windows only); the project path is a Linux path in it.
    Wsl {
        distro: String,
    },
}

impl Location {
    /// The project's files are on another system (an SSH host, a WSL distro): Linux paths, and
    /// programs are found on that system's PATH.
    pub fn is_remote(&self) -> bool {
        !matches!(self, Location::Local)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub location: Location,
    /// The folder (on `location`).
    pub path: String,
    /// The agent new chats start with; none means the Providers default.
    pub default_agent: Option<AgentKind>,
    /// Empty is the agent's default model.
    pub default_model: String,
    pub git: ProjectGit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct ProjectGit {
    /// New worktrees branch from this; empty is the remote's default branch.
    pub base_branch: String,
    /// Where new worktrees go; empty is `<repo>/../<repo>-worktrees`.
    pub worktree_dir: String,
    /// Removing a worktree also deletes its branch.
    pub delete_branch_on_remove: bool,
}

impl Project {
    /// A new project for `path`, named after its last component.
    pub fn new(location: Location, path: &str) -> Project {
        let trimmed = path.trim_end_matches(['/', '\\']);
        let name = trimmed.rsplit(['/', '\\']).next().filter(|n| !n.is_empty()).unwrap_or(trimmed).to_string();
        Project {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            name,
            location,
            path: path.to_string(),
            ..Default::default()
        }
    }
}

// ---- The file --------------------------------------------------------------------------------

impl Settings {
    /// Reads `path`; a missing file is the defaults.
    pub fn load(path: &Path) -> Result<Settings> {
        match std::fs::read_to_string(path) {
            Ok(text) if text.trim().is_empty() => Ok(Settings::default()),
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes `path` whole: to a temp file beside it, then renamed over it.
    pub fn save(&self, path: &Path) -> Result<()> {
        let dir = path.parent().ok_or_else(|| Error::Invalid("settings path has no folder".into()))?;
        std::fs::create_dir_all(dir)?;
        let tmp: PathBuf = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)? + "\n")?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Adds a project, unless one for the same folder is already there (that one is returned).
    pub fn add_project(&mut self, location: Location, path: &str) -> Result<Project> {
        let path = path.trim();
        if path.is_empty() {
            return Err(Error::Invalid("no folder given".into()));
        }
        match &location {
            Location::Ssh { host } if !crate::ssh::valid_host(host) => {
                return Err(Error::Invalid(format!("not a valid SSH host: {host:?}")));
            }
            Location::Wsl { distro } if !crate::wsl::valid_distro(distro) => {
                return Err(Error::Invalid(format!("not a valid WSL distro name: {distro:?}")));
            }
            Location::Wsl { .. } if !path.starts_with('/') => {
                return Err(Error::Invalid(format!("a WSL project needs a Linux path (like /home/you/code), not {path:?}")));
            }
            _ => {}
        }
        let local = location == Location::Local;
        if let Some(p) = self.projects.iter().find(|p| p.location == location && same_path(&p.path, path, local)) {
            return Ok(p.clone());
        }
        let p = Project::new(location, path);
        self.projects.push(p.clone());
        Ok(p)
    }

    pub fn remove_project(&mut self, id: &str) -> bool {
        let before = self.projects.len();
        self.projects.retain(|p| p.id != id);
        self.projects.len() != before
    }
}

/// The same folder: Windows paths compare without case, Linux paths (SSH, WSL) with it.
fn same_path(a: &str, b: &str, local: bool) -> bool {
    let norm = |s: &str| s.trim_end_matches(['/', '\\']).replace('\\', "/");
    if local && cfg!(windows) {
        norm(a).eq_ignore_ascii_case(&norm(b))
    } else {
        norm(a) == norm(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app").join("settings.json");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default(), "missing file is the defaults");

        let mut s = Settings::default();
        s.appearance.theme = Theme::Dark;
        s.providers.codex.default_model = "gpt-5.5-codex".into();
        s.ssh_hosts.push(SshHost { host: "devbox".into(), ..Default::default() });
        let p = s.add_project(Location::Ssh { host: "devbox".into() }, "/home/u/code/api/").unwrap();
        assert_eq!(p.name, "api");
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);

        std::fs::write(&path, r#"{"appearance":{"theme":"light"},"futureField":1}"#).unwrap();
        let old = Settings::load(&path).unwrap();
        assert_eq!(old.appearance.theme, Theme::Light);
        assert_eq!(old.appearance.ui_font_size, 13, "missing fields get defaults");
        assert!(old.providers.claude.enabled);
    }

    #[test]
    fn windows_executables_stay_on_this_computer() {
        let p = |b: &str| Provider { binary: b.into(), ..Default::default() };
        let wsl = Location::Wsl { distro: "Ubuntu".into() };
        assert_eq!(p(r"C:\Users\u\.local\bin\claude.exe").binary_on(&Location::Local), r"C:\Users\u\.local\bin\claude.exe");
        assert_eq!(p(r"C:\Users\u\.local\bin\claude.exe").binary_on(&wsl), "");
        assert_eq!(p(r"\\server\tools\codex.cmd").binary_on(&Location::Ssh { host: "box".into() }), "");
        assert_eq!(p("/opt/claude/bin/claude").binary_on(&wsl), "/opt/claude/bin/claude");
        assert_eq!(p("claude-beta").binary_on(&wsl), "claude-beta");
    }

    #[test]
    fn projects_are_unique_per_folder() {
        let mut s = Settings::default();
        let a = s.add_project(Location::Local, "/code/web").unwrap();
        let b = s.add_project(Location::Local, "/code/web/").unwrap();
        assert_eq!(a.id, b.id);
        let c = s.add_project(Location::Ssh { host: "box".into() }, "/code/web").unwrap();
        assert_ne!(a.id, c.id, "same folder on another machine is another project");
        assert!(s.add_project(Location::Ssh { host: "-oX".into() }, "/x").is_err());
        let w = s.add_project(Location::Wsl { distro: "Ubuntu".into() }, "/home/u/web").unwrap();
        assert_eq!(w.name, "web");
        assert_ne!(s.add_project(Location::Wsl { distro: "Ubuntu".into() }, "/home/u/Web").unwrap().id, w.id, "Linux paths keep case");
        assert!(s.add_project(Location::Wsl { distro: "-d".into() }, "/x").is_err());
        assert!(s.add_project(Location::Wsl { distro: "Ubuntu".into() }, r"C:\x").is_err());
        assert!(s.remove_project(&a.id));
        assert_eq!(s.projects.len(), 3);
        assert_eq!(Project::new(Location::Local, r"C:\Users\u\code\oah").name, "oah");
    }
}
