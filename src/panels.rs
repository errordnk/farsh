//! Far style panels: two directory listings, a command line and a key bar.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::Color;

use crate::grid::Grid;

const PANEL_BG: Color = Color::Rgb { r: 0, g: 0, b: 170 };
const PANEL_FG: Color = Color::Rgb { r: 85, g: 255, b: 255 };
const DIR_FG: Color = Color::Rgb { r: 255, g: 255, b: 255 };
const CURSOR_BG: Color = Color::Rgb { r: 0, g: 170, b: 170 };
const CURSOR_FG: Color = Color::Rgb { r: 0, g: 0, b: 0 };

const KEY_BAR: [&str; 10] =
    ["Help", "UserMn", "View", "Edit", "Copy", "RenMov", "MkFold", "Delete", "ConfMn", "Quit"];

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub struct Panel {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    top: usize,
}

impl Panel {
    pub fn new(dir: PathBuf) -> Self {
        let mut panel = Self { dir, entries: Vec::new(), cursor: 0, top: 0 };
        panel.reload(None);
        panel
    }

    /// Reads the directory: `..` first, then folders, then files, each by name;
    /// the cursor goes to `select` if it is there, else to the top.
    pub fn reload(&mut self, select: Option<&str>) {
        let mut entries: Vec<Entry> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|item| {
                let meta = item.metadata().ok();
                Entry {
                    name: item.file_name().to_string_lossy().into_owned(),
                    is_dir: meta.as_ref().is_some_and(|m| m.is_dir()),
                    size: meta.map_or(0, |m| m.len()),
                }
            })
            .collect();
        entries.sort_by_key(|e| (!e.is_dir, e.name.to_lowercase()));
        if self.dir.parent().is_some() {
            entries.insert(0, Entry { name: "..".into(), is_dir: true, size: 0 });
        }
        self.entries = entries;
        self.cursor = select.and_then(|name| self.entries.iter().position(|e| e.name == name)).unwrap_or(0);
        self.top = 0;
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    /// Opens the folder under the cursor (`..` goes up and lands on the
    /// folder it came from); a file does nothing.
    pub fn enter(&mut self) {
        let Some(entry) = self.entries.get(self.cursor).cloned() else { return };
        if !entry.is_dir {
            return;
        }
        if entry.name == ".." {
            let from = self.dir.file_name().map(|n| n.to_string_lossy().into_owned());
            if let Some(parent) = self.dir.parent() {
                self.dir = parent.to_path_buf();
            }
            self.reload(from.as_deref());
        } else {
            self.dir.push(&entry.name);
            self.reload(None);
        }
    }

    /// Keeps the cursor inside the `height` visible rows.
    fn scroll_to_cursor(&mut self, height: usize) {
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if height > 0 && self.cursor >= self.top + height {
            self.top = self.cursor + 1 - height;
        }
    }

    fn paint(&mut self, grid: &mut Grid, col: u16, width: u16, height: u16, active: bool) {
        grid.fill(col, 0, width, height, PANEL_BG);
        let inner = width.saturating_sub(2) as usize;
        let horizontal = "\u{2550}".repeat(inner);
        grid.put(col, 0, &format!("\u{2554}{horizontal}\u{2557}"), PANEL_FG, PANEL_BG);
        grid.put(col, height - 1, &format!("\u{255A}{horizontal}\u{255D}"), PANEL_FG, PANEL_BG);
        for row in 1..height - 1 {
            grid.put(col, row, "\u{2551}", PANEL_FG, PANEL_BG);
            grid.put(col + width - 1, row, "\u{2551}", PANEL_FG, PANEL_BG);
        }
        let mut title = self.dir.display().to_string();
        if title.chars().count() + 4 > width as usize {
            let keep = (width as usize).saturating_sub(7);
            let tail: String = title.chars().rev().take(keep).collect::<Vec<_>>().into_iter().rev().collect();
            title = format!("...{tail}");
        }
        let (fg, bg) = if active { (CURSOR_FG, CURSOR_BG) } else { (PANEL_FG, PANEL_BG) };
        let title = format!(" {title} ");
        let title_col = col + (width.saturating_sub(title.chars().count() as u16)) / 2;
        grid.put(title_col, 0, &title, fg, bg);

        let rows = height.saturating_sub(2) as usize;
        self.scroll_to_cursor(rows);
        for (line, entry) in self.entries.iter().skip(self.top).take(rows).enumerate() {
            let row = 1 + line as u16;
            let selected = active && self.top + line == self.cursor;
            let (fg, bg) = match (selected, entry.is_dir) {
                (true, _) => (CURSOR_FG, CURSOR_BG),
                (false, true) => (DIR_FG, PANEL_BG),
                (false, false) => (PANEL_FG, PANEL_BG),
            };
            let size = if entry.is_dir { "<DIR>".to_string() } else { entry.size.to_string() };
            let name_width = inner.saturating_sub(size.chars().count() + 1);
            let name: String = entry.name.chars().take(name_width).collect();
            let text = format!("{name:<name_width$} {size:>width$}", width = size.chars().count());
            grid.put(col + 1, row, &text, fg, bg);
        }
    }
}

