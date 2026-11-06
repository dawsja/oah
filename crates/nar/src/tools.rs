//! Built-in tools exposed to the model as Responses API function tools.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use crate::config::Mode;
use crate::memory::Memory;
use crate::skills::Skills;

const MAX_OUTPUT_CHARS: usize = 30_000;
const MAX_READ_LINES: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Risk {
    /// Bookkeeping tools that never need permission.
    Internal,
    /// Reads inside the workspace.
    Read,
    /// Reads outside the workspace.
    ReadOutside,
    /// File modifications.
    Write,
    /// Shell commands judged read-only.
    SafeExec,
    /// Other shell commands.
    Exec,
    /// MCP tools.
    External,
}

#[derive(Clone, Debug, Default)]
pub struct ToolOutcome {
    /// Text returned to the model.
    pub output: String,
    /// One-line summary for the transcript.
    pub summary: String,
    /// Preview lines for the transcript (lines starting with '+'/'-' render as diff).
    pub detail: Vec<String>,
    pub ok: bool,
}

impl ToolOutcome {
    fn ok(output: String, summary: String) -> Self {
        ToolOutcome { output, summary, detail: vec![], ok: true }
    }
    fn err(msg: impl Into<String>) -> Self {
        let m = msg.into();
        ToolOutcome { output: format!("Error: {m}"), summary: m, detail: vec![], ok: false }
    }
}

#[derive(Clone)]
pub struct ToolCtx {
    pub cwd: PathBuf,
    pub cancel: CancellationToken,
    pub skills: Arc<Skills>,
    pub memory: Arc<Memory>,
}

fn func(name: &str, description: &str, params: Value) -> Value {
    json!({ "type": "function", "name": name, "description": description, "parameters": params, "strict": false })
}

pub fn builtin_specs(mode: Mode, goal_active: bool, has_skills: bool) -> Vec<Value> {
    let mut v = vec![
        func(
            "read_file",
            "Read a text file. Returns numbered lines. Use offset/limit (1-based line numbers) for large files.",
            json!({"type":"object","properties":{
                "path":{"type":"string","description":"File path, absolute or relative to the workspace"},
                "offset":{"type":"integer","description":"First line to read (1-based)"},
                "limit":{"type":"integer","description":"Maximum lines to read (default 2000)"}},
                "required":["path"]}),
        ),
        func(
            "list_dir",
            "List a directory (non-recursive). Directories end with '/'.",
            json!({"type":"object","properties":{"path":{"type":"string","description":"Directory (default: workspace root)"}}}),
        ),
        func(
            "glob",
            "Find files by glob pattern (e.g. 'src/**/*.rs'), respecting .gitignore. Returns up to 500 paths.",
            json!({"type":"object","properties":{
                "pattern":{"type":"string"},
                "path":{"type":"string","description":"Directory to search (default: workspace root)"}},
                "required":["pattern"]}),
        ),
        func(
            "grep",
            "Search file contents with a regular expression, respecting .gitignore. Returns 'path:line: text' matches (max 300).",
            json!({"type":"object","properties":{
                "pattern":{"type":"string","description":"Rust regex syntax"},
                "path":{"type":"string","description":"File or directory (default: workspace root)"},
                "glob":{"type":"string","description":"Only search files matching this glob, e.g. '*.ts'"},
                "case_insensitive":{"type":"boolean"}},
                "required":["pattern"]}),
        ),
        func(
            "todo_write",
            "Replace the visible task list for the current work. Use it for multi-step tasks; keep exactly one item in_progress.",
            json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"object","properties":{
                "text":{"type":"string"},
                "status":{"type":"string","enum":["pending","in_progress","completed"]}},
                "required":["text","status"]}}},"required":["items"]}),
        ),
        func(
            "memory",
            "Persistent memory across sessions. 'add' saves a durable fact or preference (project scope by default, or global), 'remove' deletes entries containing text, 'list' shows all.",
            json!({"type":"object","properties":{
                "action":{"type":"string","enum":["add","remove","list"]},
                "text":{"type":"string"},
                "scope":{"type":"string","enum":["project","global"]}},
                "required":["action"]}),
        ),
    ];
    if has_skills {
        v.push(func(
            "load_skill",
            "Load the full instructions of an available skill by name before using it.",
            json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}),
        ));
    }
    if mode == Mode::Build {
        v.push(func(
            "write_file",
            "Create or overwrite a file with the given content. Parent directories are created.",
            json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
        ));
        v.push(func(
            "edit_file",
            "Replace an exact string in a file. old_string must match exactly once unless replace_all is true. Read the file first.",
            json!({"type":"object","properties":{
                "path":{"type":"string"},
                "old_string":{"type":"string"},
                "new_string":{"type":"string"},
                "replace_all":{"type":"boolean"}},
                "required":["path","old_string","new_string"]}),
        ));
        v.push(func(
            "bash",
            "Run a shell command in the workspace with bash. Returns combined stdout/stderr and the exit code. Avoid interactive commands.",
            json!({"type":"object","properties":{
                "command":{"type":"string"},
                "timeout_ms":{"type":"integer","description":"Default 120000, max 600000"}},
                "required":["command"]}),
        ));
    } else {
        v.push(func(
            "bash",
            "Run a READ-ONLY shell command (plan mode): inspection only, e.g. git log, ls, cat. Commands that modify state are refused.",
            json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
        ));
    }
    if goal_active {
        v.push(func(
            "goal_complete",
            "Call ONLY when the active goal is fully achieved and verified. Provide a concise summary of what was done and how it was verified.",
            json!({"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"]}),
        ));
    }
    v
}

