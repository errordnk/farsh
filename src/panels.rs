//! Far style panels: two directory listings and a key bar.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::grid::{Cell, Grid};

const PANEL_BG: Color = Color::Rgb { r: 0, g: 0, b: 170 };
const PANEL_FG: Color = Color::Rgb { r: 85, g: 255, b: 255 };
const DIR_FG: Color = Color::Rgb { r: 255, g: 255, b: 255 };
const MARK_FG: Color = Color::Rgb { r: 255, g: 255, b: 85 };
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
    /// Names of the marked entries.
    pub marked: HashSet<String>,
    top: usize,
    /// Where it was last drawn: column, row, width, height.
    area: (u16, u16, u16, u16),
}

impl Panel {
    pub fn new(dir: PathBuf) -> Self {
        let mut panel = Self { dir, entries: Vec::new(), cursor: 0, marked: HashSet::new(), top: 0, area: (0, 0, 0, 0) };
        panel.reload(None);
        panel
    }

    /// Reads the directory: `..` first, then folders, then files, each by name;
    /// the cursor goes to `select` if it is there, else to the top. Marks are dropped.
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
        self.marked.clear();
        self.cursor = select.and_then(|name| self.entries.iter().position(|e| e.name == name)).unwrap_or(0);
        self.top = 0;
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    /// Marks or unmarks the entry under the cursor and moves down (Insert).
    pub fn toggle_mark(&mut self) {
        self.toggle_mark_at(self.cursor);
        self.move_cursor(1);
    }

    fn toggle_mark_at(&mut self, index: usize) {
        if let Some(entry) = self.entries.get(index).filter(|e| e.name != "..") {
            let name = entry.name.clone();
            if !self.marked.remove(&name) {
                self.marked.insert(name);
            }
        }
    }

    fn contains(&self, col: u16, row: u16) -> bool {
        let (x, y, w, h) = self.area;
        col >= x && col < x + w && row >= y && row < y + h
    }

    /// The entry drawn at (`col`, `row`), if one is.
    fn entry_at(&self, col: u16, row: u16) -> Option<usize> {
        let (x, y, w, h) = self.area;
        let inside = col > x && col + 1 < x + w && row > y && row + 1 < y + h;
        let index = self.top + row.checked_sub(y + 1)? as usize;
        (inside && index < self.entries.len()).then_some(index)
    }

    /// The marked entries, or the one under the cursor when nothing is marked
    /// (never `..`), as full paths.
    pub fn sources(&self) -> Vec<PathBuf> {
        if self.marked.is_empty() {
            return self
                .current()
                .filter(|e| e.name != "..")
                .map(|e| vec![self.dir.join(&e.name)])
                .unwrap_or_default();
        }
        self.entries.iter().filter(|e| self.marked.contains(&e.name)).map(|e| self.dir.join(&e.name)).collect()
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
        self.area = (col, 0, width, height);
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
            let marked = self.marked.contains(&entry.name);
            let (fg, bg) = match (selected, marked, entry.is_dir) {
                (true, true, _) => (MARK_FG, CURSOR_BG),
                (true, false, _) => (CURSOR_FG, CURSOR_BG),
                (false, true, _) => (MARK_FG, PANEL_BG),
                (false, false, true) => (DIR_FG, PANEL_BG),
                (false, false, false) => (PANEL_FG, PANEL_BG),
            };
            let size = if entry.is_dir { "<DIR>".to_string() } else { entry.size.to_string() };
            let name_width = inner.saturating_sub(size.chars().count() + 1);
            let name: String = entry.name.chars().take(name_width).collect();
            let text = format!("{name:<name_width$} {size:>width$}", width = size.chars().count());
            grid.put(col + 1, row, &text, fg, bg);
        }
    }
}

/// What a key asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    View(PathBuf),
    Edit(PathBuf),
    /// Sources and the other panel's folder (the default destination).
    Copy(Vec<PathBuf>, PathBuf),
    Move(Vec<PathBuf>, PathBuf),
    MkDir,
    Delete(Vec<PathBuf>),
    Quit,
    /// F9: the menu bar.
    Menu,
    /// Alt+F1 / Alt+F2: another drive (mount point) for the left / right panel.
    ChangeDrive { right: bool },
}

/// What the panels made of a key.
#[derive(Debug, PartialEq)]
pub enum Key {
    /// Not theirs: it belongs to the shell's command line.
    Pass,
    Taken,
    Ask(Request),
}

