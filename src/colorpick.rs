//! Options > Colors (setcolor.cpp): "Color groups" at (2, 1), each group's
//! "Group items" 10 columns right and 5 rows down of it (a submenu goes
//! further), and Far's color dialog (color_picker.cpp's GetColorDialog, 66
//! by 19): the foreground's 16 colors and Default, the background's, a
//! sample, Set / Reset / Cancel. farsh's terminal colors are the 16
//! console ones: the code, 256-color and RGB fields and the styles are
//! shown disabled.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::colordata::{Item, GROUPS};
use crate::grid::Grid;
use crate::lang::tr;
use crate::palette;
use crate::vmenu::{Entry, VMenu};

/// "Color groups": the groups, Themes, Palette.
pub fn groups_menu() -> VMenu {
    let mut entries: Vec<Entry> = GROUPS.iter().map(|(name, _)| Entry::new(tr(name))).collect();
    entries.push(Entry::separator(""));
    entries.push(Entry::new(tr("Themes")));
    entries.push(Entry::separator(""));
    entries.push(Entry::new(tr("Palette")));
    let mut menu = VMenu::with("Color groups", entries);
    menu.at = Some((2, 1));
    menu
}

/// What a row of the groups menu is.
pub enum GroupRow {
    Group(&'static [Item]),
    Themes,
    Palette,
}

pub fn group_row(index: usize) -> Option<GroupRow> {
    let n = GROUPS.len();
    match index {
        i if i < n => Some(GroupRow::Group(GROUPS[i].1)),
        i if i == n + 1 => Some(GroupRow::Themes),
        i if i == n + 3 => Some(GroupRow::Palette),
        _ => None,
    }
}

/// "Group items" (SetItemColors), at `at`.
pub fn items_menu(items: &'static [Item], at: (u16, u16)) -> VMenu {
    let entries = items
        .iter()
        .map(|item| match item {
            Item::Color(label, _) | Item::Sub(label, _) => Entry::new(tr(label)),
        })
        .collect();
    let mut menu = VMenu::with("Group items", entries);
    menu.at = Some(at);
    menu
}

pub fn themes_menu() -> VMenu {
    let mut menu = VMenu::with("Themes", vec![Entry::new(tr("Default")), Entry::new(tr("Default (RGB)"))]);
    menu.at = Some((2, 1));
    menu
}

/// The focus in the color dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Fore,
    Back,
    Button(usize),
}

pub enum Outcome {
    Open,
    Closed,
    /// Set: the colors chosen (foreground, background).
    Set(u8, u8),
    Reset,
}

/// The 16 colors' order in the cells: the dark ones on top, the bright below.
const CELLS: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// The Default cell (the terminal's own color).
const DEFAULT: u8 = 16;

pub struct Picker {
    pub name: &'static str,
    /// A highlighting group's color (0..7), not a palette one.
    pub slot: Option<usize>,
    fore: u8,
    back: u8,
    focus: Focus,
    /// The box's left and top at the last paint.
    origin: (u16, u16),
}

const W: u16 = 66;
const H: u16 = 19;
const BUTTONS: [(&str, &str); 3] = [("Set", "set"), ("&Reset", "reset"), ("Cancel", "cancel")];

impl Picker {
    pub fn new(name: &'static str) -> Self {
        let (f, b) = crate::colors::attr(name);
        // the terminal's default colors are the Default cells
        let fore = if f == palette::LIGHT_GRAY { DEFAULT } else { f };
        let back = if b == palette::BLACK { DEFAULT } else { b };
        Self { name, slot: None, fore, back, focus: Focus::Fore, origin: (0, 0) }
    }

    /// A color that is not the palette's (black on black: the default).
    pub fn with(f: u8, b: u8) -> Self {
        let both_default = (f, b) == (0, 0);
        let fore = if both_default || f == palette::LIGHT_GRAY { DEFAULT } else { f };
        let back = if both_default || b == palette::BLACK { DEFAULT } else { b };
        Self { name: "", slot: None, fore, back, focus: Focus::Fore, origin: (0, 0) }
    }

    fn value(cell: u8, default: u8) -> u8 {
        if cell == DEFAULT { default } else { cell }
    }

    /// Where the cell of `color` is, in the box (x, y), for a plane at `top`.
    fn cell_place(color: u8, top: u16) -> (u16, u16) {
        if color == DEFAULT {
            return (5, top + 3);
        }
        let i = CELLS.iter().position(|&c| c == color).unwrap_or(0) as u16;
        (5 + 3 * (i % 8), top + 1 + i / 8)
    }

