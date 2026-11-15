//! A small text editor used for the compose box and every text field.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Debug, Default)]
pub struct Input {
    pub text: String,
    /// Byte offset of the cursor in `text`.
    pub cursor: usize,
}

impl Input {
    pub fn with(text: &str) -> Self {
        Self { text: text.to_string(), cursor: text.len() }
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.len();
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn insert_str(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
    }

    fn prev(&self) -> Option<usize> {
        self.text[..self.cursor].char_indices().next_back().map(|(i, _)| i)
    }

    fn next(&self) -> Option<usize> {
        self.text[self.cursor..].chars().next().map(|c| self.cursor + c.len_utf8())
    }

    pub fn backspace(&mut self) {
        if let Some(p) = self.prev() {
            self.text.replace_range(p..self.cursor, "");
            self.cursor = p;
        }
    }

    pub fn delete(&mut self) {
        if let Some(n) = self.next() {
            self.text.replace_range(self.cursor..n, "");
        }
    }

    pub fn left(&mut self) {
        if let Some(p) = self.prev() {
            self.cursor = p;
        }
    }

    pub fn right(&mut self) {
        if let Some(n) = self.next() {
            self.cursor = n;
        }
    }

    pub fn home(&mut self) {
        self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
    }

    pub fn end(&mut self) {
        self.cursor = self.text[self.cursor..].find('\n').map_or(self.text.len(), |i| self.cursor + i);
    }

    fn word_start(&self) -> usize {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end_matches(|c: char| c.is_whitespace());
        trimmed.rfind(|c: char| c.is_whitespace()).map_or(0, |i| i + trimmed[i..].chars().next().map_or(1, |c| c.len_utf8()))
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start();
    }

    pub fn word_right(&mut self) {
        let after = &self.text[self.cursor..];
        let skipped = after.len() - after.trim_start_matches(|c: char| c.is_whitespace()).len();
        let rest = &after[skipped..];
        let len = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        self.cursor += skipped + len;
    }

    pub fn delete_word(&mut self) {
        let s = self.word_start();
        self.text.replace_range(s..self.cursor, "");
        self.cursor = s;
    }

    /// Apply an editing key. Returns true when the key was consumed.
    pub fn handle_key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Char('a') if ctrl => self.home(),
            KeyCode::Char('w') if ctrl => self.delete_word(),
            KeyCode::Char('u') if ctrl => {
                let c = self.cursor;
                self.home();
                self.text.replace_range(self.cursor..c, "");
            }
            KeyCode::Char(c) if !ctrl && !alt => self.insert(c),
            KeyCode::Backspace if ctrl || alt => self.delete_word(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left if ctrl || alt => self.word_left(),
            KeyCode::Right if ctrl || alt => self.word_right(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.home(),
            KeyCode::End => self.end(),
            _ => return false,
        }
        true
    }

    /// Hard-wrap the text to `width` columns; returns the rows and the cursor (col, row).
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(2);
        let mut rows = vec![String::new()];
        let mut col = 0;
        let mut cur = (0, 0);
        for (i, c) in self.text.char_indices() {
            if i == self.cursor {
                cur = (col, rows.len() - 1);
            }
            if c == '\n' {
                rows.push(String::new());
                col = 0;
                continue;
            }
            let cw = c.width().unwrap_or(0);
            if col + cw > width {
                rows.push(String::new());
                col = 0;
                if i == self.cursor {
                    cur = (0, rows.len() - 1);
                }
            }
            rows.last_mut().unwrap().push(c);
            col += cw;
        }
        if self.cursor >= self.text.len() {
            cur = (col, rows.len() - 1);
            if col >= width {
                rows.push(String::new());
                cur = (0, rows.len() - 1);
            }
        }
        (rows, cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing() {
        let mut i = Input::default();
        i.insert_str("hello wörld");
        i.backspace();
        assert_eq!(i.text, "hello wörl");
        i.word_left();
        i.insert('X');
        assert_eq!(i.text, "hello Xwörl");
        i.delete_word();
        assert_eq!(i.text, "hello wörl");
        i.home();
        i.delete();
        assert_eq!(i.text, "ello wörl");
        i.end();
        i.delete_word();
        assert_eq!(i.text, "ello ");
    }

    #[test]
    fn layout_wraps_and_tracks_cursor() {
        let i = Input::with("abcdef\ngh");
        let (rows, cur) = i.layout(4);
        assert_eq!(rows, vec!["abcd", "ef", "gh"]);
        assert_eq!(cur, (2, 2));
        let mut j = Input::with("abcd");
        assert_eq!(j.layout(4).1, (0, 1));
        j.left();
        assert_eq!(j.layout(4).1, (3, 0));
    }
}
