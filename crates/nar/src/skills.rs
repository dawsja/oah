//! Agent Skills: folders with a `SKILL.md` (YAML front matter `name` and
//! `description`, then Markdown instructions). Only names and descriptions go
//! into the system prompt; the model loads a skill's body on demand with the
//! `load_skill` tool, and users can invoke one directly as `/<skill-name>`.
//!
//! Search order (first name wins): `./.nar/skills`, `./.agents/skills`,
//! `./.claude/skills`, `~/.nar/skills`, `~/.agents/skills`, `~/.claude/skills`.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub scope: &'static str,
}

#[derive(Default, Debug)]
pub struct Skills {
    pub list: Vec<Skill>,
}

pub fn parse_front_matter(text: &str) -> (Vec<(String, String)>, &str) {
    let t = text.trim_start_matches('\u{feff}');
    let Some(rest) = t.strip_prefix("---") else { return (vec![], t) };
    let rest = rest.trim_start_matches(['\r']).strip_prefix('\n').unwrap_or(rest);
    let Some(end) = rest.find("\n---") else { return (vec![], t) };
    let fm = &rest[..end];
    let body = rest[end + 4..].trim_start_matches(|c| c != '\n').trim_start_matches('\n');
    let mut out: Vec<(String, String)> = Vec::new();
    let mut lines = fm.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with(' ') || line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        let mut v = v.trim().to_string();
        if v == ">" || v == "|" || v == ">-" || v == "|-" {
            let mut parts = Vec::new();
            while let Some(next) = lines.peek() {
                if next.starts_with(' ') || next.trim().is_empty() {
                    parts.push(next.trim().to_string());
                    lines.next();
                } else {
                    break;
                }
            }
            v = parts.join(" ").trim().to_string();
        }
        let v = v.trim_matches('"').trim_matches('\'').to_string();
        out.push((k.trim().to_string(), v));
    }
    (out, body)
}

impl Skills {
    pub fn discover(cwd: &Path) -> Skills {
        let home = dirs::home_dir().unwrap_or_default();
        let roots: Vec<(PathBuf, &'static str)> = vec![
            (cwd.join(".nar/skills"), "project"),
            (cwd.join(".agents/skills"), "project"),
            (cwd.join(".claude/skills"), "project"),
            (crate::config::home_dir().join("skills"), "user"),
            (home.join(".agents/skills"), "user"),
            (home.join(".claude/skills"), "user"),
        ];
        let mut list: Vec<Skill> = Vec::new();
        for (root, scope) in roots {
            let Ok(rd) = std::fs::read_dir(&root) else { continue };
            let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            dirs.sort();
            for dir in dirs {
                let file = dir.join("SKILL.md");
                let Ok(text) = std::fs::read_to_string(&file) else { continue };
                let (fm, body) = parse_front_matter(&text);
                let get = |k: &str| fm.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
                let name = get("name").unwrap_or_else(|| dir.file_name().unwrap_or_default().to_string_lossy().into_owned());
                let description = get("description")
                    .unwrap_or_else(|| body.lines().find(|l| !l.trim().is_empty() && !l.starts_with('#')).unwrap_or("").trim().to_string());
                if list.iter().any(|s| s.name == name) {
                    continue;
                }
                list.push(Skill { name, description, path: file, scope });
            }
        }
        Skills { list }
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        let n = name.trim().trim_start_matches('/');
        self.list.iter().find(|s| s.name.eq_ignore_ascii_case(n))
    }

    /// Full skill text for the model, with its base directory and bundled files.
    pub fn load(&self, name: &str) -> Option<String> {
        let skill = self.get(name)?;
        let text = std::fs::read_to_string(&skill.path).ok()?;
        let (_, body) = parse_front_matter(&text);
        let dir = skill.path.parent().unwrap_or(Path::new("."));
        let mut files = Vec::new();
        for e in ignore::WalkBuilder::new(dir).max_depth(Some(3)).build().flatten() {
            if e.file_type().map(|t| t.is_file()).unwrap_or(false) && e.path() != skill.path {
                files.push(e.path().strip_prefix(dir).unwrap_or(e.path()).display().to_string());
                if files.len() >= 40 {
                    break;
                }
            }
        }
        let mut out = format!("# Skill: {}\nBase directory: {}\n\n{}", skill.name, dir.display(), body.trim());
        if !files.is_empty() {
            out.push_str("\n\nBundled files (relative to the base directory):\n");
            for f in files {
                out.push_str(&format!("- {f}\n"));
            }
        }
        Some(out)
    }

    pub fn prompt_section(&self) -> String {
        if self.list.is_empty() {
            return String::new();
        }
        let mut s = String::from(
            "# Skills\nSkills are packaged instructions for specific tasks. When a task matches a skill's description, call `load_skill` with its name first and follow it.\n",
        );
        for sk in &self.list {
            s.push_str(&format!("- {}: {}\n", sk.name, sk.description));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter() {
        let (fm, body) = parse_front_matter("---\nname: pdf\ndescription: \"Work with PDFs\"\n---\n# Body\ntext");
        assert_eq!(fm[0], ("name".into(), "pdf".into()));
        assert_eq!(fm[1].1, "Work with PDFs");
        assert!(body.starts_with("# Body"));
        let (fm, _) = parse_front_matter("---\nname: x\ndescription: >\n  folded\n  text\n---\nb");
        assert_eq!(fm[1].1, "folded text");
    }
}