pub struct Panels {
    left: Panel,
    right: Panel,
    right_active: bool,
}

impl Panels {
    pub fn new(dir: PathBuf) -> Self {
        Self { left: Panel::new(dir.clone()), right: Panel::new(dir), right_active: false }
    }

    pub fn active_dir(&self) -> &std::path::Path {
        if self.right_active {
            &self.right.dir
        } else {
            &self.left.dir
        }
    }

    /// Takes the active panel to `dir` (the shell went there).
    pub fn go_to(&mut self, dir: PathBuf) {
        if self.active_dir() != dir {
            let panel = self.active();
            panel.dir = dir;
            panel.reload(None);
        }
    }

    fn active(&mut self) -> &mut Panel {
        if self.right_active {
            &mut self.right
        } else {
            &mut self.left
        }
    }

    /// Handles a key while the panels are up; returns whether the panels took
    /// it. Everything else (typing, editing, history, Enter over a typed
    /// command) is the shell's own command line. While that line is empty
    /// Enter, Home and End also belong to the panel.
    pub fn key(&mut self, key: KeyEvent, line_is_empty: bool) -> bool {
        let plain = !key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Tab => self.right_active = !self.right_active,
            KeyCode::Up if plain => self.active().move_cursor(-1),
            KeyCode::Down if plain => self.active().move_cursor(1),
            KeyCode::PageUp => self.active().move_cursor(-10),
            KeyCode::PageDown => self.active().move_cursor(10),
            KeyCode::Home if line_is_empty => self.active().move_cursor(isize::MIN / 2),
            KeyCode::End if line_is_empty => self.active().move_cursor(isize::MAX / 2),
            KeyCode::Enter if line_is_empty => self.active().enter(),
            _ => return false,
        }
        true
    }

    /// Draws the panels and the key bar over `grid`, with the shell's own
    /// command line (`line`, its cursor at `cursor_col`) between them;
    /// returns where the cursor goes.
    pub fn paint(&mut self, grid: &mut Grid, line: &str, cursor_col: u16) -> Option<(u16, u16)> {
        if grid.rows < 5 || grid.cols < 20 {
            return None;
        }
        let height = grid.rows - 2;
        let left_width = grid.cols / 2;
        let right_width = grid.cols - left_width;
        let right_active = self.right_active;
        self.left.paint(grid, 0, left_width, height, !right_active);
        self.right.paint(grid, left_width, right_width, height, right_active);

        let row = grid.rows - 2;
        let grey = Color::Rgb { r: 170, g: 170, b: 170 };
        let black = Color::Rgb { r: 0, g: 0, b: 0 };
        grid.fill_from(0, row, black);
        grid.put(0, row, &line.chars().take(grid.cols as usize).collect::<String>(), grey, black);

        let cell = grid.cols / 10;
        for (index, label) in KEY_BAR.iter().enumerate() {
            let col = index as u16 * cell;
            let number = format!("{}", index + 1);
            grid.put(col, grid.rows - 1, &number, Color::Rgb { r: 170, g: 170, b: 170 }, Color::Rgb { r: 0, g: 0, b: 0 });
            let label: String = label.chars().take(cell.saturating_sub(number.len() as u16) as usize).collect();
            let label = format!("{label:<width$}", width = cell as usize - number.len());
            grid.put(col + number.len() as u16, grid.rows - 1, &label, CURSOR_FG, CURSOR_BG);
        }
        Some((cursor_col.min(grid.cols - 1), row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("farsh-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("beta")).unwrap();
        std::fs::create_dir_all(dir.join("Alpha")).unwrap();
        std::fs::write(dir.join("zeta.txt"), "abc").unwrap();
        std::fs::write(dir.join("apple.txt"), "").unwrap();
        dir
    }

    fn names(panel: &Panel) -> Vec<&str> {
        panel.entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn listing_puts_up_then_folders_then_files() {
        let dir = scratch("listing");
        let panel = Panel::new(dir.clone());
        assert_eq!(names(&panel), ["..", "Alpha", "beta", "apple.txt", "zeta.txt"]);
        assert_eq!(panel.entries[4].size, 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn entering_a_folder_and_coming_back_keeps_the_cursor_on_it() {
        let dir = scratch("enter");
        let mut panel = Panel::new(dir.clone());
        panel.move_cursor(2); // beta
        panel.enter();
        assert_eq!(panel.dir, dir.join("beta"));
        assert_eq!(names(&panel), [".."]);
        panel.enter(); // ".." from inside beta
        assert_eq!(panel.dir, dir);
        assert_eq!(panel.entries[panel.cursor].name, "beta");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_panel_follows_the_shell_to_another_directory() {
        let dir = scratch("follow");
        let mut panels = Panels::new(dir.clone());
        panels.go_to(dir.join("beta"));
        assert_eq!(panels.active_dir(), dir.join("beta"));
        panels.go_to(dir.join("beta"));
        assert_eq!(panels.active_dir(), dir.join("beta"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_cursor_stays_inside_the_list() {
        let dir = scratch("clamp");
        let mut panel = Panel::new(dir.clone());
        panel.move_cursor(-5);
        assert_eq!(panel.cursor, 0);
        panel.move_cursor(100);
        assert_eq!(panel.cursor, panel.entries.len() - 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_panels_take_navigation_and_leave_the_command_line_to_the_shell() {
        let dir = scratch("keys");
        let mut panels = Panels::new(dir.clone());
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert!(panels.key(key(KeyCode::Down), false));
        assert!(!panels.key(key(KeyCode::Char('l')), true));
        assert!(!panels.key(key(KeyCode::Backspace), true));
        // Enter, Home and End are the panel's only while the line is empty.
        assert!(!panels.key(key(KeyCode::Enter), false));
        assert!(!panels.key(key(KeyCode::Home), false));
        assert!(panels.key(key(KeyCode::Enter), true));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn far_history_keys_reach_the_shell_as_arrows() {
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert_eq!(to_shell(ctrl('e')).code, KeyCode::Up);
        assert_eq!(to_shell(ctrl('x')).code, KeyCode::Down);
        assert_eq!(to_shell(ctrl('a')).code, KeyCode::Char('a'));
    }

    #[test]
    fn painting_shows_both_panels_and_the_key_bar() {
        let dir = scratch("paint");
        let mut panels = Panels::new(dir.clone());
        let mut grid = Grid::new(80, 24);
        let cursor = panels.paint(&mut grid, "~/src$ ls", 9);
        assert_eq!(cursor, Some((9, 22)));
        assert_eq!(grid.cell_mut(1, 22).unwrap().text, "/");
        assert_eq!(grid.cell_mut(0, 0).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(40, 0).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(0, 23).unwrap().text, "1");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// A key as the shell should get it: Ctrl+E and Ctrl+X are Far's history
/// keys (the arrows belong to the panel), so they go as Up and Down.
pub fn to_shell(key: KeyEvent) -> KeyEvent {
    if key.modifiers == KeyModifiers::CONTROL {
        match key.code {
            KeyCode::Char('e') => return KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            KeyCode::Char('x') => return KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            _ => {}
        }
    }
    key
}