pub struct Panels {
    left: Panel,
    right: Panel,
    right_active: bool,
    /// The screen size at the last paint.
    screen: (u16, u16),
}

impl Panels {
    pub fn new(dir: PathBuf) -> Self {
        Self { left: Panel::new(dir.clone()), right: Panel::new(dir), right_active: false, screen: (0, 0) }
    }

    pub fn active_dir(&self) -> &Path {
        &self.active_ref().dir
    }

    /// The name under the cursor of the active panel.
    pub fn current_name(&self) -> Option<&str> {
        self.active_ref().current().map(|e| e.name.as_str())
    }

    pub fn other_dir(&self) -> &Path {
        if self.right_active {
            &self.left.dir
        } else {
            &self.right.dir
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

    /// The folder the left or right panel shows.
    pub fn dir(&self, right: bool) -> &Path {
        if right { &self.right.dir } else { &self.left.dir }
    }

    /// Takes the left or right panel to `dir` and makes it the active one.
    pub fn open_dir(&mut self, right: bool, dir: PathBuf) {
        self.right_active = right;
        let panel = self.active();
        panel.dir = dir;
        panel.reload(None);
    }

    pub fn right_active(&self) -> bool {
        self.right_active
    }

    /// Ctrl+U: the panels change places; the active one stays the same panel.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.left, &mut self.right);
        self.right_active = !self.right_active;
    }

    /// Gray *: marks what is not marked and unmarks what is (files only, as Far does).
    pub fn invert_marks(&mut self) {
        let panel = self.active();
        let names: Vec<String> =
            panel.entries.iter().filter(|e| e.name != ".." && !e.is_dir).map(|e| e.name.clone()).collect();
        for name in names {
            if !panel.marked.remove(&name) {
                panel.marked.insert(name);
            }
        }
    }

    /// Reads both panels again (after a copy, a delete...); the active
    /// panel's cursor goes to `select` if given.
    pub fn refresh(&mut self, select: Option<&str>) {
        let keep = self.active_ref().current().map(|e| e.name.clone());
        let other_keep = {
            let other = if self.right_active { &self.left } else { &self.right };
            other.current().map(|e| e.name.clone())
        };
        let right_active = self.right_active;
        for (is_right, panel) in [(false, &mut self.left), (true, &mut self.right)] {
            let wanted = if is_right == right_active { select.map(String::from).or(keep.clone()) } else { other_keep.clone() };
            panel.reload(wanted.as_deref());
        }
    }

    fn active_ref(&self) -> &Panel {
        if self.right_active {
            &self.right
        } else {
            &self.left
        }
    }

    fn active(&mut self) -> &mut Panel {
        if self.right_active {
            &mut self.right
        } else {
            &mut self.left
        }
    }

    /// Handles a key while the panels are up. Everything but navigation,
    /// marking and the function keys (typing, editing, history, Enter over a
    /// typed command) is the shell's own command line; while that line is
    /// empty Enter, Home and End also belong to the panel.
    pub fn key(&mut self, key: KeyEvent, line_is_empty: bool) -> Key {
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
            KeyCode::Insert => self.active().toggle_mark(),
            // Far's Ctrl+R and Ctrl+U, while nothing is typed (else they are the shell's)
            KeyCode::Char('r') if !plain && line_is_empty => self.refresh(None),
            KeyCode::Char('u') if !plain && line_is_empty => self.swap(),
            KeyCode::F(n @ (1 | 2)) if key.modifiers.contains(KeyModifiers::ALT) => {
                return Key::Ask(Request::ChangeDrive { right: n == 2 });
            }
            KeyCode::F(n) => return self.function_key(n),
            _ => return Key::Pass,
        }
        Key::Taken
    }

    /// Handles a mouse event over the panels. `double` is a second click on
    /// the same spot in quick succession.
    pub fn mouse(&mut self, event: MouseEvent, double: bool) -> Key {
        let (cols, rows) = self.screen;
        let (col, row) = (event.column, event.row);
        let side = if self.left.contains(col, row) {
            Some(false)
        } else if self.right.contains(col, row) {
            Some(true)
        } else {
            None
        };
        match event.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let Some(right) = side else { return Key::Pass };
                self.right_active = right;
                let step = if event.kind == MouseEventKind::ScrollUp { -3 } else { 3 };
                self.active().move_cursor(step);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if rows > 0 && row == rows - 1 {
                    // the key bar: ten equal cells
                    let cell = (cols / 10).max(1);
                    return self.function_key(((col / cell) + 1).min(10) as u8);
                }
                let Some(right) = side else { return Key::Pass };
                self.right_active = right;
                if let Some(index) = self.active().entry_at(col, row) {
                    self.active().cursor = index;
                    if double && self.active().entries[index].is_dir {
                        self.active().enter();
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                let Some(right) = side else { return Key::Pass };
                self.right_active = right;
                if let Some(index) = self.active().entry_at(col, row) {
                    self.active().cursor = index;
                    self.active().toggle_mark_at(index);
                }
            }
            _ => return Key::Pass,
        }
        Key::Taken
    }