pub fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "read_file" | "list_dir" | "glob" | "grep" | "write_file" | "edit_file" | "bash" | "todo_write" | "memory" | "load_skill" | "goal_complete"
    )
}

fn arg<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(|v| v.as_str()).unwrap_or("")
}

pub fn resolve(cwd: &Path, p: &str) -> PathBuf {
    let p = p.trim();
    let expanded = if let Some(rest) = p.strip_prefix("~/") {
        dirs::home_dir().unwrap_or_default().join(rest)
    } else {
        PathBuf::from(p)
    };
    let joined = if expanded.is_absolute() { expanded } else { cwd.join(expanded) };
    normalize(&joined)
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn inside(cwd: &Path, p: &Path) -> bool {
    p.starts_with(cwd)
}

fn rel(cwd: &Path, p: &Path) -> String {
    p.strip_prefix(cwd).map(|r| r.display().to_string()).unwrap_or_else(|_| crate::config::short_path(p))
}

/// Commands that only inspect state (used by `auto` permission and plan mode).
pub fn is_safe_command(cmd: &str, extra: &[String]) -> bool {
    let c = cmd.trim();
    if c.is_empty() {
        return false;
    }
    // Reject anything that redirects into files or chains into unknown commands.
    if c.contains('>') || c.contains("`") || c.contains("$(") {
        return false;
    }
    let segments: Vec<&str> = c.split(['|', ';', '&']).map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    const SAFE: &[&str] = &[
        "ls", "pwd", "cat", "head", "tail", "wc", "echo", "grep", "rg", "fd", "find", "tree", "stat", "file", "which", "whoami",
        "date", "uname", "env", "printenv", "du", "df", "sort", "uniq", "cut", "tr", "diff", "less", "more", "basename", "dirname",
        "realpath", "readlink", "type", "true", "nl", "column", "jq", "sha256sum", "md5sum",
    ];
    const SAFE_GIT: &[&str] = &["status", "diff", "log", "show", "branch", "remote", "rev-parse", "ls-files", "blame", "describe", "tag", "config"];
    const SAFE_CARGO: &[&str] = &["check", "metadata", "tree", "--version", "version"];
    segments.iter().all(|seg| {
        if extra.iter().any(|e| seg.starts_with(e.as_str())) {
            return true;
        }
        let mut words = seg.split_whitespace();
        let first = words.next().unwrap_or("");
        let second = words.next().unwrap_or("");
        if first == "find" && (seg.contains("-exec") || seg.contains("-delete") || seg.contains("-ok")) {
            return false;
        }
        if first == "git" {
            return SAFE_GIT.contains(&second) && !(second == "config" && !seg.contains("--get")) && !seg.contains("branch -d");
        }
        if first == "cargo" {
            return SAFE_CARGO.contains(&second);
        }
        SAFE.contains(&first)
    })
}

pub fn risk(name: &str, args: &Value, cwd: &Path, extra_safe: &[String]) -> Risk {
    match name {
        "todo_write" | "memory" | "load_skill" | "goal_complete" => Risk::Internal,
        "read_file" | "list_dir" | "glob" | "grep" => {
            let p = arg(args, "path");
            if p.is_empty() || inside(cwd, &resolve(cwd, p)) {
                Risk::Read
            } else {
                Risk::ReadOutside
            }
        }
        "write_file" | "edit_file" => Risk::Write,
        "bash" => {
            if is_safe_command(arg(args, "command"), extra_safe) {
                Risk::SafeExec
            } else {
                Risk::Exec
            }
        }
        _ => Risk::External,
    }
}

/// Short human title for a call, e.g. `Bash(cargo test)`.
pub fn title(name: &str, args: &Value, cwd: &Path) -> String {
    let path = || {
        let p = arg(args, "path");
        if p.is_empty() {
            ".".to_string()
        } else {
            rel(cwd, &resolve(cwd, p))
        }
    };
    match name {
        "read_file" => format!("Read {}", path()),
        "list_dir" => format!("List {}", path()),
        "glob" => format!("Glob {}", arg(args, "pattern")),
        "grep" => format!("Grep \"{}\"{}", arg(args, "pattern"), if arg(args, "path").is_empty() { String::new() } else { format!(" in {}", path()) }),
        "write_file" => format!("Write {}", path()),
        "edit_file" => format!("Edit {}", path()),
        "bash" => format!("Bash {}", one_line(arg(args, "command"), 120)),
        "todo_write" => "Update todos".into(),
        "memory" => format!("Memory {}", arg(args, "action")),
        "load_skill" => format!("Skill {}", arg(args, "name")),
        "goal_complete" => "Goal complete".into(),
        other => {
            let compact = if args.as_object().map(|o| o.is_empty()).unwrap_or(true) { String::new() } else { args.to_string() };
            if let Some(rest) = other.strip_prefix("mcp__") {
                let (server, tool) = rest.split_once("__").unwrap_or((rest, ""));
                format!("MCP {server}/{tool} {}", one_line(&compact, 100)).trim_end().to_string()
            } else {
                format!("{} {}", other, one_line(&compact, 100)).trim_end().to_string()
            }
        }
    }
}

pub fn one_line(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        let t: String = flat.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    } else {
        flat
    }
}

fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_CHARS {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX_OUTPUT_CHARS / 2).collect();
    let tail_chars: Vec<char> = s.chars().rev().take(MAX_OUTPUT_CHARS / 2).collect();
    let tail: String = tail_chars.into_iter().rev().collect();
    let omitted = s.len() - head.len() - tail.len();
    format!("{head}\n\n… [{omitted} bytes omitted] …\n\n{tail}")
}

fn preview(s: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = s.lines().collect();
    let mut out: Vec<String> = lines.iter().take(n).map(|l| one_line_keep(l, 200)).collect();
    if lines.len() > n {
        out.push(format!("… +{} lines", lines.len() - n));
    }
    out
}

fn one_line_keep(s: &str, max: usize) -> String {
    let s = s.replace('\t', "    ");
    if s.chars().count() > max {
        let t: String = s.chars().take(max - 1).collect();
        format!("{t}…")
    } else {
        s
    }
}

pub async fn run_builtin(name: &str, args: &Value, ctx: &ToolCtx, mode: Mode) -> ToolOutcome {
    match name {
        "read_file" => read_file(args, ctx),
        "list_dir" => list_dir(args, ctx),
        "glob" => glob_files(args, ctx),
        "grep" => grep(args, ctx),
        "write_file" if mode == Mode::Build => write_file(args, ctx),
        "edit_file" if mode == Mode::Build => edit_file(args, ctx),
        "write_file" | "edit_file" => ToolOutcome::err("plan mode is read-only; switch to build mode (shift+tab) to edit"),
        "bash" => {
            if mode == Mode::Plan && !is_safe_command(arg(args, "command"), &[]) {
                return ToolOutcome::err("plan mode only allows read-only commands");
            }
            bash(args, ctx).await
        }
        "load_skill" => match ctx.skills.load(arg(args, "name")) {
            Some(body) => ToolOutcome::ok(body, format!("loaded skill {}", arg(args, "name"))),
            None => ToolOutcome::err(format!("no skill named '{}'", arg(args, "name"))),
        },
        "memory" => {
            let action = arg(args, "action");
            let global = arg(args, "scope") == "global";
            match action {
                "add" => match ctx.memory.add(arg(args, "text"), global) {
                    Ok(()) => ToolOutcome::ok("Saved.".into(), format!("remembered: {}", one_line(arg(args, "text"), 80))),
                    Err(e) => ToolOutcome::err(e.to_string()),
                },
                "remove" => match ctx.memory.remove(arg(args, "text"), global) {
                    Ok(n) => ToolOutcome::ok(format!("Removed {n} entries."), format!("forgot {n} entries")),
                    Err(e) => ToolOutcome::err(e.to_string()),
                },
                _ => {
                    let all = ctx.memory.render();
                    ToolOutcome::ok(if all.is_empty() { "(memory is empty)".into() } else { all }, "listed memory".into())
                }
            }
        }
        "todo_write" => {
            let items = args.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let lines: Vec<String> = items
                .iter()
                .map(|i| {
                    let mark = match i["status"].as_str() {
                        Some("completed") => "☒",
                        Some("in_progress") => "◐",
                        _ => "☐",
                    };
                    format!("{mark} {}", i["text"].as_str().unwrap_or(""))
                })
                .collect();
            let done = items.iter().filter(|i| i["status"] == "completed").count();
            ToolOutcome {
                output: "Todo list updated.".into(),
                summary: format!("{done}/{} done", items.len()),
                detail: lines,
                ok: true,
            }
        }
        "goal_complete" => ToolOutcome::ok("Goal marked complete.".into(), one_line(arg(args, "summary"), 200)),
        _ => ToolOutcome::err(format!("unknown tool {name}")),
    }
}

