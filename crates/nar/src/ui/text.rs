//! Styled lines, width-aware wrapping and a lightweight Markdown renderer.

use opentui::{attr, Rgba};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::theme;

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub fg: Rgba,
    pub bg: Option<Rgba>,
    pub attrs: u32,
    /// Decorative (gutters, bullets): drawn but never copied.
    pub deco: bool,
}

impl Span {
    pub fn new(text: impl Into<String>, fg: Rgba) -> Span {
        Span { text: text.into(), fg, bg: None, attrs: 0, deco: false }
    }
    pub fn deco(text: impl Into<String>, fg: Rgba) -> Span {
        Span { text: text.into(), fg, bg: None, attrs: 0, deco: true }
    }
    pub fn attrs(mut self, a: u32) -> Span {
        self.attrs |= a;
        self
    }
    pub fn bg(mut self, bg: Rgba) -> Span {
        self.bg = Some(bg);
        self
    }
    pub fn width(&self) -> usize {
        w(&self.text)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub spans: Vec<Span>,
    /// Background for the whole row.
    pub bg: Option<Rgba>,
}

impl Line {
    pub fn new(spans: Vec<Span>) -> Line {
        Line { spans, bg: None }
    }
    pub fn blank() -> Line {
        Line::default()
    }
    pub fn with_bg(mut self, bg: Rgba) -> Line {
        self.bg = Some(bg);
        self
    }
    pub fn width(&self) -> usize {
        self.spans.iter().map(|s| s.width()).sum()
    }
    /// Copyable text between display columns [c0, c1).
    pub fn copy_text(&self, c0: usize, c1: usize) -> String {
        let mut out = String::new();
        let mut col = 0;
        for s in &self.spans {
            for g in s.text.graphemes(true) {
                let gw = w(g).max(if g == "\t" { 1 } else { 0 });
                if col >= c0 && col < c1 && !s.deco {
                    out.push_str(g);
                }
                col += gw;
            }
        }
        out
    }
}

pub fn w(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Truncate to a display width, appending an ellipsis when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if w(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for g in s.graphemes(true) {
        let gw = w(g);
        if used + gw + 1 > max {
            break;
        }
        out.push_str(g);
        used += gw;
    }
    out.push('…');
    out
}

/// Wrap styled spans into lines of at most `width` columns. `first` and `rest`
/// are prefixes (gutters) for the first and continuation lines.
pub fn wrap(spans: &[Span], width: usize, first: &[Span], rest: &[Span], line_bg: Option<Rgba>) -> Vec<Line> {
    let pw_first: usize = first.iter().map(|s| s.width()).sum();
    let pw_rest: usize = rest.iter().map(|s| s.width()).sum();
    let mut lines: Vec<Line> = Vec::new();
    let mut cur: Vec<Span> = first.to_vec();
    let mut cur_w = pw_first;
    let mut avail = width.saturating_sub(pw_first).max(1);
    let mut has_content = false;

    // Tokenize into words (with trailing spaces) preserving styles.
    struct Tok {
        text: String,
        style: usize,
        space: bool,
    }
    let mut toks: Vec<Tok> = Vec::new();
    for (i, s) in spans.iter().enumerate() {
        let mut buf = String::new();
        let mut buf_space = false;
        for g in s.text.graphemes(true) {
            let is_space = g == " ";
            if !buf.is_empty() && is_space != buf_space {
                toks.push(Tok { text: std::mem::take(&mut buf), style: i, space: buf_space });
            }
            buf_space = is_space;
            buf.push_str(if g == "\t" { "    " } else { g });
        }
        if !buf.is_empty() {
            toks.push(Tok { text: buf, style: i, space: buf_space });
        }
    }

    let push_text = |cur: &mut Vec<Span>, text: &str, style: &Span| {
        if let Some(last) = cur.last_mut() {
            if !last.deco && last.fg == style.fg && last.bg == style.bg && last.attrs == style.attrs {
                last.text.push_str(text);
                return;
            }
        }
        cur.push(Span { text: text.to_string(), ..style.clone() });
    };

    let new_line = |cur: &mut Vec<Span>, cur_w: &mut usize, avail: &mut usize, lines: &mut Vec<Line>| {
        // Drop trailing spaces.
        if let Some(last) = cur.last_mut() {
            if !last.deco {
                let t = last.text.trim_end_matches(' ').to_string();
                last.text = t;
            }
        }
        lines.push(Line { spans: std::mem::take(cur), bg: line_bg });
        *cur = rest.to_vec();
        *cur_w = pw_rest;
        *avail = width.saturating_sub(pw_rest).max(1);
    };

    for t in toks {
        let style = &spans[t.style];
        let tw = w(&t.text);
        if t.space {
            if !has_content {
                // Leading spaces (indentation) are kept on the first line only.
                if lines.is_empty() {
                    push_text(&mut cur, &t.text, style);
                    cur_w += tw;
                }
                continue;
            }
            if cur_w - (if lines.is_empty() { pw_first } else { pw_rest }) + tw <= avail {
                push_text(&mut cur, &t.text, style);
                cur_w += tw;
            } else {
                new_line(&mut cur, &mut cur_w, &mut avail, &mut lines);
                has_content = false;
            }
            continue;
        }
        let used = cur_w - if lines.is_empty() { pw_first } else { pw_rest };
        if used + tw <= avail {
            push_text(&mut cur, &t.text, style);
            cur_w += tw;
            has_content = true;
            continue;
        }
        if has_content && tw <= avail {
            new_line(&mut cur, &mut cur_w, &mut avail, &mut lines);
            push_text(&mut cur, &t.text, style);
            cur_w += tw;
            has_content = true;
            continue;
        }
        // Hard-break a long word.
        for g in t.text.graphemes(true) {
            let gw = w(g);
            let used = cur_w - if lines.is_empty() { pw_first } else { pw_rest };
            if used + gw > avail && used > 0 {
                new_line(&mut cur, &mut cur_w, &mut avail, &mut lines);
            }
            push_text(&mut cur, g, style);
            cur_w += gw;
            has_content = true;
        }
    }
    lines.push(Line { spans: cur, bg: line_bg });
    lines
}

/// Wrap plain text (respecting newlines) with one style.
pub fn wrap_plain(text: &str, width: usize, style: &Span, first: &[Span], rest: &[Span], bg: Option<Rgba>) -> Vec<Line> {
    let mut out = Vec::new();
    for (i, para) in text.split('\n').enumerate() {
        let span = Span { text: para.to_string(), ..style.clone() };
        let f = if i == 0 { first } else { rest };
        out.extend(wrap(&[span], width, f, rest, bg));
    }
    out
}

// ---------------------------------------------------------------------------
// Markdown

fn inline(text: &str, base: &Span) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut buf = String::new();
    let mut bold = false;
    let mut italic = false;
    let flush = |buf: &mut String, out: &mut Vec<Span>, bold: bool, italic: bool| {
        if !buf.is_empty() {
            let mut s = Span { text: std::mem::take(buf), ..base.clone() };
            if bold {
                s.attrs |= attr::BOLD;
                s.fg = theme::TEXT;
            }
            if italic {
                s.attrs |= attr::ITALIC;
            }
            out.push(s);
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            if let Some(end) = chars[i + 1..].iter().position(|&x| x == '`') {
                flush(&mut buf, &mut out, bold, italic);
                let code: String = chars[i + 1..i + 1 + end].iter().collect();
                out.push(Span { text: code, fg: theme::INLINE_CODE, bg: None, attrs: 0, deco: false });
                i += end + 2;
                continue;
            }
        }
        if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            flush(&mut buf, &mut out, bold, italic);
            bold = !bold;
            i += 2;
            continue;
        }
        if (c == '*' || c == '_') && !(c == '_' && i > 0 && chars[i - 1].is_alphanumeric()) {
            let next_ok = i + 1 < chars.len() && !chars[i + 1].is_whitespace();
            if italic || (next_ok && chars[i + 1..].contains(&c)) {
                flush(&mut buf, &mut out, bold, italic);
                italic = !italic;
                i += 1;
                continue;
            }
        }
        if c == '[' {
            if let Some(close) = chars[i + 1..].iter().position(|&x| x == ']') {
                let after = i + 1 + close + 1;
                if after < chars.len() && chars[after] == '(' {
                    if let Some(pclose) = chars[after + 1..].iter().position(|&x| x == ')') {
                        flush(&mut buf, &mut out, bold, italic);
                        let label: String = chars[i + 1..i + 1 + close].iter().collect();
                        let url: String = chars[after + 1..after + 1 + pclose].iter().collect();
                        out.push(Span { text: label, fg: theme::CYAN, bg: None, attrs: attr::UNDERLINE, deco: false });
                        out.push(Span { text: format!(" ({url})"), fg: theme::FAINT, bg: None, attrs: 0, deco: false });
                        i = after + 1 + pclose + 1;
                        continue;
                    }
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut out, bold, italic);
    out
}

/// Render Markdown into lines. `first`/`rest` are gutters applied to every
/// produced line (first only on the very first line).
pub fn markdown(src: &str, width: usize, first: &[Span], rest: &[Span]) -> Vec<Line> {
    let base = Span::new("", theme::TEXT);
    let mut out: Vec<Line> = Vec::new();
    let mut in_code = false;
    let gutter = |out: &Vec<Line>| if out.is_empty() { first.to_vec() } else { rest.to_vec() };
    let rest_w: usize = rest.iter().map(|s| s.width()).sum();
    let inner = width.saturating_sub(rest_w).max(8);

    for raw in src.split('\n') {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if in_code {
                in_code = false;
                let mut g = gutter(&out);
                g.push(Span::deco(" ".repeat(inner), theme::FAINT).bg(theme::CODE_BG));
                out.push(Line::new(g));
            } else {
                in_code = true;
                let code_lang = trimmed.trim_start_matches(['`', '~']).trim().to_string();
                let mut g = gutter(&out);
                let label = if code_lang.is_empty() { String::new() } else { format!(" {code_lang}") };
                let pad = inner.saturating_sub(w(&label));
                g.push(Span::deco(format!("{label}{}", " ".repeat(pad)), theme::FAINT).bg(theme::CODE_BG));
                out.push(Line::new(g));
            }
            continue;
        }
        if in_code {
            let text = line.replace('\t', "    ");
            let mut g = gutter(&out);
            g.push(Span::deco(" ", theme::FAINT).bg(theme::CODE_BG));
            let mut r = rest.to_vec();
            r.push(Span::deco(" ", theme::FAINT).bg(theme::CODE_BG));
            let style = Span::new("", theme::CODE_FG).bg(theme::CODE_BG);
            for mut l in wrap_plain(&text, width, &style, &g, &r, None) {
                let lw = l.width();
                if lw < width {
                    l.spans.push(Span::deco(" ".repeat(width - lw), theme::FAINT).bg(theme::CODE_BG));
                }
                out.push(l);
            }
            continue;
        }
        if trimmed.is_empty() {
            out.push(Line::new(gutter(&out)));
            continue;
        }
        // Horizontal rule.
        if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_') && trimmed.chars().all(|c| c == trimmed.chars().next().unwrap()) {
            let mut g = gutter(&out);
            g.push(Span::deco("─".repeat(inner.min(60)), theme::BORDER));
            out.push(Line::new(g));
            continue;
        }
        // Headings.
        if let Some(level) = heading_level(trimmed) {
            let text = trimmed[level..].trim();
            let mut s = inline(text, &base);
            for sp in &mut s {
                sp.attrs |= attr::BOLD;
                sp.fg = if level <= 2 { theme::ACCENT } else { theme::TEXT };
            }
            out.extend(wrap(&s, width, &gutter(&out), rest, None));
            continue;
        }
        // Block quote.
        if let Some(q) = trimmed.strip_prefix('>') {
            let mut f = gutter(&out);
            f.push(Span::deco("┃ ", theme::FAINT));
            let mut r = rest.to_vec();
            r.push(Span::deco("┃ ", theme::FAINT));
            let mut s = inline(q.trim_start(), &base);
            for sp in &mut s {
                sp.fg = theme::DIM;
                sp.attrs |= attr::ITALIC;
            }
            out.extend(wrap(&s, width, &f, &r, None));
            continue;
        }
        // Lists.
        let indent = line.len() - trimmed.len();
        let pad = " ".repeat(indent.min(12));
        if let Some((marker, body)) = list_item(trimmed) {
            let mut f = gutter(&out);
            f.push(Span::deco(format!("{pad}{marker} "), theme::ACCENT));
            let mut r = rest.to_vec();
            r.push(Span::deco(" ".repeat(indent.min(12) + w(&marker) + 1), theme::FAINT));
            let s = inline(body, &base);
            out.extend(wrap(&s, width, &f, &r, None));
            continue;
        }
        // Tables: keep pipe rows unwrapped where possible.
        let s = inline(line, &base);
        out.extend(wrap(&s, width, &gutter(&out), rest, None));
    }
    if in_code {
        let mut g = gutter(&out);
        g.push(Span::deco(" ".repeat(inner), theme::FAINT).bg(theme::CODE_BG));
        out.push(Line::new(g));
    }
    // Trim trailing blank lines.
    while out.len() > 1 && out.last().map(|l| l.spans.iter().all(|s| s.deco || s.text.trim().is_empty())).unwrap_or(false) {
        out.pop();
    }
    out
}

fn heading_level(s: &str) -> Option<usize> {
    let n = s.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&n) && s[n..].starts_with(' ') {
        Some(n)
    } else {
        None
    }
}

