//! Drawing. Layout (top → bottom):
//!
//! ```text
//!  ◆ nar  ~/project                                   session 3f2a91c0 · ChatGPT plan
//!  ▌ You                                                        ← transcript (scrolls)
//!  ▌ refactor the parser
//!  ● Here's the plan…
//!    ⏺ Bash cargo test   ⎿ exit 0 · 34 lines
//!  ⠋ Thinking… 12s · esc to interrupt                          ← activity line
//! ╭──────────────────────────────────────────────────────────╮
//! │ › ask anything…                                           │ ← input
//! ╰──────────────────────────────────────────────────────────╯
//!  gpt-5.5 · high · auto · ◆ build          ChatGPT plan · 23k/272k ▰▰▱▱▱▱▱▱ 9%
//! ```

use opentui::{attr, Align, BorderStyle, Frame, Rgba, Terminal};

use crate::config::{self, Mode, Permission};
use crate::session::{Entry, ToolStatus};

use super::app::{App, BusyKind, InputMode, Rect};
use super::text::{self, markdown, truncate, w, wrap, wrap_plain, Line, Span};
use super::theme as t;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 10_000 {
        format!("{}k", n / 1000)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

fn draw_spans(f: &mut Frame, x: i32, y: i32, spans: &[Span], max_x: i32) -> i32 {
    let mut cx = x;
    for s in spans {
        if cx >= max_x {
            break;
        }
        let avail = (max_x - cx) as usize;
        let txt = if s.width() > avail { truncate(&s.text, avail) } else { s.text.clone() };
        f.text(cx, y, &txt, s.fg, s.bg, s.attrs, 0);
        cx += w(&txt) as i32;
    }
    cx
}

/// Draw `line` at row `y`, re-drawing [c0, c1) with the selection background.
fn draw_line(f: &mut Frame, x: i32, y: i32, width: u32, line: &Line, sel: Option<(usize, usize)>) {
    if let Some(bg) = line.bg {
        f.fill(x, y, width, 1, bg);
    }
    draw_spans(f, x, y, &line.spans, x + width as i32);
    if let Some((c0, c1)) = sel {
        use unicode_segmentation::UnicodeSegmentation;
        let mut col = 0usize;
        for s in &line.spans {
            let mut seg = String::new();
            let mut seg_start = None;
            for g in s.text.graphemes(true) {
                let gw = w(g);
                if col >= c0 && col < c1 {
                    seg_start.get_or_insert(col);
                    seg.push_str(g);
                }
                col += gw;
            }
            if let Some(st) = seg_start {
                f.text(x + st as i32, y, &seg, t::TEXT, Some(t::SEL_BG), s.attrs, 0);
            }
        }
        // Selected trailing space past the end of the line.
        let lw = line.width();
        if c1 > lw && lw < width as usize {
            let a = c0.max(lw);
            let n = (c1.min(width as usize)).saturating_sub(a);
            if n > 0 {
                f.fill(x + a as i32, y, n as u32, 1, t::SEL_BG);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Transcript → lines

fn entry_lines(e: &Entry, width: usize) -> Vec<Line> {
    let mut out = Vec::new();
    match e {
        Entry::User { text } => {
            let bar = Span::deco("▌ ", t::ACCENT);
            let style = Span::new("", t::TEXT);
            out.push(Line::new(vec![Span::deco("▌ ", t::ACCENT), Span::deco("you", t::DIM).attrs(attr::BOLD)]).with_bg(t::USER_BG));
            out.extend(wrap_plain(text, width, &style, &[bar.clone()], &[bar], Some(t::USER_BG)));
        }
        Entry::Assistant { text } => {
            let first = [Span::deco("● ", t::ACCENT)];
            let rest = [Span::deco("  ", t::ACCENT)];
            out.extend(markdown(text, width, &first, &rest));
        }
        Entry::Reasoning { text } => {
            let body = text.trim();
            if body.is_empty() {
                return out;
            }
            let style = Span::new("", t::FAINT).attrs(attr::ITALIC);
            let first = [Span::deco("∴ ", t::FAINT)];
            let rest = [Span::deco("  ", t::FAINT)];
            let mut lines = Vec::new();
            for (i, para) in body.split("\n\n").enumerate() {
                let p = para.replace("**", "");
                let f: &[Span] = if i == 0 { &first } else { &rest };
                lines.extend(wrap_plain(p.trim(), width, &style, f, &rest, None));
            }
            // Keep long thoughts compact.
            if lines.len() > 10 {
                let hidden = lines.len() - 9;
                lines.truncate(9);
                lines.push(Line::new(vec![Span::deco("  ", t::FAINT), Span::new(format!("… {hidden} more lines"), t::FAINT).attrs(attr::ITALIC)]));
            }
            out.extend(lines);
        }
        Entry::Tool { title, summary, detail, status, .. } => {
            let (glyph, color) = match status {
                ToolStatus::Pending | ToolStatus::Running => ("◌", t::YELLOW),
                ToolStatus::Ok => ("⏺", t::GREEN),
                ToolStatus::Failed => ("⏺", t::RED),
                ToolStatus::Denied => ("⊘", t::ORANGE),
            };
            let (verb, rest_title) = match title.split_once(' ') {
                Some((v, r)) => (v.to_string(), r.to_string()),
                None => (title.clone(), String::new()),
            };
            let head = vec![
                Span::new(verb, t::TEXT).attrs(attr::BOLD),
                Span::new(if rest_title.is_empty() { String::new() } else { format!(" {rest_title}") }, t::DIM),
            ];
            out.extend(wrap(&head, width, &[Span::deco(format!("{glyph} "), color)], &[Span::deco("  ", color)], None));
            if !summary.is_empty() {
                let c = match status {
                    ToolStatus::Failed => t::RED,
                    ToolStatus::Denied => t::ORANGE,
                    _ => t::DIM,
                };
                out.extend(wrap_plain(summary, width, &Span::new("", c), &[Span::deco("  ⎿ ", t::FAINT)], &[Span::deco("    ", t::FAINT)], None));
            }
            for d in detail {
                let (fg, bg) = if d.starts_with('+') {
                    (t::DIFF_ADD, Some(t::DIFF_ADD_BG))
                } else if d.starts_with('-') {
                    (t::DIFF_DEL, Some(t::DIFF_DEL_BG))
                } else if d.starts_with('…') {
                    (t::FAINT, None)
                } else {
                    (t::DIM, None)
                };
                let line = truncate(d, width.saturating_sub(4));
                let mut l = Line::new(vec![Span::deco("    ", t::FAINT), Span::new(line, fg)]);
                if let Some(bg) = bg {
                    let lw = l.width();
                    l.spans.push(Span::deco(" ".repeat(width.saturating_sub(lw)), fg).bg(bg));
                    l.spans[1].bg = Some(bg);
                }
                out.push(l);
            }
        }
        Entry::Info { text } => {
            out.extend(markdown(text, width, &[Span::deco("※ ", t::CYAN)], &[Span::deco("  ", t::CYAN)]).into_iter().map(|mut l| {
                for s in l.spans.iter_mut() {
                    if !s.deco && s.fg == t::TEXT {
                        s.fg = t::DIM;
                    }
                }
                l
            }));
        }
        Entry::Error { text } => {
            out.extend(wrap_plain(text, width, &Span::new("", t::RED), &[Span::deco("✗ ", t::RED)], &[Span::deco("  ", t::RED)], None));
        }
    }
    out
}

fn welcome_lines(app: &App, width: usize) -> Vec<Line> {
    let mut out = vec![Line::blank()];
    let logo = [
        "  ███╗   ██╗ █████╗ ██████╗ ",
        "  ████╗  ██║██╔══██╗██╔══██╗",
        "  ██╔██╗ ██║███████║██████╔╝",
        "  ██║╚██╗██║██╔══██║██╔══██╗",
        "  ██║ ╚████║██║  ██║██║  ██║",
        "  ╚═╝  ╚═══╝╚═╝  ╚═╝╚═╝  ╚═╝",
    ];
    let grad = [0x9d8cff, 0x8f95ff, 0x82a0ff, 0x76abff, 0x6cb7ff, 0x6cc7ff];
    if width >= 34 {
        for (l, c) in logo.iter().zip(grad) {
            out.push(Line::new(vec![Span::new(*l, Rgba::hex(c))]));
        }
    } else {
        out.push(Line::new(vec![Span::new("  nar", t::ACCENT).attrs(attr::BOLD)]));
    }
    out.push(Line::blank());
    out.push(Line::new(vec![
        Span::deco("  ", t::DIM),
        Span::new("an AI coding agent for your terminal", t::DIM),
        Span::new(format!("  v{}", config::VERSION), t::FAINT),
    ]));
    out.push(Line::new(vec![Span::deco("  ", t::DIM), Span::new(config::short_path(&app.cwd), t::FAINT)]));
    out.push(Line::blank());
    let signed_in = app.auth_status.method != crate::auth::AuthMethod::None;
    let tips: Vec<(&str, &str)> = if signed_in {
        vec![
            ("/model", "pick a model"),
            ("/effort", "reasoning depth"),
            ("shift+tab", "plan ↔ build"),
            ("/goal", "work until done"),
            ("/help", "keys & commands"),
        ]
    } else {
        vec![("/auth", "continue with ChatGPT or add an API key"), ("/help", "keys & commands")]
    };
    for (k, d) in tips {
        out.push(Line::new(vec![Span::deco("  ", t::DIM), Span::new(format!("{k:<11}"), t::ACCENT), Span::new(d, t::DIM)]));
    }
    out
}

fn todo_lines(app: &App, width: usize) -> Vec<Line> {
    let todos = &app.session.todos;
    if todos.is_empty() || todos.iter().all(|(_, s)| s == "completed") {
        return vec![];
    }
    let done = todos.iter().filter(|(_, s)| s == "completed").count();
    let mut out = vec![Line::new(vec![
        Span::deco("☰ ", t::CYAN),
        Span::new("Tasks", t::TEXT).attrs(attr::BOLD),
        Span::new(format!("  {done}/{}", todos.len()), t::DIM),
    ])];
    for (text, status) in todos {
        let (mark, fg, a) = match status.as_str() {
            "completed" => ("☒ ", t::FAINT, attr::STRIKETHROUGH),
            "in_progress" => ("◐ ", t::YELLOW, attr::BOLD),
            _ => ("☐ ", t::DIM, 0),
        };
        out.extend(wrap(&[Span::new(text.clone(), fg).attrs(a)], width, &[Span::deco(format!("  {mark}"), fg)], &[Span::deco("    ", fg)], None));
    }
    out
}

impl App {
    fn rebuild_flat(&mut self, width: usize) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.revs.iter().copied().max().unwrap_or(0).hash(&mut h);
        self.revs.len().hash(&mut h);
        self.session.todos.hash(&mut h);
        (self.auth_status.method as u8).hash(&mut h);
        let key = (h.finish(), width);
        if self.flat_key == key && !self.flat.is_empty() {
            return;
        }
        self.flat_key = key;
        let mut flat = Vec::new();
        if self.session.transcript.is_empty() {
            flat = welcome_lines(self, width);
        }
        for i in 0..self.session.transcript.len() {
            let rev = self.revs[i];
            let fresh = matches!(&self.cache[i], Some((r, wd, _)) if *r == rev && *wd == width);
            if !fresh {
                let lines = entry_lines(&self.session.transcript[i], width);
                self.cache[i] = Some((rev, width, lines));
            }
            let lines = &self.cache[i].as_ref().unwrap().2;
            if lines.is_empty() {
                continue;
            }
            let compact = matches!(
                (&self.session.transcript[i], i.checked_sub(1).map(|p| &self.session.transcript[p])),
                (Entry::Tool { .. }, Some(Entry::Tool { .. }))
            );
            if !flat.is_empty() && !compact {
                flat.push(Line::blank());
            }
            flat.extend(lines.iter().cloned());
        }
        let todo = todo_lines(self, width);
        if !todo.is_empty() {
            flat.push(Line::blank());
            flat.extend(todo);
        }
        flat.push(Line::blank());
        self.flat = flat;
    }
}

// ---------------------------------------------------------------------------
// Frame

pub fn render(app: &mut App, term: &mut Terminal) {
    let width = term.width() as i32;
    let height = term.height() as i32;
    app.screen_w = term.width();

    // Input sizing.
    let inner_w = (width - 6).max(4) as usize;
    let rows = app.editor.rows(inner_w);
    let input_rows = rows.len().clamp(1, 10) as i32;
    let perm_h = if let Some(p) = &app.permission {
        let lines = wrap_plain(&p.title, (width - 6).max(10) as usize, &Span::new("", t::TEXT), &[], &[], None).len().min(6);
        lines as i32 + 4
    } else {
        0
    };
    let input_h = if app.permission.is_some() { perm_h } else { input_rows + 2 };
    let status_y = height - 1;
    let input_y = status_y - input_h;
    let activity_y = input_y - 1;
    let header_h = 1;
    let chat_y = header_h;
    let chat_h = (activity_y - chat_y).max(1);
    let chat_x = 1;
    let chat_w = (width - 3).max(10) as u32; // leave room for the scrollbar

    app.chat_rect = Rect { x: chat_x, y: chat_y, w: chat_w, h: chat_h as u32 };
    app.rebuild_flat(chat_w as usize);
    let total = app.flat.len();
    let max_scroll = total.saturating_sub(chat_h as usize);
    if app.scroll > max_scroll {
        app.scroll = max_scroll;
    }
    let top = total.saturating_sub(chat_h as usize + app.scroll);
    app.chat_top = top;

    let palette = app.palette_items();
    let mut cursor: Option<(u16, u16)> = None;
    {
        let mut f = term.frame();
        f.clear(t::BG);

        // Header.
        f.fill(0, 0, width as u32, 1, t::PANEL);
        f.text(0, 0, " ◆ nar ", Rgba::hex(0x0f1117), Some(t::ACCENT), attr::BOLD, 0);
        let cwd = config::short_path(&app.cwd);
        f.text(8, 0, &cwd, t::DIM, Some(t::PANEL), 0, (width / 2).max(10) as u32);
        let mut right = format!("session {}", &app.session.id[..8.min(app.session.id.len())]);
        if let Some(g) = &app.session.goal {
            if !g.done {
                right = format!("◎ goal · turn {} · {right}", g.turns);
            }
        }
        let rx = width - w(&right) as i32 - 1;
        if rx > 8 + w(&cwd) as i32 + 2 {
            let color = if app.session.goal.as_ref().map(|g| !g.done).unwrap_or(false) { t::YELLOW } else { t::FAINT };
            f.text(rx, 0, &right, color, Some(t::PANEL), 0, 0);
        }

        // Transcript.
        for row in 0..chat_h {
            let idx = top + row as usize;
            let Some(line) = app.flat.get(idx) else { break };
            let sel = app.selection.and_then(|s| s.cols(idx, line.width()));
            draw_line(&mut f, chat_x, chat_y + row, chat_w, line, sel);
        }
        // Scrollbar.
        if total > chat_h as usize {
            let track_h = chat_h as f64;
            let thumb_h = ((chat_h as f64 / total as f64) * track_h).max(1.0);
            let thumb_top = (top as f64 / total as f64) * track_h;
            for r in 0..chat_h {
                let on = (r as f64) >= thumb_top.floor() && (r as f64) < (thumb_top + thumb_h).ceil();
                f.text(width - 1, chat_y + r, if on { "┃" } else { "│" }, if on { t::BORDER_HI } else { t::PANEL_HI }, None, 0, 0);
            }
        }
        if app.scroll > 0 {
            let label = format!(" ↓ {} more · end ", app.scroll);
            let lx = (width - w(&label) as i32) / 2;
            f.text(lx, activity_y - 1, &label, t::TEXT, Some(t::BORDER_HI), 0, 0);
        }

        // Activity line.
        draw_activity(app, &mut f, activity_y, width);

        // Input / permission.
        if let Some(p) = &app.permission {
            let bw = (width - 2).max(10) as u32;
            f.boxed(
                1,
                input_y,
                bw,
                input_h as u32,
                BorderStyle::Rounded,
                t::YELLOW,
                Some(t::PANEL),
                Some((&format!(" Permission · {} ", p.kind), Align::Left, t::YELLOW)),
                None,
            );
            let lines = wrap_plain(&p.title, (bw - 4) as usize, &Span::new("", t::TEXT), &[], &[], None);
            for (i, l) in lines.iter().take(6).enumerate() {
                draw_spans(&mut f, 3, input_y + 1 + i as i32, &l.spans, 1 + bw as i32 - 2);
            }
            let oy = input_y + input_h - 2;
            let opts = [("y", "allow once"), ("a", "always allow"), ("n", "deny")];
            let mut x = 3;
            for (k, label) in opts {
                f.text(x, oy, &format!(" {k} "), Rgba::hex(0x0f1117), Some(t::YELLOW), attr::BOLD, 0);
                x += 4;
                f.text(x, oy, label, t::TEXT, None, 0, 0);
                x += w(label) as i32 + 3;
            }
            f.text(x, oy, "esc deny", t::FAINT, None, 0, 0);
        } else {
            let mode_color = match app.settings.mode {
                Mode::Plan => t::PLAN,
                Mode::Build => t::BORDER_HI,
            };
            let border = if app.input_mode == InputMode::ApiKey { t::YELLOW } else { mode_color };
            let title = if app.input_mode == InputMode::ApiKey { Some((" OpenAI API key ", Align::Left, t::YELLOW)) } else { None };
            f.boxed(1, input_y, (width - 2).max(4) as u32, input_h as u32, BorderStyle::Rounded, border, Some(t::BG), title, None);
            let px = 3;
            let first = rows.len().saturating_sub(input_rows as usize);
            let (crow, ccol) = app.editor.cursor_pos(inner_w);
            let first = if crow < first { crow } else if crow >= first + input_rows as usize { crow + 1 - input_rows as usize } else { first };
            let prompt_color = if app.settings.mode == Mode::Plan { t::PLAN } else { t::ACCENT };
            f.text(px, input_y + 1, "›", prompt_color, None, attr::BOLD, 0);
            if app.editor.is_empty() {
                let ph = match app.input_mode {
                    InputMode::ApiKey => "paste key, enter to save · esc to cancel",
                    InputMode::Prompt if app.busy.is_some() => "type to queue a message · esc to interrupt",
                    InputMode::Prompt if app.settings.mode == Mode::Plan => "describe what to plan… (shift+tab for build mode)",
                    InputMode::Prompt => "ask anything · / for commands · shift+enter for newline",
                };
                f.text(px + 2, input_y + 1, ph, t::FAINT, None, 0, inner_w as u32);
            }
            for (i, r) in rows.iter().enumerate().skip(first).take(input_rows as usize) {
                let s = app.editor.display_slice(*r);
                let color = if app.editor.text.starts_with('/') && i == 0 { t::CYAN } else { t::TEXT };
                f.text(px + 2, input_y + 1 + (i - first) as i32, &s, color, None, 0, 0);
            }
            if app.picker.is_none() {
                cursor = Some(((px + 2 + ccol as i32) as u16, (input_y + 1 + (crow - first) as i32) as u16));
            }
        }

        // Status bar.
        draw_status(app, &mut f, status_y, width);

        // Overlays.
        if !palette.is_empty() {
            draw_palette(app, &mut f, &palette, input_y, width);
        }
        if app.picker.is_some() {
            draw_picker(app, &mut f, width, height);
        }
    }
    term.set_cursor(cursor);
}

fn draw_activity(app: &App, f: &mut Frame, y: i32, width: i32) {
    let mut x = 2;
    if let Some(b) = &app.busy {
        let spin = SPINNER[(app.tick as usize / 2) % SPINNER.len()];
        let color = match b.kind {
            BusyKind::Compact => t::CYAN,
            BusyKind::Login => t::YELLOW,
            BusyKind::Turn => t::ACCENT,
        };
        f.text(x, y, spin, color, None, attr::BOLD, 0);
        x += 2;
        let label = if app.activity.is_empty() { "Working".to_string() } else { app.activity.clone() };
        let dots = ["", ".", "..", "..."][(app.tick as usize / 6) % 4];
        let label = format!("{label}{dots}");
        f.text(x, y, &label, t::TEXT, None, 0, (width - 30).max(10) as u32);
        x += w(&label).min((width - 30).max(10) as usize) as i32 + 1;
        let secs = b.started.elapsed().as_secs();
        let el = if secs >= 60 { format!("{}m{:02}s", secs / 60, secs % 60) } else { format!("{secs}s") };
        f.text(x, y, &format!("{el} · esc to interrupt"), t::FAINT, None, 0, 0);
    } else if let Some((msg, _, err)) = &app.toast {
        f.text(x, y, if *err { "✗" } else { "✓" }, if *err { t::RED } else { t::GREEN }, None, attr::BOLD, 0);
        f.text(x + 2, y, msg, if *err { t::RED } else { t::DIM }, None, 0, (width - 6).max(4) as u32);
        return;
    }
    if let Some((msg, _, err)) = &app.toast {
        if app.busy.is_some() {
            let tw = w(msg) as i32;
            let tx = width - tw - 2;
            if tx > x + 4 {
                f.text(tx, y, msg, if *err { t::RED } else { t::GREEN }, None, 0, 0);
            }
        }
    }
}

fn draw_status(app: &App, f: &mut Frame, y: i32, width: i32) {
    f.fill(0, y, width as u32, 1, t::BG);
    let mut x = 2;
    let sep = |f: &mut Frame, x: &mut i32| {
        f.text(*x, y, " · ", t::FAINT, None, 0, 0);
        *x += 3;
    };
    let model = &app.settings.model;
    f.text(x, y, model, t::TEXT, None, attr::BOLD, 0);
    x += w(model) as i32;
    sep(f, &mut x);
    let effort = app.settings.effort.as_str();
    let ec = match app.settings.effort {
        config::Effort::Minimal | config::Effort::Low => t::DIM,
        config::Effort::Medium => t::CYAN,
        config::Effort::High => t::ACCENT,
        config::Effort::Xhigh => t::ORANGE,
    };
    f.text(x, y, effort, ec, None, 0, 0);
    x += w(effort) as i32;
    sep(f, &mut x);
    let perm = app.settings.permission.as_str();
    let pc = match app.settings.permission {
        Permission::Auto => t::GREEN,
        Permission::AlwaysAsk => t::YELLOW,
        Permission::AlwaysAllow => t::RED,
    };
    f.text(x, y, perm, pc, None, 0, 0);
    x += w(perm) as i32;
    sep(f, &mut x);
    let (mode, mc) = match app.settings.mode {
        Mode::Plan => ("◇ plan", t::PLAN),
        Mode::Build => ("◆ build", t::BUILD),
    };
    f.text(x, y, mode, mc, None, attr::BOLD, 0);
    x += w(mode) as i32;
    f.text(x + 1, y, "(shift+tab)", t::FAINT, None, 0, 0);
    let left_end = x + 12;

    // Right: auth + context meter.
    let window = app.context_window();
    let used = app.session.context_tokens;
    let pct = if window > 0 { ((used as f64 / window as f64) * 100.0).round().min(999.0) as u64 } else { 0 };
    let bar_n = 10usize;
    let filled = ((pct as usize * bar_n) + 99) / 100;
    let filled = filled.min(bar_n);
    let meter_color = if pct >= 85 {
        t::RED
    } else if pct >= 60 {
        t::YELLOW
    } else {
        t::GREEN
    };
    let ctx_text = format!("{} / {} ", fmt_tokens(used), fmt_tokens(window));
    let pct_text = format!(" {pct}%");
    let auth_label = match app.auth_status.method {
        crate::auth::AuthMethod::ChatGpt if app.auth_status.plan_usage => "Using ChatGPT plan".to_string(),
        crate::auth::AuthMethod::None => "not signed in · /auth".to_string(),
        _ => app.auth_status.label.clone(),
    };
    let total_w = w(&auth_label) + 3 + w(&ctx_text) + bar_n + w(&pct_text) + 2;
    let mut rx = width - total_w as i32;
    if rx <= left_end {
        // Narrow: only the meter.
        rx = width - (w(&ctx_text) + bar_n + w(&pct_text) + 2) as i32;
        if rx <= left_end {
            return;
        }
    } else {
        let ac = if app.auth_status.method == crate::auth::AuthMethod::None { t::YELLOW } else { t::DIM };
        f.text(rx, y, &auth_label, ac, None, 0, 0);
        rx += w(&auth_label) as i32;
        f.text(rx, y, " · ", t::FAINT, None, 0, 0);
        rx += 3;
    }
    f.text(rx, y, &ctx_text, t::DIM, None, 0, 0);
    rx += w(&ctx_text) as i32;
    for i in 0..bar_n {
        let (ch, c) = if i < filled { ("▰", meter_color) } else { ("▱", t::FAINT) };
        f.text(rx + i as i32, y, ch, c, None, 0, 0);
    }
    rx += bar_n as i32;
    f.text(rx, y, &pct_text, meter_color, None, attr::BOLD, 0);
}

fn draw_palette(app: &App, f: &mut Frame, items: &[(String, String, String)], input_y: i32, width: i32) {
    let n = items.len().min(10);
    let h = n as i32 + 2;
    let y = input_y - h;
    let bw = (width - 2).max(20);
    f.boxed(1, y, bw as u32, h as u32, BorderStyle::Rounded, t::BORDER_HI, Some(t::PANEL), None, None);
    let sel = app.palette_idx.min(items.len().saturating_sub(1));
    let start = if sel >= n { sel + 1 - n } else { 0 };
    let name_w = items.iter().map(|(n, a, _)| w(n) + if a.is_empty() { 0 } else { w(a) + 1 }).max().unwrap_or(10).min(46) + 3;
    for (row, (i, (name, args, desc))) in items.iter().enumerate().skip(start).take(n).enumerate() {
        let ry = y + 1 + row as i32;
        let selected = i == sel;
        if selected {
            f.fill(2, ry, (bw - 2) as u32, 1, t::PANEL_HI);
        }
        let bg = if selected { t::PANEL_HI } else { t::PANEL };
        f.text(3, ry, &format!("/{name}"), if selected { t::ACCENT } else { t::TEXT }, Some(bg), if selected { attr::BOLD } else { 0 }, 0);
        if !args.is_empty() {
            f.text(4 + w(name) as i32 + 1, ry, args, t::FAINT, Some(bg), 0, (name_w.saturating_sub(w(name) + 2)) as u32);
        }
        f.text(3 + name_w as i32 + 1, ry, desc, t::DIM, Some(bg), 0, (bw - name_w as i32 - 6).max(4) as u32);
    }
    f.text(bw - 22, y + h - 1, " tab complete · ⏎ run ", t::FAINT, Some(t::PANEL), 0, 0);
}

fn draw_picker(app: &App, f: &mut Frame, width: i32, height: i32) {
    let Some(p) = &app.picker else { return };
    let vis = p.visible();
    let bw = (width - 8).clamp(30, 84);
    let max_rows = (height - 10).max(3) as usize;
    let rows = vis.len().clamp(1, max_rows);
    let bh = rows as i32 + 4;
    let bx = (width - bw) / 2;
    let by = ((height - bh) / 2).max(1);
    f.boxed(
        bx,
        by,
        bw as u32,
        bh as u32,
        BorderStyle::Rounded,
        t::ACCENT,
        Some(t::PANEL),
        Some((&format!(" {} ", p.title), Align::Left, t::ACCENT)),
        Some((" ↑↓ select · ⏎ confirm · esc close ", Align::Right)),
    );
    let filter = if p.filter.is_empty() {
        if p.loading {
            "loading…".to_string()
        } else {
            "type to filter".to_string()
        }
    } else {
        format!("{}▏", p.filter)
    };
    f.text(bx + 2, by + 1, "⌕", t::DIM, Some(t::PANEL), 0, 0);
    f.text(bx + 4, by + 1, &filter, if p.filter.is_empty() { t::FAINT } else { t::TEXT }, Some(t::PANEL), 0, (bw - 6) as u32);
    if vis.is_empty() {
        f.text(bx + 4, by + 2, "no matches", t::FAINT, Some(t::PANEL), attr::ITALIC, 0);
        return;
    }
    let sel = p.selected.min(vis.len() - 1);
    let start = if sel >= rows { sel + 1 - rows } else { 0 };
    let label_w = vis.iter().map(|(_, i)| w(&i.label)).max().unwrap_or(10).min((bw as usize / 2).max(12));
    for (row, (i, (_, item))) in vis.iter().enumerate().skip(start).take(rows).enumerate() {
        let ry = by + 2 + row as i32;
        let selected = i == sel;
        let bg = if selected { t::PANEL_HI } else { t::PANEL };
        if selected {
            f.fill(bx + 1, ry, (bw - 2) as u32, 1, t::PANEL_HI);
            f.text(bx + 1, ry, "▌", t::ACCENT, Some(bg), 0, 0);
        }
        let mark = if item.current { "●" } else { " " };
        f.text(bx + 2, ry, mark, t::GREEN, Some(bg), 0, 0);
        f.text(bx + 4, ry, &truncate(&item.label, label_w), if selected { t::TEXT } else { t::DIM }, Some(bg), if selected { attr::BOLD } else { 0 }, 0);
        let dx = bx + 4 + label_w as i32 + 2;
        let dw = (bx + bw - 2 - dx).max(0) as u32;
        if dw > 3 {
            f.text(dx, ry, &item.detail, t::FAINT, Some(bg), 0, dw);
        }
    }
    let _ = text::w;
}