fn read_file(args: &Value, ctx: &ToolCtx) -> ToolOutcome {
    let path = resolve(&ctx.cwd, arg(args, "path"));
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return ToolOutcome::err(format!("{}: {e}", rel(&ctx.cwd, &path))),
    };
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return ToolOutcome::err(format!("{} looks like a binary file ({} bytes)", rel(&ctx.cwd, &path), bytes.len()));
    }
    let text = String::from_utf8_lossy(&bytes);
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(MAX_READ_LINES).min(MAX_READ_LINES);
    let total = text.lines().count();
    let mut out = String::new();
    let mut shown = 0;
    for (i, line) in text.lines().enumerate().skip(offset - 1).take(limit) {
        let line = if line.len() > 2000 { &line[..line.char_indices().nth(2000).map(|c| c.0).unwrap_or(line.len())] } else { line };
        out.push_str(&format!("{:>6}\t{}\n", i + 1, line));
        shown += 1;
    }
    if offset - 1 + shown < total {
        out.push_str(&format!("… ({} more lines; use offset={} to continue)\n", total - (offset - 1 + shown), offset + shown));
    }
    if total == 0 {
        out = "(empty file)".into();
    }
    ToolOutcome::ok(out, format!("{shown} of {total} lines"))
}

fn list_dir(args: &Value, ctx: &ToolCtx) -> ToolOutcome {
    let p = arg(args, "path");
    let path = if p.is_empty() { ctx.cwd.clone() } else { resolve(&ctx.cwd, p) };
    let rd = match std::fs::read_dir(&path) {
        Ok(r) => r,
        Err(e) => return ToolOutcome::err(format!("{}: {e}", rel(&ctx.cwd, &path))),
    };
    let mut entries: Vec<String> = rd
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                format!("{name}/")
            } else {
                name
            }
        })
        .collect();
    entries.sort();
    let n = entries.len();
    ToolOutcome::ok(entries.join("\n"), format!("{n} entries"))
}

fn walker(root: &Path) -> ignore::Walk {
    ignore::WalkBuilder::new(root).hidden(false).git_ignore(true).filter_entry(|e| e.file_name() != ".git").build()
}

fn glob_files(args: &Value, ctx: &ToolCtx) -> ToolOutcome {
    let p = arg(args, "path");
    let root = if p.is_empty() { ctx.cwd.clone() } else { resolve(&ctx.cwd, p) };
    let pattern = arg(args, "pattern");
    let matcher = match globset::GlobBuilder::new(pattern).literal_separator(false).build() {
        Ok(g) => g.compile_matcher(),
        Err(e) => return ToolOutcome::err(format!("bad glob: {e}")),
    };
    let mut out = Vec::new();
    for e in walker(&root).flatten() {
        if ctx.cancel.is_cancelled() {
            break;
        }
        if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let r = e.path().strip_prefix(&root).unwrap_or(e.path());
        if matcher.is_match(r) || matcher.is_match(e.path().file_name().unwrap_or_default()) && !pattern.contains('/') {
            out.push(rel(&ctx.cwd, e.path()));
            if out.len() >= 500 {
                break;
            }
        }
    }
    out.sort();
    let n = out.len();
    if n == 0 {
        return ToolOutcome::ok("No files matched.".into(), "no matches".into());
    }
    ToolOutcome::ok(out.join("\n"), format!("{n} files"))
}

