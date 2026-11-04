//! A Far menu centered on the screen (vmenu.cpp's VMenu shown at -1, -1):
//! a double box on cyan with a margin and a shadow, the service area of
//! VMenu (box, check mark, the left scroll mark, the right one, box: 5)
//! around the longest item or title, a title on the top line, another on
//! the bottom one, the scroll bar on the right border when the items do not
//! fit (MaxHeight). Hot keys are `&` in the text (unless the menu shows
//! ampersands) and, with `auto_highlight`, AssignHighlights' choice. Keys
//! the menu does not use itself go back to the caller (Far's menu callback).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::grid::Grid;
use crate::palette;

const BG: Color = palette::bg(palette::CYAN);
const TEXT: Color = palette::fg(palette::WHITE);
const HOT: Color = palette::fg(palette::YELLOW);
const GREY: Color = palette::fg(palette::DARK_GRAY);
const SELECTED_BG: Color = palette::bg(palette::BLACK);

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Closed,
    Chosen(usize),
    /// A key the menu does not use: the caller's (the item is `selected()`).
    Key(KeyEvent),
}

#[derive(Clone, Debug, Default)]
pub struct Entry {
    /// `&` before the hot key (`&&` for one `&`), unless the menu shows ampersands.
    pub text: String,
    /// LIF_CHECKED: `√`, or the item's own mark (`+`, `-` of the filters).
    pub check: Option<char>,
    /// LIF_SEPARATOR, its text (if any) in the line.
    pub separator: bool,
    pub disabled: bool,
    /// MIF_SUBMENU: `►` on the right.
    pub submenu: bool,
}

impl Entry {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Self::default() }
    }

    pub fn separator(text: impl Into<String>) -> Self {
        Self { text: text.into(), separator: true, ..Self::default() }
    }

    pub fn checked(mut self, check: Option<char>) -> Self {
        self.check = check;
        self
    }
}

pub struct VMenu {
    pub title: String,
    pub bottom: String,
    pub entries: Vec<Entry>,
    selected: usize,
    /// The first item shown.
    top: usize,
    /// MaxHeight as Far gives it: the screen's last row (ScrY) less this; 0 is none.
    pub max_height_margin: Option<u16>,
    /// VMENU_SHOWAMPERSAND: `&` is just a character.
    pub show_ampersand: bool,
    /// VMENU_AUTOHIGHLIGHT / AssignHighlights.
    pub auto_highlight: bool,
    /// left, top, right, bottom at the last paint
    frame: (u16, u16, u16, u16),
    /// The visible rows at the last paint.
    rows: usize,
}

impl VMenu {
    /// A plain menu (a plugin's Far::menu): wrap mode, no limit.
    pub fn new(title: &str, items: Vec<String>) -> Self {
        Self::with(title, items.into_iter().map(Entry::new).collect())
    }

