//! Far's "Sort by" menu (Ctrl+F12, filelist.cpp `SelectSortMode`), drawn as
//! Far draws a VMenu: a double box on cyan with a margin, a shadow, the
//! current mode checked with its direction, hotkeys in yellow.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::grid::Grid;
use crate::palette;
use crate::sorting::{SortMode, Sorting};

#[allow(non_snake_case)]
fn BG() -> Color {
    crate::colors::bg("Menu.Text")
}
#[allow(non_snake_case)]
fn TEXT() -> Color {
    crate::colors::fg("Menu.Text")
}
#[allow(non_snake_case)]
fn HOT() -> Color {
    crate::colors::fg("Menu.Highlight")
}
#[allow(non_snake_case)]
fn SELECTED_BG() -> Color {
    crate::colors::bg("Menu.Text.Selected")
}

const TITLE: &str = "Sort by";
const BOTTOM: &str = "+ - * F4";

/// What a line of the menu stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    Mode(SortMode),
    /// 0: sort groups, 1: selected first, 2: directories first.
    Option(usize),
}

/// A choice and how it was made: Enter (or a hotkey) flips/toggles, `+`
/// and `-` set ascending/on or descending/off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picked {
    pub choice: Choice,
    pub invert: bool,
    pub plus: bool,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Open,
    Closed,
    Picked(Picked),
}

struct Line {
    label: &'static str,
    accel: &'static str,
    check: Option<char>,
    choice: Option<Choice>,
}

pub struct SortMenu {
    lines: Vec<Line>,
    selected: usize,
    /// Left column of the panel it belongs to.
    panel_left: u16,
    top: usize,
    /// The box at the last paint: left, top, right, bottom.
    frame: (u16, u16, u16, u16),
}

fn accel(mode: SortMode) -> &'static str {
    match mode.key() {
        Some(3) => "Ctrl+F3",
        Some(4) => "Ctrl+F4",
        Some(5) => "Ctrl+F5",
        Some(6) => "Ctrl+F6",
        Some(7) => "Ctrl+F7",
        Some(8) => "Ctrl+F8",
        Some(9) => "Ctrl+F9",
        Some(10) => "Ctrl+F10",
        Some(11) => "Ctrl+F11",
        _ => "",
    }
}

/// A label without its `&`, and the hotkey's place in it.
fn plain(label: &str) -> (String, Option<usize>) {
    match label.find('&') {
        Some(at) => (label.replacen('&', "", 1), Some(label[..at].chars().count())),
        None => (label.to_string(), None),
    }
}

impl SortMenu {
    pub fn new(sorting: &Sorting, panel_left: u16) -> Self {
        let mut lines: Vec<Line> = SortMode::MENU
            .iter()
            .map(|&mode| Line {
                label: crate::lang::tr(mode.label()),
                accel: accel(mode),
                check: (mode == sorting.mode).then_some(if sorting.reverse { '\u{25BC}' } else { '\u{25B2}' }),
                choice: Some(Choice::Mode(mode)),
            })
            .collect();
        lines.push(Line { label: "", accel: "", check: None, choice: None });
        let options = [
            ("Use sort groups", "Shift+F11", sorting.groups),
            ("Show selected f&irst", "Shift+F12", sorting.selected_first),
            ("Show directories &first", "", sorting.directories_first),
        ];
        for (i, (label, accel, on)) in options.into_iter().enumerate() {
            lines.push(Line { label, accel, check: on.then_some('\u{221A}'), choice: Some(Choice::Option(i)) });
        }
        let selected = SortMode::MENU.iter().position(|&m| m == sorting.mode).unwrap_or(0);
        Self { lines, selected, panel_left, top: 0, frame: (0, 0, 0, 0) }
    }

    fn pick(&self, index: usize, invert: bool, plus: bool) -> Outcome {
        match self.lines.get(index).and_then(|l| l.choice) {
            Some(choice) => Outcome::Picked(Picked { choice, invert, plus }),
            None => Outcome::Open,
        }
    }