fn grep(args: &Value, ctx: &ToolCtx) -> ToolOutcome {
    let p = arg(args, "path");
    let root = if p.is_empty() { ctx.cwd.clone() } else { resolve(&ctx.cwd, p) };
    let ci = args.get("case_insensitive").and_then(|v| v.as_bool()).unwrap_or(false);
    let re = match regex::RegexBuilder::new(arg(args, "pattern")).case_insensitive(ci).build() {
        Ok(r) => r,
        Err(e) => return ToolOutcome::err(format!("bad regex: {e}")),
    };
    let filter = {
        let g = arg(args, "glob");
        if g.is_empty() {
            None
        } else {
            globset::Glob::new(g).ok().map(|g| g.compile_matcher())
        }
    };
    let mut out = Vec::new();
    let mut files = 0;
    'outer: for e in walker(&root).flatten() {
        if ctx.cancel.is_cancelled() {
            break;
        }
        if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        if let Some(f) = &filter {
            if !f.is_match(e.path().file_name().unwrap_or_default()) && !f.is_match(e.path()) {
                continue;
            }
        }
        let Ok(bytes) = std::fs::read(e.path()) else { continue };
        if bytes.len() > 4 << 20 || bytes.iter().take(4096).any(|b| *b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut hit = false;
        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                hit = true;
                out.push(format!("{}:{}: {}", rel(&ctx.cwd, e.path()), i + 1, one_line_keep(line.trim_end(), 300)));
                if out.len() >= 300 {
                    break 'outer;
                }
            }
        }
        if hit {
            files += 1;
        }
    }
    if out.is_empty() {
        return ToolOutcome::ok("No matches.".into(), "no matches".into());
    }
    let n = out.len();
    ToolOutcome::ok(out.join("\n"), format!("{n} matches in {files} files"))
}

fn write_file(args: &Value, ctx: &ToolCtx) -> ToolOutcome {
    let path = resolve(&ctx.cwd, arg(args, "path"));
    let content = arg(args, "content");
    let existed = path.exists();
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return ToolOutcome::err(e.to_string());
        }
    }
    if let Err(e) = std::fs::write(&path, content) {
        return ToolOutcome::err(format!("{}: {e}", rel(&ctx.cwd, &path)));
    }
    let lines = content.lines().count();
    let mut o = ToolOutcome::ok(
        format!("Wrote {} ({} lines).", rel(&ctx.cwd, &path), lines),
        if existed { format!("overwrote · {lines} lines") } else { format!("created · {lines} lines") },
    );
    o.detail = if existed { diff_lines(&old, content, 12) } else { content.lines().take(8).map(|l| format!("+{l}")).collect() };
    o
}

fn edit_file(args: &Value, ctx: &ToolCtx) -> ToolOutcome {
    let path = resolve(&ctx.cwd, arg(args, "path"));
    let old_s = arg(args, "old_string");
    let new_s = arg(args, "new_string");
    let all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return ToolOutcome::err(format!("{}: {e}", rel(&ctx.cwd, &path))),
    };
    if old_s.is_empty() {
        return ToolOutcome::err("old_string is empty; use write_file to create files");
    }
    let count = text.matches(old_s).count();
    if count == 0 {
        return ToolOutcome::err(format!("old_string not found in {}", rel(&ctx.cwd, &path)));
    }
    if count > 1 && !all {
        return ToolOutcome::err(format!("old_string matches {count} times; add context or set replace_all"));
    }
    let updated = if all { text.replace(old_s, new_s) } else { text.replacen(old_s, new_s, 1) };
    if let Err(e) = std::fs::write(&path, &updated) {
        return ToolOutcome::err(e.to_string());
    }
    let added = new_s.lines().count();
    let removed = old_s.lines().count();
    let mut o = ToolOutcome::ok(
        format!("Edited {} ({count} replacement{}).", rel(&ctx.cwd, &path), if count == 1 { "" } else { "s" }),
        format!("+{added} -{removed}"),
    );
    o.detail = diff_lines(old_s, new_s, 16);
    o
}