    pub fn with(title: &str, entries: Vec<Entry>) -> Self {
        let mut menu = Self {
            title: title.into(),
            bottom: String::new(),
            entries,
            selected: 0,
            top: 0,
            max_height_margin: None,
            show_ampersand: false,
            auto_highlight: false,
            frame: (0, 0, 0, 0),
            rows: 0,
        };
        menu.select(0);
        menu
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Selects `index`, or the nearest item after it (before it at the end) that can be.
    pub fn select(&mut self, index: usize) {
        let n = self.entries.len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        let index = index.min(n - 1);
        let usable = |i: usize| !self.entries[i].separator && !self.entries[i].disabled;
        self.selected = (index..n).find(|&i| usable(i)).or_else(|| (0..index).rev().find(|&i| usable(i))).unwrap_or(index);
        self.scroll_to_selected();
    }

    /// Replaces the items, keeping the selection where it was (or at `index`).
    pub fn set_entries(&mut self, entries: Vec<Entry>, index: usize) {
        self.entries = entries;
        self.select(index);
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    fn scroll_to_selected(&mut self) {
        let rows = self.rows.max(1);
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + rows {
            self.top = self.selected + 1 - rows;
        }
    }

    /// Moves by `step` items that can be selected; round the ends (wrap mode)
    /// unless `wrap` is off.
    fn step(&mut self, step: isize, wrap: bool) {
        let n = self.entries.len() as isize;
        if n == 0 {
            return;
        }
        let mut at = self.selected as isize;
        for _ in 0..n {
            let next = at + step;
            at = if wrap { next.rem_euclid(n) } else if !(0..n).contains(&next) { return } else { next };
            let entry = &self.entries[at as usize];
            if !entry.separator && !entry.disabled {
                self.selected = at as usize;
                self.scroll_to_selected();
                return;
            }
        }
    }

    fn page(&mut self, down: bool) {
        let rows = self.rows.max(1);
        for _ in 0..rows.saturating_sub(1).max(1) {
            let before = self.selected;
            self.step(if down { 1 } else { -1 }, false);
            if self.selected == before {
                break;
            }
        }
    }

    /// The hot key of every item and where it is in the shown text.
    fn hot_keys(&self) -> Vec<Option<(char, usize)>> {
        let mut used: Vec<char> = Vec::new();
        let mut keys: Vec<Option<(char, usize)>> = vec![None; self.entries.len()];
        let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
        if !self.show_ampersand {
            for (i, entry) in self.entries.iter().enumerate() {
                if entry.separator {
                    continue;
                }
                if let Some((c, at)) = explicit_hot(&entry.text)
                    && !used.contains(&lower(c))
                {
                    used.push(lower(c));
                    keys[i] = Some((c, at));
                }
            }
        }
        if self.auto_highlight {
            for (i, entry) in self.entries.iter().enumerate() {
                if entry.separator || (!self.show_ampersand && explicit_hot(&entry.text).is_some()) {
                    continue;
                }
                let shown = self.shown(&entry.text);
                if let Some((at, c)) = shown.chars().enumerate().find(|&(_, c)| (c == '&' || c.is_alphanumeric()) && !used.contains(&lower(c))) {
                    used.push(lower(c));
                    keys[i] = Some((c, at));
                }
            }
        }
        keys
    }

    /// The text as shown: `&` marks taken out (unless shown as they are).
    fn shown(&self, text: &str) -> String {
        if self.show_ampersand { text.to_string() } else { strip_hot(text) }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let plain = !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc | KeyCode::F(10) if plain => return Outcome::Closed,
            KeyCode::Up if plain => self.step(-1, true),
            KeyCode::Down if plain => self.step(1, true),
            KeyCode::Home if plain => {
                self.selected = 0;
                self.select(0);
            }
            KeyCode::End if plain => {
                let last = self.entries.len().saturating_sub(1);
                self.selected = last;
                if self.entries.get(last).is_some_and(|e| e.separator || e.disabled) {
                    self.step(-1, false);
                }
                self.scroll_to_selected();
            }
            KeyCode::PageUp if plain => self.page(false),
            KeyCode::PageDown if plain => self.page(true),
            KeyCode::Enter if plain && !key.modifiers.contains(KeyModifiers::SHIFT) => {
                return match self.current() {
                    Some(entry) if !entry.disabled && !entry.separator => Outcome::Chosen(self.selected),
                    _ => Outcome::Open,
                };
            }
            KeyCode::Char(c) if plain && c != ' ' => {
                let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
                if let Some(index) = self.hot_keys().iter().position(|hot| hot.is_some_and(|(h, _)| lower(h) == lower(c)))
                    && !self.entries[index].disabled
                {
                    self.selected = index;
                    self.scroll_to_selected();
                    return Outcome::Chosen(index);
                }
                return Outcome::Key(key);
            }
            _ => return Outcome::Key(key),
        }
        Outcome::Open
    }

    pub fn click(&mut self, event: MouseEvent) -> Outcome {
        let (left, top, right, bottom) = self.frame;
        let (col, row) = (event.column, event.row);
        match event.kind {
            MouseEventKind::ScrollUp => {
                self.step(-1, false);
                return Outcome::Open;
            }
            MouseEventKind::ScrollDown => {
                self.step(1, false);
                return Outcome::Open;
            }
            MouseEventKind::Down(MouseButton::Left) => {}
            _ => return Outcome::Open,
        }
        if col > left && col < right && row > top && row < bottom {
            let index = self.top + (row - top - 1) as usize;
            if let Some(entry) = self.entries.get(index)
                && !entry.separator
                && !entry.disabled
            {
                self.selected = index;
                return Outcome::Chosen(index);
            }
            return Outcome::Open;
        }
        if col == right && row > top && row < bottom && self.rows < self.entries.len() {
            // the scroll bar: its arrows move a line
            if row == top + 1 {
                self.step(-1, false);
            } else if row + 1 == bottom {
                self.step(1, false);
            } else {
                self.page(row > (top + bottom) / 2);
            }
            return Outcome::Open;
        }
        if col + 2 < left || col > right + 2 || row + 1 < top || row > bottom + 1 { Outcome::Closed } else { Outcome::Open }
    }

    pub fn paint(&mut self, grid: &mut Grid) {
        let (scr_x, scr_y) = (grid.cols.saturating_sub(1), grid.rows.saturating_sub(1));
        let n = self.entries.len();
        let longest = self
            .entries
            .iter()
            .map(|e| self.shown(&e.text).chars().count() + if e.separator { 2 } else { 0 })
            .chain([self.title.chars().count() + 2, self.bottom.chars().count() + 2])
            .max()
            .unwrap_or(0) as u16;
        let submenus = self.entries.iter().any(|e| e.submenu);
        let service = 5 + u16::from(submenus);
        let menu_width = service + longest.min(scr_x.saturating_sub(service));
        let left = (scr_x.saturating_sub(menu_width) / 2).max(2);
        let right = (left + menu_width).min(scr_x.saturating_sub(2));
        let max_height = self.max_height_margin.map(|m| scr_y.saturating_sub(m) as usize).filter(|&m| m > 0);
        let visible = max_height.map_or(n, |m| n.min(m)).max(1);
        let mut top = (scr_y as usize).saturating_sub(visible + 2) as u16 / 2;
        let mut bottom = (top + visible as u16 + 1).min(scr_y);
        if top < 3 && bottom > scr_y.saturating_sub(3) {
            top = 2;
            bottom = scr_y.saturating_sub(2).max(top + 2);
        }
        self.frame = (left, top, right, bottom);
        self.rows = (bottom - top - 1) as usize;
        self.scroll_to_selected();
        if self.top + self.rows > n {
            self.top = n.saturating_sub(self.rows);
        }
        let inner = (right - left - 1) as usize;

        let margin_left = left.saturating_sub(2);
        let margin_top = top.saturating_sub(1);
        let (margin_w, margin_h) = ((right + 3).min(grid.cols) - margin_left, (bottom + 2).min(grid.rows) - margin_top);
        grid.fill_with(margin_left, margin_top, margin_w, margin_h, TEXT, BG);
        let horizontal = "\u{2550}".repeat(inner);
        grid.put(left, top, &format!("\u{2554}{horizontal}\u{2557}"), TEXT, BG);
        grid.put(left, bottom, &format!("\u{255A}{horizontal}\u{255D}"), TEXT, BG);
        for (title, y) in [(&self.title, top), (&self.bottom, bottom)] {
            if title.is_empty() {
                continue;
            }
            let shown: String = title.chars().take(inner.saturating_sub(2)).collect();
            let x = left + 1 + (inner.saturating_sub(shown.chars().count() + 2) / 2) as u16;
            grid.put(x, y, &format!(" {shown} "), TEXT, BG);
        }
        let hot = self.hot_keys();
        let text_x = left + 3;
        let text_w = (right - 1 - u16::from(submenus)).saturating_sub(text_x) as usize;
        for row in 0..self.rows {
            let y = top + 1 + row as u16;
            grid.put(left, y, "\u{2551}", TEXT, BG);
            grid.put(right, y, "\u{2551}", TEXT, BG);
            let index = self.top + row;
            let Some(entry) = self.entries.get(index) else {
                grid.fill_with(left + 1, y, inner as u16, 1, TEXT, BG);
                continue;
            };
            if entry.separator {
                let label = self.shown(&entry.text);
                let mut line: Vec<char> = "\u{2500}".repeat(inner).chars().collect();
                if !label.is_empty() {
                    let label: Vec<char> = format!(" {label} ").chars().take(inner).collect();
                    let at = (inner - label.len()) / 2;
                    line[at..at + label.len()].copy_from_slice(&label);
                }
                let line: String = line.into_iter().collect();
                grid.put(left, y, &format!("\u{255F}{line}\u{2562}"), TEXT, BG);
                continue;
            }
            let selected = index == self.selected;
            let bg = if selected { SELECTED_BG } else { BG };
            let fg = if entry.disabled { GREY } else { TEXT };
            grid.fill_with(left + 1, y, inner as u16, 1, fg, bg);
            if let Some(mark) = entry.check {
                grid.put(left + 1, y, &mark.to_string(), fg, bg);
            }
            let shown: Vec<char> = self.shown(&entry.text).chars().collect();
            for (i, c) in shown.iter().take(text_w).enumerate() {
                let is_hot = !entry.disabled && hot[index].is_some_and(|(_, at)| at == i);
                grid.put(text_x + i as u16, y, &c.to_string(), if is_hot { HOT } else { fg }, bg);
            }
            if shown.len() > text_w {
                grid.put(right - 1 - u16::from(submenus), y, "\u{00BB}", fg, bg);
            }
            if entry.submenu {
                grid.put(right - 1, y, "\u{25BA}", fg, bg);
            }
        }
        if self.rows < n && self.rows >= 2 {
            let bar = scroll_bar(self.rows, self.top, self.top + self.rows, n);
            for (i, c) in bar.chars().enumerate() {
                grid.put(right, top + 1 + i as u16, &c.to_string(), TEXT, BG);
            }
        }
        palette::drop_shadow(grid, margin_left, margin_top, margin_w, margin_h);
    }
}

/// Far's MakeScrollBarEx: ▲, the track with the slider, ▼.
fn scroll_bar(length: usize, start: usize, end: usize, size: usize) -> String {
    let mut bar = vec!['\u{2591}'; length];
    bar[0] = '\u{25B2}';
    bar[length - 1] = '\u{25BC}';
    let field = length.saturating_sub(2);
    if field > 0 && size > 0 {
        let rounded = |nom: usize| ((nom as f64 * field as f64 / size as f64).round()) as usize;
        let begin = rounded(start).max(usize::from(start > 0)).min(field - 1);
        let mut len = rounded(end.min(size) - start).max(1);
        if end < size && begin + len >= field {
            len = (field - begin).saturating_sub(1).max(1);
        }
        for slot in bar.iter_mut().skip(1 + begin).take(len.min(field - begin)) {
            *slot = '\u{2588}';
        }
    }
    bar.into_iter().collect()
}

/// The letter after a single `&` and where it is in the text without the marks.
fn explicit_hot(text: &str) -> Option<(char, usize)> {
    let mut shown = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.peek() {
                Some('&') => {
                    chars.next();
                }
                Some(&next) => return Some((next, shown)),
                None => return None,
            }
        }
        shown += 1;
    }
    None
}

