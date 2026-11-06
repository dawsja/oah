//! Memory: instruction files plus persistent notes.
//!
//! * Instruction files are read every turn: `~/.nar/AGENTS.md` (global), then
//!   for each directory from the repository root down to the working directory
//!   the first of `AGENTS.md`, `NAR.md`, `CLAUDE.md`.
//! * Notes are durable facts the user or the model saved (`/memory add`, `#note`,
//!   or the `memory` tool): `~/.nar/memory/global.md` and a per-project file.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::config;

pub struct Memory {
    pub root: PathBuf,
    pub cwd: PathBuf,
    pub global_file: PathBuf,
    pub project_file: PathBuf,
    lock: Mutex<()>,
}

pub fn project_root(cwd: &Path) -> PathBuf {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        if d.join(".git").exists() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    cwd.to_path_buf()
}

impl Memory {
    pub fn new(cwd: &Path) -> Memory {
        let root = project_root(cwd);
        let hash = Sha256::digest(root.display().to_string().as_bytes());
        let id: String = hash.iter().take(8).map(|b| format!("{b:02x}")).collect();
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "root".into());
        let mem = config::home_dir().join("memory");
        Memory {
            global_file: mem.join("global.md"),
            project_file: mem.join("projects").join(format!("{name}-{id}.md")),
            root,
            cwd: cwd.to_path_buf(),
            lock: Mutex::new(()),
        }
    }

    /// Instruction files that apply to the working directory, outermost first.
    pub fn instruction_files(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let global = config::home_dir().join("AGENTS.md");
        if global.is_file() {
            out.push(global);
        }
        let mut chain = Vec::new();
        let mut d = Some(self.cwd.as_path());
        while let Some(dir) = d {
            chain.push(dir.to_path_buf());
            if dir == self.root {
                break;
            }
            d = dir.parent();
        }
        chain.reverse();
        for dir in chain {
            for name in ["AGENTS.md", "NAR.md", "CLAUDE.md"] {
                let p = dir.join(name);
                if p.is_file() {
                    out.push(p);
                    break;
                }
            }
        }
        out
    }

    fn entries(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.strip_prefix("- ").map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn global_entries(&self) -> Vec<String> {
        Self::entries(&self.global_file)
    }
    pub fn project_entries(&self) -> Vec<String> {
        Self::entries(&self.project_file)
    }

    fn write(path: &Path, title: &str, entries: &[String]) -> std::io::Result<()> {
        let mut s = format!("# {title}\n\n");
        for e in entries {
            s.push_str(&format!("- {e}\n"));
        }
        config::write_atomic(path, s.as_bytes())
    }

    pub fn add(&self, text: &str, global: bool) -> anyhow::Result<()> {
        let text = text.trim().replace('\n', " ");
        if text.is_empty() {
            anyhow::bail!("nothing to remember");
        }
        let _g = self.lock.lock().unwrap();
        let (path, title) = if global {
            (&self.global_file, "nar global memory".to_string())
        } else {
            (&self.project_file, format!("nar memory for {}", self.root.display()))
        };
        let mut entries = Self::entries(path);
        if !entries.iter().any(|e| e == &text) {
            entries.push(text);
        }
        Self::write(path, &title, &entries)?;
        Ok(())
    }

    pub fn remove(&self, needle: &str, global: bool) -> anyhow::Result<usize> {
        let _g = self.lock.lock().unwrap();
        let mut removed = 0;
        let targets: Vec<(&PathBuf, String)> = if global {
            vec![(&self.global_file, "nar global memory".into())]
        } else {
            vec![
                (&self.project_file, format!("nar memory for {}", self.root.display())),
                (&self.global_file, "nar global memory".into()),
            ]
        };
        let needle = needle.trim().to_lowercase();
        for (path, title) in targets {
            let entries = Self::entries(path);
            let kept: Vec<String> = entries.iter().filter(|e| needle.is_empty() || !e.to_lowercase().contains(&needle)).cloned().collect();
            if kept.len() != entries.len() {
                removed += entries.len() - kept.len();
                Self::write(path, &title, &kept)?;
            }
        }
        Ok(removed)
    }

    pub fn render(&self) -> String {
        let mut s = String::new();
        let g = self.global_entries();
        let p = self.project_entries();
        if !g.is_empty() {
            s.push_str("Global:\n");
            for e in g {
                s.push_str(&format!("- {e}\n"));
            }
        }
        if !p.is_empty() {
            s.push_str("This project:\n");
            for e in p {
                s.push_str(&format!("- {e}\n"));
            }
        }
        s
    }

    /// The memory section of the system prompt.
    pub fn prompt_section(&self) -> String {
        let mut s = String::new();
        for f in self.instruction_files() {
            if let Ok(text) = std::fs::read_to_string(&f) {
                let text: String = text.chars().take(40_000).collect();
                s.push_str(&format!("# Instructions from {}\n{}\n\n", config::short_path(&f), text.trim()));
            }
        }
        let notes = self.render();
        if !notes.is_empty() {
            s.push_str("# Memory\nThings the user asked you to remember (use the `memory` tool to add or remove):\n");
            s.push_str(&notes);
            s.push('\n');
        }
        s
    }
}