/// What a permission prompt shows before a call runs.
pub fn permission_preview(name: &str, args: &Value) -> Vec<String> {
    match name {
        "edit_file" => diff_lines(arg(args, "old_string"), arg(args, "new_string"), 10),
        "write_file" => {
            let c = arg(args, "content");
            let mut v: Vec<String> = c.lines().take(8).map(|l| format!("+{l}")).collect();
            let n = c.lines().count();
            if n > 8 {
                v.push(format!("… +{} lines", n - 8));
            }
            v
        }
        "bash" => {
            let c = arg(args, "command");
            if c.lines().count() > 1 || c.chars().count() > 100 {
                c.lines().take(8).map(|l| format!("$ {l}")).collect()
            } else {
                vec![]
            }
        }
        n if n.starts_with("mcp__") => serde_json::to_string_pretty(args)
            .unwrap_or_default()
            .lines()
            .take(8)
            .map(String::from)
            .collect(),
        _ => vec![],
    }
}

/// A tiny line diff (common prefix/suffix trimmed) for previews.
fn diff_lines(old: &str, new: &str, max: usize) -> Vec<String> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut start = 0;
    while start < a.len() && start < b.len() && a[start] == b[start] {
        start += 1;
    }
    let mut end_a = a.len();
    let mut end_b = b.len();
    while end_a > start && end_b > start && a[end_a - 1] == b[end_b - 1] {
        end_a -= 1;
        end_b -= 1;
    }
    let mut out = Vec::new();
    for l in &a[start..end_a] {
        out.push(format!("-{l}"));
    }
    for l in &b[start..end_b] {
        out.push(format!("+{l}"));
    }
    if out.len() > max {
        let extra = out.len() - max;
        out.truncate(max);
        out.push(format!("… +{extra} lines"));
    }
    out
}

async fn bash(args: &Value, ctx: &ToolCtx) -> ToolOutcome {
    let command = arg(args, "command").to_string();
    if command.trim().is_empty() {
        return ToolOutcome::err("empty command");
    }
    let timeout = args.get("timeout_ms").and_then(|v| v.as_u64()).unwrap_or(120_000).clamp(1000, 600_000);
    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-c")
        .arg(&command)
        .current_dir(&ctx.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NAR", "1")
        .env("TERM", "dumb")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolOutcome::err(format!("spawn failed: {e}")),
    };
    let pid = child.id();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let read_all = async move {
        let mut o = Vec::new();
        let mut e = Vec::new();
        let _ = tokio::join!(stdout.read_to_end(&mut o), stderr.read_to_end(&mut e));
        (o, e)
    };
    let kill_group = || {
        #[cfg(unix)]
        if let Some(pid) = pid {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    };
    let result = tokio::select! {
        _ = ctx.cancel.cancelled() => { kill_group(); return ToolOutcome::err("interrupted"); }
        r = tokio::time::timeout(Duration::from_millis(timeout), async { let out = read_all.await; let st = child.wait().await; (out, st) }) => r,
    };
    let ((o, e), status) = match result {
        Ok(v) => v,
        Err(_) => {
            kill_group();
            return ToolOutcome::err(format!("timed out after {}s", timeout / 1000));
        }
    };
    let code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
    let mut text = String::from_utf8_lossy(&o).into_owned();
    let err = String::from_utf8_lossy(&e);
    if !err.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&err);
    }
    let text = strip_ansi(&text);
    let lines = text.lines().count();
    let body = truncate_output(text.trim_end());
    let output = if body.trim().is_empty() { format!("(no output)\nexit code: {code}") } else { format!("{body}\nexit code: {code}") };
    ToolOutcome { output, summary: format!("exit {code} · {lines} lines"), detail: preview(&text, 6), ok: code == 0 }
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(n) = chars.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
        } else if c == '\r' {
            continue;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_commands() {
        assert!(is_safe_command("ls -la", &[]));
        assert!(is_safe_command("git status && git diff", &[]));
        assert!(is_safe_command("cat a | grep x | wc -l", &[]));
        assert!(!is_safe_command("rm -rf /", &[]));
        assert!(!is_safe_command("echo hi > file", &[]));
        assert!(!is_safe_command("find . -delete", &[]));
        assert!(!is_safe_command("git push", &[]));
        assert!(!is_safe_command("ls; rm x", &[]));
        assert!(is_safe_command("npm test", &["npm test".into()]));
    }

    #[test]
    fn diff_preview() {
        let d = diff_lines("a\nb\nc", "a\nB\nc", 10);
        assert_eq!(d, vec!["-b", "+B"]);
    }

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m\r\n"), "red\n");
    }
}