    /// Moves by `step` over the lines, wrapping round and stepping over the separator.
    fn step(&mut self, step: isize) {
        let n = self.lines.len() as isize;
        let mut at = self.selected as isize;
        loop {
            at = (at + step).rem_euclid(n);
            if self.lines[at as usize].choice.is_some() {
                break;
            }
        }
        self.selected = at as usize;
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers == KeyModifiers::CONTROL;
        let shift = key.modifiers == KeyModifiers::SHIFT;
        match key.code {
            KeyCode::Esc | KeyCode::F(10) => return Outcome::Closed,
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            KeyCode::Home | KeyCode::PageUp => self.selected = 0,
            KeyCode::End | KeyCode::PageDown => self.selected = self.lines.len() - 1,
            KeyCode::Enter => return self.pick(self.selected, true, false),
            // `=` and `8`: the + and * keys without Shift
            KeyCode::Char('+' | '=') => return self.pick(self.selected, false, true),
            KeyCode::Char('-') => return self.pick(self.selected, false, false),
            KeyCode::Char('*' | '8') => return self.pick(self.selected, true, false),
            KeyCode::F(n) if ctrl => {
                if let Some(index) = SortMode::MENU.iter().position(|m| m.key() == Some(n)) {
                    return self.pick(index, true, false);
                }
            }
            KeyCode::F(n @ (11 | 12)) if shift => {
                let option = Choice::Option(if n == 11 { 0 } else { 1 });
                if let Some(index) = self.lines.iter().position(|l| l.choice == Some(option)) {
                    return self.pick(index, true, false);
                }
            }
            KeyCode::Char(c) if !ctrl => {
                let c = c.to_lowercase().next().unwrap_or(c);
                let hit = self.lines.iter().position(|l| {
                    let (text, hot) = plain(l.label);
                    hot.and_then(|h| text.chars().nth(h)).is_some_and(|h| crate::keys::same(c, h))
                });
                if let Some(index) = hit {
                    self.selected = index;
                    return self.pick(index, true, false);
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    pub fn click(&mut self, event: MouseEvent) -> Outcome {
        let (left, top, right, bottom) = self.frame;
        let (col, row) = (event.column, event.row);
        match event.kind {
            MouseEventKind::ScrollUp => self.step(-1),
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::Down(MouseButton::Left) => {
                if col > left && col < right && row > top && row < bottom {
                    let index = self.top + (row - top - 1) as usize;
                    if self.lines.get(index).is_some_and(|l| l.choice.is_some()) {
                        self.selected = index;
                        return self.pick(index, true, false);
                    }
                    return Outcome::Open;
                }
                if col + 2 < left || col > right + 2 || row + 1 < top || row > bottom + 1 {
                    return Outcome::Closed;
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    pub fn paint(&mut self, grid: &mut Grid) {
        let text_width = self.lines.iter().map(|l| plain(l.label).0.chars().count()).max().unwrap_or(0);
        let accel_width = self.lines.iter().map(|l| l.accel.len()).max().unwrap_or(0);
        let inner = 2 + text_width + 2 + accel_width + 1;
        let width = inner as u16 + 2;
        // as many lines as the screen leaves room for (the margin above, both
        // borders), the rest scrolled
        let room = grid.rows.saturating_sub(3) as usize;
        let shown = self.lines.len().min(room.max(1));
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + shown {
            self.top = self.selected + 1 - shown;
        }
        let height = shown as u16 + 2;
        let mut left = self.panel_left + 4;
        // Far keeps the right margin and one column of the shadow on the screen
        if left + width + 3 > grid.cols {
            left = grid.cols.saturating_sub(width + 3);
        }
        let top = (grid.rows.saturating_sub(height) / 2).max(1);
        let (right, bottom) = (left + width - 1, top + height - 1);
        self.frame = (left, top, right, bottom);

        let margin_left = left.saturating_sub(2);
        grid.fill_with(margin_left, top.saturating_sub(1), right + 3 - margin_left, height + 2, TEXT(), BG());
        let horizontal = "\u{2550}".repeat(inner);
        grid.put(left, top, &format!("\u{2554}{horizontal}\u{2557}"), TEXT(), BG());
        grid.put(left, bottom, &format!("\u{255A}{horizontal}\u{255D}"), TEXT(), BG());
        for (title, y) in [(crate::lang::tr(TITLE), top), (BOTTOM, bottom)] {
            let title = format!(" {title} ");
            let x = left + 1 + (inner.saturating_sub(title.chars().count()) / 2) as u16;
            grid.put(x, y, &title, TEXT(), BG());
        }
        for row in 0..shown {
            let y = top + 1 + row as u16;
            let index = self.top + row;
            let line = &self.lines[index];
            grid.put(left, y, "\u{2551}", TEXT(), BG());
            grid.put(right, y, "\u{2551}", TEXT(), BG());
            if line.choice.is_none() {
                grid.put(left, y, &format!("\u{255F}{}\u{2562}", "\u{2500}".repeat(inner)), TEXT(), BG());
                continue;
            }
            let bg = if index == self.selected { SELECTED_BG() } else { BG() };
            let (label, hot) = plain(line.label);
            let text = format!(
                "{} {label:<text_width$}  {:<accel_width$} ",
                line.check.unwrap_or(' '),
                line.accel,
                text_width = text_width,
                accel_width = accel_width
            );
            grid.put(left + 1, y, &text, TEXT(), bg);
            if let Some(h) = hot {
                let c: String = label.chars().nth(h).into_iter().collect();
                grid.put(left + 3 + h as u16, y, &c, HOT(), bg);
            }
        }
        if shown < self.lines.len() {
            let bar = scroll_bar(shown, self.top, self.lines.len());
            for (i, c) in bar.chars().enumerate() {
                grid.put(right, top + 1 + i as u16, &c.to_string(), TEXT(), BG());
            }
        }
        let margin_top = top.saturating_sub(1);
        palette::drop_shadow(grid, margin_left, margin_top, right + 3 - margin_left, bottom + 2 - margin_top);
    }
}

/// Far's `MakeScrollBarEx`: arrows at the ends, the slider over the shaded field.
fn scroll_bar(length: usize, start: usize, size: usize) -> String {
    let mut bar = vec!['\u{2591}'; length.max(2)];
    bar[0] = '\u{25B2}';
    let last = bar.len() - 1;
    bar[last] = '\u{25BC}';
    let field = bar.len() - 2;
    if field == 0 {
        return bar.into_iter().collect();
    }
    let end = (start + length).min(size);
    let rounded = |nom: usize| ((nom * field * 10 / size) as f64 / 10.0).round() as usize;
    let mut begin = rounded(start).max(usize::from(start > 0));
    if begin == field {
        begin -= 1;
    }
    let slider = rounded(end - start).max(1);
    let mut finish = if end == size { field } else { begin + slider };
    if finish == field && end < size {
        finish -= 1;
        if begin > 1 {
            begin -= 1;
        }
    }
    for c in bar.iter_mut().skip(1 + begin).take(finish.saturating_sub(begin)) {
        *c = '\u{2588}';
    }
    bar.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn the_current_mode_is_checked_and_selected() {
        let menu = SortMenu::new(&Sorting::default(), 0);
        assert_eq!(menu.lines[menu.selected].choice, Some(Choice::Mode(SortMode::Extension)));
        assert_eq!(menu.lines[menu.selected].check, Some('\u{25B2}'));
        assert_eq!(menu.lines[18].check, Some('\u{221A}'), "directories first is on");
    }

    #[test]
    fn keys_pick_like_far() {
        let mut menu = SortMenu::new(&Sorting::default(), 0);
        assert_eq!(
            menu.key(key(KeyCode::Char('w'))),
            Outcome::Picked(Picked { choice: Choice::Mode(SortMode::Write), invert: true, plus: false })
        );
        let mut menu = SortMenu::new(&Sorting::default(), 0);
        menu.key(key(KeyCode::Up));
        menu.key(key(KeyCode::Up));
        menu.key(key(KeyCode::Up)); // wraps to the last line
        assert_eq!(menu.lines[menu.selected].choice, Some(Choice::Option(2)));
        menu.key(key(KeyCode::Up));
        menu.key(key(KeyCode::Up));
        menu.key(key(KeyCode::Up)); // over the separator
        assert_eq!(menu.lines[menu.selected].choice, Some(Choice::Mode(SortMode::StreamsSize)));
        assert_eq!(
            menu.key(key(KeyCode::Char('-'))),
            Outcome::Picked(Picked { choice: Choice::Mode(SortMode::StreamsSize), invert: false, plus: false })
        );
        // = and 8: the + and * keys without Shift
        assert_eq!(
            menu.key(key(KeyCode::Char('='))),
            Outcome::Picked(Picked { choice: Choice::Mode(SortMode::StreamsSize), invert: false, plus: true })
        );
        assert_eq!(
            menu.key(key(KeyCode::Char('8'))),
            Outcome::Picked(Picked { choice: Choice::Mode(SortMode::StreamsSize), invert: true, plus: false })
        );
        let ctrl_f6 = KeyEvent::new(KeyCode::F(6), KeyModifiers::CONTROL);
        assert!(matches!(menu.key(ctrl_f6), Outcome::Picked(Picked { choice: Choice::Mode(SortMode::Size), .. })));
        assert_eq!(menu.key(key(KeyCode::Esc)), Outcome::Closed);
    }

    #[test]
    fn painting_matches_far_geometry() {
        let mut menu = SortMenu::new(&Sorting::default(), 0);
        let mut grid = Grid::new(100, 30);
        menu.paint(&mut grid);
        assert_eq!(menu.frame, (4, 4, 41, 24));
        assert_eq!(grid.cell_mut(4, 4).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(5, 7).unwrap().text, "\u{25B2}");
    }
}