    fn step(color: u8, dx: i16, dy: i16) -> u8 {
        if color == DEFAULT {
            return if dy < 0 { 8 } else { DEFAULT };
        }
        let i = CELLS.iter().position(|&c| c == color).unwrap_or(0) as i16;
        let (mut x, mut y) = (i % 8, i / 8);
        x = (x + dx).rem_euclid(8);
        y += dy;
        if y > 1 {
            return DEFAULT;
        }
        CELLS[(y.clamp(0, 1) * 8 + x) as usize]
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let plane = match self.focus {
            Focus::Fore => Some(&mut self.fore),
            Focus::Back => Some(&mut self.back),
            Focus::Button(_) => None,
        };
        match key.code {
            KeyCode::Esc => return Outcome::Closed,
            KeyCode::Tab | KeyCode::BackTab => {
                let order = [Focus::Fore, Focus::Back, Focus::Button(0), Focus::Button(1), Focus::Button(2)];
                let at = order.iter().position(|f| *f == self.focus).unwrap_or(0);
                let back = key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
                self.focus = order[(at + if back { order.len() - 1 } else { 1 }) % order.len()];
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down if plane.is_some() => {
                let (dx, dy) = match key.code {
                    KeyCode::Left => (-1, 0),
                    KeyCode::Right => (1, 0),
                    KeyCode::Up => (0, -1),
                    _ => (0, 1),
                };
                if let Some(cell) = plane {
                    *cell = Self::step(*cell, dx, dy);
                }
            }
            KeyCode::Left | KeyCode::Right => {
                if let Focus::Button(i) = self.focus {
                    self.focus = Focus::Button(if key.code == KeyCode::Left { (i + 2) % 3 } else { (i + 1) % 3 });
                }
            }
            KeyCode::Enter => {
                return match self.focus {
                    Focus::Button(1) => Outcome::Reset,
                    Focus::Button(2) => Outcome::Closed,
                    _ => Outcome::Set(Self::value(self.fore, palette::LIGHT_GRAY), Self::value(self.back, palette::BLACK)),
                };
            }
            KeyCode::Char(c) if crate::keys::same(c, 'r') => return Outcome::Reset,
            _ => {}
        }
        Outcome::Open
    }

    pub fn click(&mut self, event: MouseEvent) -> Outcome {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Outcome::Open;
        }
        let (x0, y0) = self.origin;
        let (Some(x), Some(y)) = (event.column.checked_sub(x0), event.row.checked_sub(y0)) else { return Outcome::Closed };
        if x >= W || y >= H {
            return Outcome::Closed;
        }
        for (top, focus) in [(2u16, Focus::Fore), (7, Focus::Back)] {
            for color in (0..16).chain([DEFAULT]) {
                let (cx, cy) = Self::cell_place(color, top);
                if y == cy && x >= cx && x < cx + 3 {
                    self.focus = focus;
                    match focus {
                        Focus::Fore => self.fore = color,
                        _ => self.back = color,
                    }
                    return Outcome::Open;
                }
            }
        }
        if y == 16
            && let Some(i) = self.button_spans().iter().position(|&(bx, bw)| x >= bx && x < bx + bw)
        {
            self.focus = Focus::Button(i);
            return self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        Outcome::Open
    }

    fn button_spans(&self) -> Vec<(u16, u16)> {
        let widths: Vec<u16> = BUTTONS.iter().map(|(l, _)| crate::vmenu::strip_hot(tr(l)).chars().count() as u16 + 4).collect();
        let total: u16 = widths.iter().sum::<u16>() + widths.len() as u16 - 1;
        let mut x = (W - total) / 2;
        widths
            .iter()
            .map(|w| {
                let span = (x, *w);
                x += w + 1;
                span
            })
            .collect()
    }

    pub fn paint(&mut self, grid: &mut Grid) {
        use crate::colors::{bg, fg};
        let (bgc, fgc) = (bg("Dialog.Text"), fg("Dialog.Text"));
        let dim = fg("Dialog.Disabled");
        let x0 = grid.cols.saturating_sub(W) / 2;
        let y0 = grid.rows.saturating_sub(H) / 2;
        self.origin = (x0, y0);
        grid.fill_with(x0, y0, W, H, fgc, bgc);
        palette::drop_shadow(grid, x0, y0, W, H);
        let put = |grid: &mut Grid, x: u16, y: u16, text: &str, f| grid.put(x0 + x, y0 + y, text, f, bgc);
        // the frame (3, 1) .. (62, 17), the separators and the line before the styles
        let inner = "\u{2550}".repeat(58);
        put(grid, 3, 1, &format!("\u{2554}{inner}\u{2557}"), fgc);
        put(grid, 3, 17, &format!("\u{255A}{inner}\u{255D}"), fgc);
        for y in 2..17 {
            put(grid, 3, y, "\u{2551}", fgc);
            put(grid, 62, y, "\u{2551}", fgc);
        }
        let title = format!(" {} ", tr("Color"));
        put(grid, 3 + (60 - title.chars().count() as u16) / 2, 1, &title, fgc);
        put(grid, 3, 15, &format!("\u{255F}{}\u{2562}", "\u{2500}".repeat(58)), fgc);
        for y in 2..15 {
            put(grid, 39, y, "\u{2502}", fgc);
        }
        put(grid, 39, 1, "\u{2564}", fgc);
        put(grid, 39, 15, "\u{2534}", fgc);
        for y in [6, 11] {
            put(grid, 3, y, &format!("\u{255F}{}\u{2524}", "\u{2500}".repeat(35)), fgc);
        }
        // the planes
        for (top, label, cell, focus) in [(2u16, "&Foreground:", self.fore, Focus::Fore), (7, "&Background:", self.back, Focus::Back)] {
            let label = crate::vmenu::strip_hot(tr(label));
            put(grid, 5, top, &label, if self.focus == focus { fg("Dialog.Text.Highlight") } else { fgc });
            for color in 0..16u8 {
                let (cx, cy) = Self::cell_place(color, top);
                let mark = if cell == color { "(\u{2022})" } else { "( )" };
                let ink = if matches!(color, 0..=6 | 8) { palette::fg(palette::WHITE) } else { palette::fg(palette::BLACK) };
                grid.put(x0 + cx, y0 + cy, mark, ink, palette::bg(color));
            }
            let (cx, cy) = Self::cell_place(DEFAULT, top);
            put(grid, cx, cy, if cell == DEFAULT { "(\u{2022})" } else { "( )" }, fgc);
            put(grid, cx + 4, cy, tr("Default"), fgc);
            // the code, 256 colors and RGB: not in a 16-color terminal
            put(grid, 30, top, tr("Index:"), dim);
            let shown = if cell == DEFAULT { "Default".to_string() } else { format!("{cell:>8}") };
            put(grid, 30, top + 1, &shown, dim);
            put(grid, 30, top + 2, "[ 256 ]", dim);
            put(grid, 30, top + 3, "[ RGB ]", dim);
        }
        // the styles, disabled
        put(grid, 41, 2, tr("Style:"), dim);
        for (i, label) in ["Inherit", "Bold", "Italic", "Overline", "Strikeout", "Faint", "Blink", "Inverse", "Invisible"].iter().enumerate() {
            let y = if i == 0 { 3 } else { 4 + i as u16 };
            put(grid, 41, y, &format!("[ ] {}", tr(label)), dim);
        }
        // the sample
        let (f, b) = (Self::value(self.fore, palette::LIGHT_GRAY), Self::value(self.back, palette::BLACK));
        // SampleX .. SampleX + SampleW
        let sample: String = tr("Text Text Text Text Text Text Text").chars().take(33).collect();
        for y in 12..15 {
            grid.put(x0 + 5, y0 + y, &sample, palette::fg(f), palette::bg(b));
        }
        // the buttons
        for (i, ((label, _), (bx, _))) in BUTTONS.iter().zip(self.button_spans()).enumerate() {
            let focused = self.focus == Focus::Button(i) || (i == 0 && !matches!(self.focus, Focus::Button(_)));
            let (open, close) = if i == 0 { ('{', '}') } else { ('[', ']') };
            let (f, b) = if self.focus == Focus::Button(i) { (fg("Dialog.Button.Selected"), bg("Dialog.Button.Selected")) } else { (fgc, bgc) };
            let _ = focused;
            grid.put(x0 + bx, y0 + 16, &format!("{open} {} {close}", crate::vmenu::strip_hot(tr(label))), f, b);
        }
    }
}