fn list_item(s: &str) -> Option<(String, &str)> {
    for m in ["- [ ] ", "- [x] ", "- [X] "] {
        if let Some(rest) = s.strip_prefix(m) {
            let mark = if m.contains('x') || m.contains('X') { "☒" } else { "☐" };
            return Some((mark.into(), rest));
        }
    }
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = s.strip_prefix(m) {
            return Some(("•".into(), rest));
        }
    }
    let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits < 4 {
        let after = &s[digits..];
        if let Some(rest) = after.strip_prefix(". ").or_else(|| after.strip_prefix(") ")) {
            return Some((format!("{}.", &s[..digits]), rest));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.text.as_str()).collect::<String>()).collect()
    }

    #[test]
    fn wraps_words() {
        let s = [Span::new("hello world this is nar", theme::TEXT)];
        let lines = wrap(&s, 11, &[], &[], None);
        assert_eq!(plain(&lines), vec!["hello world", "this is nar"]);
    }

    #[test]
    fn wraps_with_gutter() {
        let s = [Span::new("aaa bbb ccc", theme::TEXT)];
        let lines = wrap(&s, 6, &[Span::deco("> ", theme::DIM)], &[Span::deco("  ", theme::DIM)], None);
        assert_eq!(plain(&lines), vec!["> aaa", "  bbb", "  ccc"]);
        assert_eq!(lines[1].copy_text(0, 10), "bbb");
    }

    #[test]
    fn breaks_long_words() {
        let s = [Span::new("abcdefghij", theme::TEXT)];
        let lines = wrap(&s, 4, &[], &[], None);
        assert_eq!(plain(&lines), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wide_chars() {
        let s = [Span::new("世界世界", theme::TEXT)];
        let lines = wrap(&s, 5, &[], &[], None);
        assert_eq!(plain(&lines), vec!["世界", "世界"]);
    }

    #[test]
    fn markdown_basics() {
        let lines = markdown("# Title\n\n- item **bold** `code`\n\n```rust\nfn x() {}\n```", 40, &[], &[]);
        let p = plain(&lines);
        assert_eq!(p[0], "Title");
        assert!(p[2].starts_with("• item bold code"));
        assert!(p.iter().any(|l| l.contains("fn x() {}")));
    }
}