    pub fn function_key(&mut self, n: u8) -> Key {
        let sources = self.active_ref().sources();
        let file = sources.first().filter(|path| path.is_file()).cloned();
        let other = self.other_dir().to_path_buf();
        let request = match n {
            3 => file.map(Request::View),
            4 => file.map(Request::Edit),
            5 => (!sources.is_empty()).then(|| Request::Copy(sources, other)),
            6 => (!sources.is_empty()).then(|| Request::Move(sources, other)),
            7 => Some(Request::MkDir),
            8 => (!sources.is_empty()).then(|| Request::Delete(sources)),
            9 => Some(Request::Menu),
            10 => Some(Request::Quit),
            _ => None,
        };
        request.map_or(Key::Taken, Key::Ask)
    }

    /// Draws the panels and the key bar over `grid`, with the shell's own
    /// command line between them: `line` is the shell's cursor row exactly as
    /// it is drawn (colors and all), its cursor at `cursor_col`. Returns where
    /// the cursor goes.
    pub fn paint(&mut self, grid: &mut Grid, line: &[Cell], cursor_col: u16) -> Option<(u16, u16)> {
        if grid.rows < 5 || grid.cols < 20 {
            return None;
        }
        self.screen = (grid.cols, grid.rows);
        let height = grid.rows - 2;
        let left_width = grid.cols / 2;
        let right_width = grid.cols - left_width;
        let right_active = self.right_active;
        self.left.paint(grid, 0, left_width, height, !right_active);
        self.right.paint(grid, left_width, right_width, height, right_active);

        let row = grid.rows - 2;
        grid.set_row(row, line);

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

/// Ctrl+Enter: put the highlighted name on the command line. Terminals send
/// it as Enter with Ctrl, or as the line feed (Ctrl+J) a Ctrl+Enter often is.
pub fn is_insert_name(key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    ctrl && matches!(key.code, KeyCode::Enter | KeyCode::Char('j') | KeyCode::Char('\n'))
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

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
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
        assert_eq!(panels.key(key(KeyCode::Down), false), Key::Taken);
        assert_eq!(panels.key(key(KeyCode::Char('l')), true), Key::Pass);
        assert_eq!(panels.key(key(KeyCode::Backspace), true), Key::Pass);
        // Enter, Home and End are the panel's only while the line is empty.
        assert_eq!(panels.key(key(KeyCode::Enter), false), Key::Pass);
        assert_eq!(panels.key(key(KeyCode::Home), false), Key::Pass);
        assert_eq!(panels.key(key(KeyCode::Enter), true), Key::Taken);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn insert_marks_and_moves_down_and_the_marked_are_the_sources() {
        let dir = scratch("marks");
        let mut panels = Panels::new(dir.clone());
        // on ".." nothing can be marked or sent anywhere
        assert!(panels.active_ref().sources().is_empty());
        panels.key(key(KeyCode::Down), true); // Alpha
        panels.key(key(KeyCode::Insert), true); // marks Alpha, now on beta
        panels.key(key(KeyCode::Down), true); // apple.txt
        panels.key(key(KeyCode::Insert), true);
        assert_eq!(panels.active_ref().sources(), [dir.join("Alpha"), dir.join("apple.txt")]);
        // Insert again on a marked entry unmarks it
        panels.key(key(KeyCode::Up), true);
        panels.key(key(KeyCode::Insert), true);
        assert_eq!(panels.active_ref().sources(), [dir.join("Alpha")]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn function_keys_ask_for_the_right_things() {
        let dir = scratch("fkeys");
        let mut panels = Panels::new(dir.clone());
        assert_eq!(panels.key(key(KeyCode::F(5)), true), Key::Taken, "nothing to copy on ..");
        assert_eq!(panels.key(key(KeyCode::F(7)), true), Key::Ask(Request::MkDir));
        assert_eq!(panels.key(key(KeyCode::F(10)), true), Key::Ask(Request::Quit));
        panels.key(key(KeyCode::Down), true); // Alpha, a folder
        assert_eq!(panels.key(key(KeyCode::F(3)), true), Key::Taken, "a folder is not viewed");
        assert_eq!(
            panels.key(key(KeyCode::F(5)), true),
            Key::Ask(Request::Copy(vec![dir.join("Alpha")], dir.clone()))
        );
        assert_eq!(panels.key(key(KeyCode::F(8)), true), Key::Ask(Request::Delete(vec![dir.join("Alpha")])));
        panels.key(key(KeyCode::Down), true);
        panels.key(key(KeyCode::Down), true); // apple.txt
        assert_eq!(panels.key(key(KeyCode::F(3)), true), Key::Ask(Request::View(dir.join("apple.txt"))));
        assert_eq!(panels.key(key(KeyCode::F(4)), true), Key::Ask(Request::Edit(dir.join("apple.txt"))));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refresh_picks_up_new_files_and_can_land_on_one() {
        let dir = scratch("refresh");
        let mut panels = Panels::new(dir.clone());
        std::fs::create_dir(dir.join("new")).unwrap();
        panels.refresh(Some("new"));
        assert_eq!(panels.active_ref().current().unwrap().name, "new");
        assert!(panels.other_dir() == dir);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ctrl_enter_is_told_from_enter_and_the_name_under_the_cursor_is_found() {
        let ctrl = |code| KeyEvent::new(code, KeyModifiers::CONTROL);
        assert!(is_insert_name(ctrl(KeyCode::Enter)));
        assert!(is_insert_name(ctrl(KeyCode::Char('j'))));
        assert!(!is_insert_name(key(KeyCode::Enter)));
        assert!(!is_insert_name(ctrl(KeyCode::Char('o'))));
        let dir = scratch("name");
        let mut panels = Panels::new(dir.clone());
        panels.key(key(KeyCode::Down), true);
        assert_eq!(panels.current_name(), Some("Alpha"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn click(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }
    }

    #[test]
    fn the_mouse_picks_entries_opens_folders_marks_and_presses_the_key_bar() {
        let dir = scratch("mouse");
        let mut panels = Panels::new(dir.clone());
        let mut grid = Grid::new(80, 24);
        panels.paint(&mut grid, &[], 0);
        let left = MouseEventKind::Down(MouseButton::Left);
        // rows: 1 is "..", 2 Alpha, 3 beta, 4 apple.txt; the right panel starts at column 40
        assert_eq!(panels.mouse(click(left, 50, 3), false), Key::Taken);
        assert_eq!(panels.active_ref().current().unwrap().name, "beta");
        assert!(panels.right_active, "the click made the right panel active");
        // a double click opens a folder
        panels.mouse(click(left, 50, 3), true);
        assert_eq!(panels.active_dir(), dir.join("beta"));
        // right button marks
        panels.go_to(dir.clone());
        panels.mouse(click(MouseEventKind::Down(MouseButton::Right), 50, 4), false);
        assert!(panels.active_ref().marked.contains("apple.txt"));
        // the wheel moves the cursor by three
        panels.mouse(click(MouseEventKind::ScrollDown, 50, 4), false);
        assert_eq!(panels.active_ref().cursor, 4);
        // F5 sits in the fifth cell of the key bar (8 columns each)
        assert_eq!(
            panels.mouse(click(left, 8 * 4 + 2, 23), false),
            Key::Ask(Request::Copy(vec![dir.join("apple.txt")], dir.clone()))
        );
        // empty space and the command line are not the panels'
        assert_eq!(panels.mouse(click(left, 10, 22), false), Key::Pass);
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
        let red = Color::Rgb { r: 255, g: 0, b: 0 };
        let line: Vec<Cell> =
            "~/src$ ls".chars().map(|c| Cell { text: c.to_string(), fg: red, ..Cell::default() }).collect();
        let cursor = panels.paint(&mut grid, &line, 9);
        assert_eq!(cursor, Some((9, 22)));
        // the shell's own line, in the colors it drew it with
        assert_eq!(grid.cell_mut(1, 22).unwrap().text, "/");
        assert_eq!(grid.cell_mut(1, 22).unwrap().fg, red);
        assert_eq!(grid.cell_mut(0, 0).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(40, 0).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(0, 23).unwrap().text, "1");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
