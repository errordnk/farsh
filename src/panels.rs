//! Far style panels: two directory listings and a key bar, drawn the way
//! Far Manager 3 draws them (filelist.cpp `ShowFileList`/`ShowList`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::grid::{Cell, Grid};
use crate::listing::{self, Entry, Sort};

// Far's default palette, as the terminal's own 16 colors (Far's console
// attributes reach the terminal as these through ConPTY).
/// Blue: the panels' background.
const BG: Color = Color::DarkBlue;
/// Light cyan: panel text, frames, totals.
const TEXT: Color = Color::Cyan;
/// Yellow: column titles, the sort letter, marked names.
const TITLE: Color = Color::Yellow;
const CURSOR_BG: Color = Color::DarkCyan;
const BLACK: Color = Color::Black;

const KEY_BAR: [&str; 12] =
    ["Help", "UserMn", "View", "Edit", "Copy", "RenMov", "MkFold", "Delete", "ConfMn", "Quit", "", ""];

/// Far's view modes (Ctrl+1..Ctrl+4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewMode {
    /// Three columns of names, extensions aligned.
    Brief,
    /// Two columns of names.
    Medium,
    /// Name, Size, Date, Time.
    Full,
    /// Name and Size.
    Wide,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Col {
    Name,
    Size,
    Date,
    Time,
}

impl Col {
    fn title(self) -> &'static str {
        match self {
            Col::Name => "Name",
            Col::Size => "Size",
            Col::Date => "Date",
            Col::Time => "Time",
        }
    }
}

impl ViewMode {
    /// The columns and their widths (0: shares what is left), as Far's defaults.
    fn columns(self) -> &'static [(Col, usize)] {
        match self {
            ViewMode::Brief => &[(Col::Name, 0), (Col::Name, 0), (Col::Name, 0)],
            ViewMode::Medium => &[(Col::Name, 0), (Col::Name, 0)],
            ViewMode::Full => &[(Col::Name, 0), (Col::Size, 6), (Col::Date, 8), (Col::Time, 5)],
            ViewMode::Wide => &[(Col::Name, 0), (Col::Size, 6)],
        }
    }

    /// The status line under the list.
    fn status(self) -> &'static [(Col, usize)] {
        match self {
            ViewMode::Full => &[(Col::Name, 0)],
            _ => &[(Col::Name, 0), (Col::Size, 6), (Col::Date, 8), (Col::Time, 5)],
        }
    }

    fn align_extensions(self) -> bool {
        matches!(self, ViewMode::Brief | ViewMode::Full)
    }
}

/// Far's `PrepareColumnWidths`: zero widths share what is left; columns that
/// do not fit are dropped from the end, the last one cut to fit.
fn layout(spec: &[(Col, usize)], text_width: usize) -> Vec<(Col, usize)> {
    let mut cols = spec.to_vec();
    let width = text_width as isize;
    let total = |cols: &[(Col, usize)]| cols.iter().map(|c| c.1 as isize).sum::<isize>() + cols.len() as isize - 1;
    let mut extra = width - total(&cols);
    let mut zero = cols.iter().filter(|c| c.1 == 0).count() as isize;
    for col in cols.iter_mut().filter(|c| c.1 == 0) {
        let auto = (extra / zero).max(1);
        col.1 = auto as usize;
        extra -= auto;
        zero -= 1;
    }
    loop {
        let sum = total(&cols);
        if sum <= width {
            break;
        }
        let last = cols.len() - 1;
        if cols.len() <= 1 {
            cols[0].1 = width.max(0) as usize;
            break;
        }
        let without = sum - cols[last].1 as isize;
        if width >= without {
            cols[last].1 = (width - without) as usize;
            break;
        }
        cols.pop();
    }
    cols
}

/// Far's `PrepareStripes`: how many columns make one stripe of the list.
fn stripe_len(cols: &[(Col, usize)]) -> usize {
    let n = cols.len();
    (1..=n / 2)
        .find(|&stride| n % stride == 0 && cols.chunks(stride).all(|c| c.iter().map(|x| x.0).eq(cols[..stride].iter().map(|x| x.0))))
        .unwrap_or(n.max(1))
}

pub struct Panel {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    /// Names of the marked entries.
    pub marked: HashSet<String>,
    pub mode: ViewMode,
    sort: Sort,
    reverse: bool,
    top: usize,
    /// List rows and stripes at the last paint.
    height: usize,
    stripes: usize,
    /// Where it was last drawn: column, row, width, height.
    area: (u16, u16, u16, u16),
    /// Screen columns of each stripe at the last paint (first, last).
    stripe_cols: Vec<(u16, u16)>,
}

