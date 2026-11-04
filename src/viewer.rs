//! F3: a plain text viewer over the whole screen.

use std::io::Read;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::grid::Grid;

/// Bigger files are shown up to this much from the start.
const LIMIT: u64 = 32 << 20;

// Far's viewer: light cyan on blue, the title bar black on cyan
const BG: Color = crate::palette::bg(crate::palette::BLUE);
const FG: Color = crate::palette::fg(crate::palette::LIGHT_CYAN);
const BAR_FG: Color = crate::palette::fg(crate::palette::BLACK);
const BAR_BG: Color = crate::palette::bg(crate::palette::CYAN);

pub struct Viewer {
    name: String,
    lines: Vec<String>,
    truncated: bool,
    top: usize,
    left: usize,
    /// Rows of text on the last screen painted.
    page: usize,
}

pub enum Outcome {
    Open,
    Closed,
}

impl Viewer {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let mut bytes = Vec::new();
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        file.take(LIMIT).read_to_end(&mut bytes)?;
        let mut viewer = Self::from_text(&path.display().to_string(), &String::from_utf8_lossy(&bytes));
        viewer.truncated = size > LIMIT;
        Ok(viewer)
    }

    pub fn from_text(name: &str, text: &str) -> Self {
        Self {
            name: name.into(),
            lines: text.lines().map(|line| line.replace('\t', "    ").replace('\r', "")).collect(),
            truncated: false,
            top: 0,
            left: 0,
            page: 20,
        }
    }

    /// The file's name, as the screens list shows it.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn scroll(&mut self, rows: isize) {
        let last = self.lines.len().saturating_sub(1) as isize;
        self.top = (self.top as isize + rows).clamp(0, last) as usize;
    }

    /// The wheel scrolls three lines at a time.
    pub fn mouse(&mut self, event: MouseEvent) {
        match event.kind {
            MouseEventKind::ScrollUp => self.scroll(-3),
            MouseEventKind::ScrollDown => self.scroll(3),
            _ => {}
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let page = self.page.max(1) as isize;
        match key.code {
            KeyCode::Esc | KeyCode::F(3) | KeyCode::F(10) | KeyCode::Char('q') => return Outcome::Closed,
            KeyCode::Up => self.scroll(-1),
            KeyCode::Down | KeyCode::Enter => self.scroll(1),
            KeyCode::PageUp => self.scroll(-page),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll(page),
            KeyCode::Home => {
                self.top = 0;
                self.left = 0;
            }
            KeyCode::End => self.top = self.lines.len().saturating_sub(self.page),
            KeyCode::Left => self.left = self.left.saturating_sub(8),
            KeyCode::Right => self.left += 8,
            _ => {}
        }
        Outcome::Open
    }

    /// The text in a window of the screen, wrapped at its width (the quick
    /// view's viewer: Far's ViewerIsWrap is on by default), no bars.
    pub fn paint_area(&mut self, grid: &mut Grid, x: u16, y: u16, width: u16, height: u16) {
        self.page = height as usize;
        let width = width.max(1) as usize;
        let mut row = 0u16;
        'lines: for line in self.lines.iter().skip(self.top) {
            let chars: Vec<char> = line.chars().collect();
            let mut start = 0;
            loop {
                if row >= height {
                    break 'lines;
                }
                let piece: String = chars[start..(start + width).min(chars.len())].iter().collect();
                grid.put(x, y + row, &piece, FG, BG);
                row += 1;
                start += width;
                if start >= chars.len() {
                    break;
                }
            }
        }
    }

    pub fn paint(&mut self, grid: &mut Grid) {
        grid.fill_with(0, 0, grid.cols, grid.rows, FG, BG);
        if grid.rows < 3 {
            return;
        }
        self.page = grid.rows as usize - 2;
        let title: String = self.name.chars().take(grid.cols as usize).collect();
        grid.fill_with(0, 0, grid.cols, 1, BAR_FG, BAR_BG);
        grid.put(0, 0, &title, BAR_FG, BAR_BG);
        for (row, line) in self.lines.iter().skip(self.top).take(self.page).enumerate() {
            let shown: String = line.chars().skip(self.left).take(grid.cols as usize).collect();
            grid.put(0, 1 + row as u16, &shown, FG, BG);
        }
        let percent = if self.lines.len() <= self.page { 100 } else { (self.top + self.page).min(self.lines.len()) * 100 / self.lines.len() };
        let note = if self.truncated { " (cut)" } else { "" };
        let status = format!(" Line {}/{}  {}%{} -- Esc, F3 or F10 closes", self.top + 1, self.lines.len().max(1), percent, note);
        grid.fill_with(0, grid.rows - 1, grid.cols, 1, BAR_FG, BAR_BG);
        grid.put(0, grid.rows - 1, &status, BAR_FG, BAR_BG);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn scrolling_stays_inside_the_text_and_esc_closes() {
        let text: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let mut viewer = Viewer::from_text("t.txt", &text);
        let mut grid = Grid::new(40, 12);
        viewer.paint(&mut grid);
        assert_eq!(grid.cell_mut(0, 1).unwrap().text, "l");
        viewer.key(key(KeyCode::PageDown));
        viewer.paint(&mut grid);
        assert_eq!(viewer.top, 10);
        viewer.key(key(KeyCode::Home));
        viewer.key(key(KeyCode::Up));
        assert_eq!(viewer.top, 0);
        viewer.key(key(KeyCode::End));
        viewer.key(key(KeyCode::PageDown));
        assert!(viewer.top < 100);
        assert!(matches!(viewer.key(key(KeyCode::Esc)), Outcome::Closed));
    }

    #[test]
    fn a_file_is_shown_with_tabs_expanded_and_bad_utf8_replaced() {
        let path = std::env::temp_dir().join(format!("farsh-view-{}.txt", std::process::id()));
        std::fs::write(&path, b"a\tb\r\nbad \xff byte\n").unwrap();
        let viewer = Viewer::open(&path).unwrap();
        assert_eq!(viewer.lines[0], "a    b");
        assert!(viewer.lines[1].contains('\u{fffd}'));
        std::fs::remove_file(path).unwrap();
    }
}