/// The text without its `&` marks (`&&` is one `&`).
pub fn strip_hot(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                out.push('&');
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_like_far_menu_and_enter_chooses() {
        let mut grid = Grid::new(80, 25);
        let mut menu = VMenu::new("ArcLite", vec!["Test archive files".into()]);
        menu.paint(&mut grid);
        // 18 + 5 = 23 wide: left (79 - 23) / 2 = 28, right 51; top (24 - 3) / 2 = 10
        assert_eq!(menu.frame, (28, 10, 51, 12));
        let row: String = (28..52).map(|c| grid.row(11)[c].text.clone()).collect();
        assert_eq!(row, "\u{2551}  Test archive files  \u{2551}");
        assert_eq!(menu.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Outcome::Chosen(0));
    }

    #[test]
    fn hot_keys_scrolling_and_separators() {
        let mut entries: Vec<Entry> = (0..30).map(|i| Entry::new(format!("item {i}"))).collect();
        entries[0] = Entry::new("&Open archive");
        entries[1] = Entry::separator("04.11.26");
        let mut menu = VMenu::with("Title", entries);
        menu.max_height_margin = Some(8);
        let mut grid = Grid::new(80, 25);
        menu.paint(&mut grid);
        // ScrY 24, MaxHeight 16: 16 rows from (24 - 16 - 2) / 2 = 3
        assert_eq!((menu.frame.1, menu.frame.3), (3, 20));
        assert_eq!(menu.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)), Outcome::Chosen(0));
        menu.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(menu.selected(), 2, "the separator is passed over");
        menu.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        menu.paint(&mut grid);
        assert_eq!(menu.top, 14);
        assert_eq!(grid.row(19)[menu.frame.2 as usize].text, "\u{25BC}");
        assert!(matches!(menu.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)), Outcome::Key(_)));
    }
}