impl Panel {
    pub fn new(dir: PathBuf) -> Self {
        let mut panel = Self {
            dir,
            entries: Vec::new(),
            cursor: 0,
            marked: HashSet::new(),
            mode: ViewMode::Medium,
            sort: Sort::Extension,
            reverse: false,
            top: 0,
            height: 20,
            stripes: 1,
            area: (0, 0, 0, 0),
            stripe_cols: Vec::new(),
        };
        panel.reload(None);
        panel
    }

    /// Reads the directory: `..` first, then folders, then files, in the
    /// panel's sort order; the cursor goes to `select` if it is there, else
    /// to the top. Marks are dropped.
    pub fn reload(&mut self, select: Option<&str>) {
        let mut entries = listing::read(&self.dir);
        listing::sort(&mut entries, self.sort, self.reverse);
        if self.dir.parent().is_some() {
            let modified = std::fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
            entries.insert(0, Entry::up(modified));
        }
        self.entries = entries;
        self.marked.clear();
        self.cursor = select.and_then(|name| self.entries.iter().position(|e| e.name == name)).unwrap_or(0);
        self.top = 0;
    }

    /// Ctrl+F3 / Ctrl+F4: sort by name or extension; the same again reverses.
    pub fn set_sort(&mut self, sort: Sort) {
        self.reverse = self.sort == sort && !self.reverse;
        self.sort = sort;
        let keep = self.current().map(|e| e.name.clone());
        let marked = std::mem::take(&mut self.marked);
        self.reload(keep.as_deref());
        self.marked = marked;
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    fn page(&self) -> usize {
        (self.stripes * self.height).max(1)
    }

    /// Far's PgUp / PgDn: a page less one, the list moving with the cursor.
    fn page_move(&mut self, down: bool) {
        let n = self.page().saturating_sub(1).max(1);
        if down {
            self.top += n;
            self.move_cursor(n as isize);
        } else {
            self.top = self.top.saturating_sub(n);
            self.move_cursor(-(n as isize));
        }
    }

    /// Far's Left / Right: one stripe over.
    fn stripe_move(&mut self, right: bool) {
        let h = self.height.max(1);
        if right {
            if self.cursor + h < self.entries.len() && self.cursor - self.top.min(self.cursor) >= (self.stripes - 1) * h {
                self.top += h;
            }
            self.move_cursor(h as isize);
        } else {
            if self.top >= h && self.cursor.saturating_sub(self.top) < h {
                self.top -= h;
            }
            self.move_cursor(-(h as isize));
        }
    }

    /// Far's `CorrectPosition`: the cursor inside the list, the list on the cursor.
    fn correct_position(&mut self) {
        let n = self.entries.len();
        if n == 0 {
            self.cursor = 0;
            self.top = 0;
            return;
        }
        let page = self.page();
        if self.top + page > n {
            self.top = n.saturating_sub(page);
        }
        self.cursor = self.cursor.min(n - 1);
        self.top = self.top.min(n - 1);
        if self.cursor < self.top {
            self.top = self.cursor;
        }
        if self.cursor > self.top + page - 1 {
            self.top = self.cursor + 1 - page;
        }
    }

    /// Marks or unmarks the entry under the cursor and moves down (Insert).
    pub fn toggle_mark(&mut self) {
        self.toggle_mark_at(self.cursor);
        self.move_cursor(1);
    }

    fn toggle_mark_at(&mut self, index: usize) {
        if let Some(entry) = self.entries.get(index).filter(|e| !e.is_up()) {
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
        let line = (row as usize).checked_sub(2).filter(|&line| line < self.height)?;
        let stripe = self.stripe_cols.iter().position(|&(first, last)| col >= first && col <= last)?;
        let index = self.top + stripe * self.height + line;
        (index < self.entries.len()).then_some(index)
    }

    /// The marked entries, or the one under the cursor when nothing is marked
    /// (never `..`), as full paths.
    pub fn sources(&self) -> Vec<PathBuf> {
        if self.marked.is_empty() {
            return self
                .current()
                .filter(|e| !e.is_up())
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
        if entry.is_up() {
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

    /// Far's `GetShowColor`: the colors of the entry at `pos` (panel text past the end).
    fn colors(&self, pos: usize, active: bool) -> (Color, Color) {
        let Some(entry) = self.entries.get(pos) else { return (TEXT, BG) };
        let cursor = active && pos == self.cursor;
        let marked = self.marked.contains(&entry.name);
        let (normal, under_cursor) = listing::highlight(entry);
        match (cursor, marked) {
            (true, true) => (TITLE, CURSOR_BG),
            (true, false) => (under_cursor, CURSOR_BG),
            (false, true) => (TITLE, BG),
            (false, false) => (normal, BG),
        }
    }

    fn cell(&self, entry: &Entry, col: Col, width: usize, status: bool) -> (String, bool) {
        match col {
            Col::Name if status => (listing::status_name(&entry.name, width), false),
            Col::Name => listing::list_name(&entry.name, width, self.mode.align_extensions() && !entry.is_dir),
            Col::Size => (listing::size_column(entry, width), false),
            Col::Date => (listing::right(&listing::date_time(entry.modified).0, width), false),
            Col::Time => (listing::right(&listing::date_time(entry.modified).1, width), false),
        }
    }

    fn paint(&mut self, grid: &mut Grid, col: u16, width: u16, height: u16, active: bool) {
        self.area = (col, 0, width, height);
        if width < 3 || height < 3 {
            return;
        }
        let (left, right, bottom) = (col, col + width - 1, height - 1);
        grid.fill(col, 0, width, height, BG);
        let inner = (width - 2) as usize;
        let horizontal = "\u{2550}".repeat(inner);
        grid.put(left, 0, &format!("\u{2554}{horizontal}\u{2557}"), TEXT, BG);
        grid.put(left, bottom, &format!("\u{255A}{horizontal}\u{255D}"), TEXT, BG);
        for row in 1..bottom {
            grid.put(left, row, "\u{2551}", TEXT, BG);
            grid.put(right, row, "\u{2551}", TEXT, BG);
        }

        let cols = layout(self.mode.columns(), inner);
        let in_stripe = stripe_len(&cols);
        self.stripes = (cols.len() / in_stripe).max(1);
        self.height = height.saturating_sub(5) as usize;

        // column titles, with the joints of the dividers in the top border
        self.stripe_cols.clear();
        let mut x = left + 1;
        let mut stripe_start = x;
        for (i, &(kind, w)) in cols.iter().enumerate() {
            grid.put(x, 1, &listing::center(kind.title(), w), TITLE, BG);
            x += w as u16;
            if (i + 1) % in_stripe == 0 {
                self.stripe_cols.push((stripe_start, x));
                stripe_start = x + 1;
            }
            if i == cols.len() - 1 {
                break;
            }
            let double = (i + 1) % in_stripe == 0;
            grid.put(x, 0, if double { "\u{2566}" } else { "\u{2564}" }, TEXT, BG);
            grid.put(x, 1, if double { "\u{2551}" } else { "\u{2502}" }, TEXT, BG);
            x += 1;
        }
        grid.put(left + 1, 1, &self.sort.indicator(self.reverse).to_string(), TITLE, BG);

        // the folder in the top border
        let max = inner;
        let mut title = self.dir.display().to_string();
        if max >= 2 {
            title = listing::truncate_path(&title, max - 2);
        }
        let title = format!(" {title} ");
        let title: String = title.chars().take(max).collect();
        let title_x = left + 1 + (width - 1).saturating_sub(listing::chars(&title) as u16) / 2;
        let (fg, bg) = if active { (BLACK, CURSOR_BG) } else { (TEXT, BG) };
        grid.put(title_x, 0, &title, fg, bg);

        self.correct_position();
        self.paint_list(grid, &cols, in_stripe, active);
        self.paint_status(grid, left, right, bottom);
        self.paint_totals(grid, left, width, bottom, &cols, in_stripe);
    }

    fn paint_list(&self, grid: &mut Grid, cols: &[(Col, usize)], in_stripe: usize, active: bool) {
        let right = self.area.0 + self.area.2 - 1;
        for line in 0..self.height {
            let y = 2 + line as u16;
            let mut x = self.area.0 + 1;
            let mut stripe = 0;
            let mut level = 1;
            for (k, &(kind, w)) in cols.iter().enumerate() {
                let pos = self.top + line + stripe * self.height;
                let (fg, bg) = self.colors(pos, active);
                let (text, too_long) = match self.entries.get(pos) {
                    Some(entry) => self.cell(entry, kind, w, false),
                    None => (" ".repeat(w), false),
                };
                grid.put(x, y, &text, fg, bg);
                // Far's `HighlightBorder`: inside a stripe the divider takes the entry's background
                let border = if level == in_stripe { (TEXT, BG) } else { (TEXT, bg) };
                let last = k == cols.len() - 1;
                if too_long {
                    grid.put(x + w as u16, y, "}", border.0, border.1);
                } else if last {
                    let edge = if x + w as u16 == right { "\u{2551}" } else { " " };
                    grid.put(x + w as u16, y, edge, TEXT, BG);
                } else {
                    let divider = if level == in_stripe { "\u{2551}" } else { "\u{2502}" };
                    grid.put(x + w as u16, y, divider, border.0, border.1);
                }
                x += w as u16 + 1;
                if level == in_stripe {
                    level = 0;
                    stripe += 1;
                }
                level += 1;
            }
            if x < right {
                grid.put(x, y, &" ".repeat((right - x) as usize), TEXT, BG);
            }
        }
    }

    /// The divider over the status line and the status line: the entry under the cursor.
    fn paint_status(&self, grid: &mut Grid, left: u16, right: u16, bottom: u16) {
        let inner = (right - left - 1) as usize;
        let y = bottom - 1;
        grid.put(left, y - 1, &format!("\u{255F}{}\u{2562}", "\u{2500}".repeat(inner)), TEXT, BG);
        let Some(entry) = self.current() else { return };
        let cols = layout(self.mode.status(), inner);
        let mut x = left + 1;
        for (k, &(kind, w)) in cols.iter().enumerate() {
            let (text, _) = self.cell(entry, kind, w, true);
            grid.put(x, y, &text, TEXT, BG);
            let edge = if k == cols.len() - 1 && x + w as u16 == right { "\u{2551}" } else { " " };
            grid.put(x + w as u16, y, edge, TEXT, BG);
            x += w as u16 + 1;
        }
        if x < right {
            grid.put(x, y, &" ".repeat((right - x) as usize), TEXT, BG);
        }
    }

    /// The joints of the dividers, the marked total on the divider and the
    /// folder's total in the bottom border.
    fn paint_totals(&self, grid: &mut Grid, left: u16, width: u16, bottom: u16, cols: &[(Col, usize)], in_stripe: usize) {
        let mut x = left + 1;
        for (i, &(_, w)) in cols.iter().enumerate().take(cols.len().saturating_sub(1)) {
            x += w as u16;
            let joint = if (i + 1) % in_stripe == 0 { "\u{2568}" } else { "\u{2534}" };
            grid.put(x, bottom - 2, joint, TEXT, BG);
            x += 1;
        }
        let room = (width as usize).saturating_sub(4);
        let centered = |text: &str| left + 1 + (room.saturating_sub(listing::chars(text)) / 2) as u16;

        let files = self.entries.iter().filter(|e| !e.is_up() && !e.is_dir);
        let (count, bytes) = files.fold((0, 0), |(n, sum), e| (n + 1, sum + e.size));
        let folders = self.entries.iter().filter(|e| !e.is_up() && e.is_dir).count();
        let total = format!("Bytes: {}, files: {count}, folders: {folders}", listing::float_size(bytes));
        let total = listing::truncate_right(&total, room);
        grid.put(centered(&total), bottom, &format!(" {total} "), TEXT, BG);

        if !self.marked.is_empty() {
            let marked = self.entries.iter().filter(|e| self.marked.contains(&e.name));
            let (files, dirs, bytes) = marked.fold((0, 0, 0), |(f, d, sum), e| {
                if e.is_dir { (f, d + 1, sum) } else { (f + 1, d, sum + e.size) }
            });
            let line = |size: String| format!("Bytes: {size}, files: {files}, folders: {dirs}");
            let mut text = line(listing::group_digits(bytes));
            if listing::chars(&text) > room {
                let short = bytes.to_string();
                text = line(if short.len() <= 6 { short } else { listing::file_size(bytes, 6).trim().to_string() });
                text = listing::truncate_right(&text, room);
            }
            grid.put(centered(&text), bottom - 2, &format!(" {text} "), TITLE, CURSOR_BG);
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
    /// Where each key of the key bar ends (exclusive), at the last paint.
    key_ends: Vec<u16>,
}

impl Panels {
    pub fn new(dir: PathBuf) -> Self {
        Self { left: Panel::new(dir.clone()), right: Panel::new(dir), right_active: false, screen: (0, 0), key_ends: Vec::new() }
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
            panel.entries.iter().filter(|e| !e.is_up() && !e.is_dir).map(|e| e.name.clone()).collect();
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
        let ctrl_only = key.modifiers == KeyModifiers::CONTROL;
        match key.code {
            KeyCode::Tab => self.right_active = !self.right_active,
            KeyCode::Up if plain => self.active().move_cursor(-1),
            KeyCode::Down if plain => self.active().move_cursor(1),
            KeyCode::PageUp => self.active().page_move(false),
            KeyCode::PageDown => self.active().page_move(true),
            // Far: Left/Right go over stripes, or (one stripe) when nothing is typed
            KeyCode::Left | KeyCode::Right if plain && (self.active_ref().stripes > 1 || line_is_empty) => {
                let right = key.code == KeyCode::Right;
                self.active().stripe_move(right);
            }
            // Ctrl+1..Ctrl+4: Far's Brief, Medium, Full and Wide views
            KeyCode::Char(c @ '1'..='4') if ctrl_only => {
                self.active().mode = [ViewMode::Brief, ViewMode::Medium, ViewMode::Full, ViewMode::Wide]
                    [c as usize - '1' as usize];
            }
            KeyCode::F(3) if ctrl_only => self.active().set_sort(Sort::Name),
            KeyCode::F(4) if ctrl_only => self.active().set_sort(Sort::Extension),
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
                if rows > 0 && row == rows - 1 && col < cols {
                    let Some(key) = self.key_ends.iter().position(|&end| col < end) else { return Key::Taken };
                    return self.function_key(key as u8 + 1);
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
        // Far: the left panel ends at ScrX / 2
        let left_width = grid.cols.div_ceil(2);
        let right_width = grid.cols - left_width;
        let right_active = self.right_active;
        self.left.paint(grid, 0, left_width, height, !right_active);
        self.right.paint(grid, left_width, right_width, height, right_active);

        let row = grid.rows - 2;
        grid.set_row(row, line);

        self.paint_key_bar(grid);
        Some((cursor_col.min(grid.cols - 1), row))
    }

    /// Far's `KeyBar::DisplayObject`: twelve keys of at least 8 cells (10-12
    /// one more), the room left spread over them.
    fn paint_key_bar(&mut self, grid: &mut Grid) {
        const MIN_LABEL: usize = 6;
        let y = grid.rows - 1;
        let cols = grid.cols as usize;
        let count = KEY_BAR.len();
        let min_full = (1 + MIN_LABEL + 1) * count + (count - 9) - 1;
        let mut extra = cols.saturating_sub(min_full);
        let (number, text, gap) = ((Color::Grey, BLACK), (BLACK, CURSOR_BG), (Color::Grey, BLACK));
        let mut x = 0usize;
        let mut prev = 0usize;
        self.key_ends.clear();
        for (i, label) in KEY_BAR.iter().enumerate() {
            let two_digits = usize::from(i + 1 > 9);
            grid.put(x as u16, y, &(i + 1).to_string(), number.0, number.1);
            x += 1 + two_digits;
            if x >= cols {
                break;
            }
            let mut end = prev + 1 + two_digits + MIN_LABEL + 1;
            if extra > 0 {
                let even = (i + 1) * cols / count;
                if even > end {
                    extra = extra.saturating_sub(even - end);
                    end = even;
                }
            }
            self.key_ends.push(end as u16);
            let width = end - prev - 2 - two_digits;
            grid.put(x as u16, y, &format!("{:<width$}", label.chars().take(width).collect::<String>()), text.0, text.1);
            x += width;
            if x >= cols {
                break;
            }
            if i < count - 1 {
                grid.put(x as u16, y, " ", gap.0, gap.1);
                x += 1;
                if x >= cols {
                    break;
                }
            }
            prev = end;
        }
        if x < cols {
            grid.put(x as u16, y, &" ".repeat(cols - x), text.0, text.1);
        }
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
        // rows: 2 is "..", 3 Alpha, 4 beta, 5 apple.txt (row 1 has the column titles); the right panel starts at column 40
        assert_eq!(panels.mouse(click(left, 50, 4), false), Key::Taken);
        assert_eq!(panels.active_ref().current().unwrap().name, "beta");
        assert!(panels.right_active, "the click made the right panel active");
        // a double click opens a folder
        panels.mouse(click(left, 50, 4), true);
        assert_eq!(panels.active_dir(), dir.join("beta"));
        // right button marks
        panels.go_to(dir.clone());
        panels.mouse(click(MouseEventKind::Down(MouseButton::Right), 50, 5), false);
        assert!(panels.active_ref().marked.contains("apple.txt"));
        // the wheel moves the cursor by three
        panels.mouse(click(MouseEventKind::ScrollDown, 50, 5), false);
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
