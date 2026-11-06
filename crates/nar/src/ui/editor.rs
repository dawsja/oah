//! Multi-line prompt editor with soft wrapping.

use unicode_segmentation::UnicodeSegmentation;

use super::text::w;

#[derive(Default, Clone, Debug)]
pub struct Editor {
    pub text: String,
    /// Byte offset of the cursor (always on a grapheme boundary).
    pub cursor: usize,
    /// Display column to keep when moving vertically.
    goal_col: Option<usize>,
    pub masked: bool,
}

/// One visual row: byte range in `text`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
}

impl Editor {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        self.cursor = self.text.len();
        self.goal_col = None;
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        self.goal_col = None;
        std::mem::take(&mut self.text)
    }

    pub fn clear(&mut self) {
        self.take();
    }

    pub fn insert(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
        self.goal_col = None;
    }

    fn prev_boundary(&self, i: usize) -> usize {
        self.text[..i].grapheme_indices(true).next_back().map(|(p, _)| p).unwrap_or(0)
    }

    fn next_boundary(&self, i: usize) -> usize {
        self.text[i..].graphemes(true).next().map(|g| i + g.len()).unwrap_or(self.text.len())
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let p = self.prev_boundary(self.cursor);
        self.text.replace_range(p..self.cursor, "");
        self.cursor = p;
        self.goal_col = None;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let n = self.next_boundary(self.cursor);
        self.text.replace_range(self.cursor..n, "");
        self.goal_col = None;
    }

    pub fn left(&mut self) {
        self.cursor = self.prev_boundary(self.cursor);
        self.goal_col = None;
    }

    pub fn right(&mut self) {
        self.cursor = self.next_boundary(self.cursor);
        self.goal_col = None;
    }

    fn word_start(&self, from: usize) -> usize {
        let before = &self.text[..from];
        let trimmed = before.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
        let start = trimmed.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
        start.len()
    }

    fn word_end(&self, from: usize) -> usize {
        let after = &self.text[from..];
        let skip = after.len() - after.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '_').len();
        let rest = &after[skip..];
        let word = rest.len() - rest.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').len();
        from + skip + word
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start(self.cursor);
        self.goal_col = None;
    }

    pub fn word_right(&mut self) {
        self.cursor = self.word_end(self.cursor);
        self.goal_col = None;
    }

    pub fn delete_word_back(&mut self) {
        let s = self.word_start(self.cursor);
        self.text.replace_range(s..self.cursor, "");
        self.cursor = s;
        self.goal_col = None;
    }

    pub fn delete_word_forward(&mut self) {
        let e = self.word_end(self.cursor);
        self.text.replace_range(self.cursor..e, "");
        self.goal_col = None;
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map(|i| i + 1).unwrap_or(0)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..].find('\n').map(|i| self.cursor + i).unwrap_or(self.text.len())
    }

    pub fn home(&mut self) {
        self.cursor = self.line_start();
        self.goal_col = None;
    }

    pub fn end(&mut self) {
        self.cursor = self.line_end();
        self.goal_col = None;
    }

    pub fn kill_to_start(&mut self) {
        let s = self.line_start();
        if s == self.cursor && s > 0 {
            // At line start: join with previous line.
            self.backspace();
            return;
        }
        self.text.replace_range(s..self.cursor, "");
        self.cursor = s;
    }

    pub fn kill_to_end(&mut self) {
        let e = self.line_end();
        if e == self.cursor {
            self.delete();
            return;
        }
        self.text.replace_range(self.cursor..e, "");
    }

    /// Visual rows for a given width (hard newlines + soft wrap by grapheme).
    pub fn rows(&self, width: usize) -> Vec<Row> {
        let width = width.max(2);
        let mut rows = Vec::new();
        let mut start = 0;
        let mut col = 0;
        let mut last = 0;
        for (i, g) in self.text.grapheme_indices(true) {
            if g == "\n" {
                rows.push(Row { start, end: i });
                start = i + 1;
                col = 0;
                last = start;
                continue;
            }
            let gw = self.gw(g);
            if col + gw > width {
                rows.push(Row { start, end: i });
                start = i;
                col = 0;
            }
            col += gw;
            last = i + g.len();
        }
        let _ = last;
        rows.push(Row { start, end: self.text.len() });
        rows
    }

    fn gw(&self, g: &str) -> usize {
        if self.masked {
            1
        } else if g == "\t" {
            4
        } else {
            w(g).max(1)
        }
    }

    pub fn display_slice(&self, r: Row) -> String {
        let s = &self.text[r.start..r.end];
        if self.masked {
            "•".repeat(s.graphemes(true).count())
        } else {
            s.replace('\t', "    ")
        }
    }

    /// (row, col) of the cursor.
    pub fn cursor_pos(&self, width: usize) -> (usize, usize) {
        let rows = self.rows(width);
        for (ri, r) in rows.iter().enumerate() {
            let next_start = rows.get(ri + 1).map(|n| n.start);
            let within = self.cursor >= r.start && (self.cursor < r.end || self.cursor == r.end && next_start != Some(r.end));
            if within || ri == rows.len() - 1 {
                let col: usize = self.text[r.start..self.cursor.min(r.end).max(r.start)].graphemes(true).map(|g| self.gw(g)).sum();
                return (ri, col);
            }
        }
        (0, 0)
    }

    fn byte_at(&self, r: Row, col: usize) -> usize {
        let mut c = 0;
        for (i, g) in self.text[r.start..r.end].grapheme_indices(true) {
            let gw = self.gw(g);
            if c + gw > col {
                return r.start + i;
            }
            c += gw;
        }
        r.end
    }

    /// Move up one visual row. Returns false when already on the first row.
    pub fn up(&mut self, width: usize) -> bool {
        let (row, col) = self.cursor_pos(width);
        if row == 0 {
            return false;
        }
        let goal = *self.goal_col.get_or_insert(col);
        let rows = self.rows(width);
        self.cursor = self.byte_at(rows[row - 1], goal);
        true
    }

    /// Move down one visual row. Returns false when already on the last row.
    pub fn down(&mut self, width: usize) -> bool {
        let (row, col) = self.cursor_pos(width);
        let rows = self.rows(width);
        if row + 1 >= rows.len() {
            return false;
        }
        let goal = *self.goal_col.get_or_insert(col);
        self.cursor = self.byte_at(rows[row + 1], goal);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing() {
        let mut e = Editor::default();
        e.insert("hello world");
        e.word_left();
        assert_eq!(e.cursor, 6);
        e.delete_word_back();
        assert_eq!(e.text, "world");
        e.end();
        e.insert("\nline2");
        assert_eq!(e.rows(80).len(), 2);
        assert!(e.up(80));
        assert!(!e.up(80));
        assert!(e.down(80));
        assert!(!e.down(80));
    }

    #[test]
    fn soft_wrap_cursor() {
        let mut e = Editor::default();
        e.insert("abcdefgh");
        assert_eq!(e.rows(4).len(), 2);
        assert_eq!(e.cursor_pos(4), (1, 4));
        e.home();
        assert_eq!(e.cursor_pos(4), (0, 0));
    }

    #[test]
    fn unicode() {
        let mut e = Editor::default();
        e.insert("héllo 世界");
        e.backspace();
        assert_eq!(e.text, "héllo 世");
        e.left();
        e.left();
        e.backspace();
        assert_eq!(e.text, "héll 世");
    }
}
