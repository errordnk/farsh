//! A plain Far menu centered on the screen (what a plugin's Far::menu
//! shows, vmenu.cpp's Show with the position -1, -1): a double box on cyan
//! with a margin and a shadow, the service area of VMenu (box, check mark,
//! the two scroll marks and one more column: 5) around the longest item
//! or the title with a space on each side.

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::grid::Grid;
use crate::palette;

const BG: Color = palette::bg(palette::CYAN);
const TEXT: Color = palette::fg(palette::WHITE);
const SELECTED_BG: Color = palette::bg(palette::BLACK);

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Closed,
    Chosen(usize),
}

pub struct VMenu {
    title: String,
    items: Vec<String>,
    selected: usize,
    /// left, top, right, bottom at the last paint
    frame: (u16, u16, u16, u16),
}

impl VMenu {
    pub fn new(title: &str, items: Vec<String>) -> Self {
        Self { title: title.into(), items, selected: 0, frame: (0, 0, 0, 0) }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let n = self.items.len();
        match key.code {
            KeyCode::Esc | KeyCode::F(10) => return Outcome::Closed,
            KeyCode::Up => self.selected = (self.selected + n - 1) % n,
            KeyCode::Down => self.selected = (self.selected + 1) % n,
            KeyCode::Home | KeyCode::PageUp => self.selected = 0,
            KeyCode::End | KeyCode::PageDown => self.selected = n - 1,
            KeyCode::Enter => return Outcome::Chosen(self.selected),
            _ => {}
        }
        Outcome::Open
    }

    pub fn click(&mut self, event: MouseEvent) -> Outcome {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Outcome::Open;
        }
        let (left, top, right, bottom) = self.frame;
        let (col, row) = (event.column, event.row);
        if col > left && col < right && row > top && row < bottom {
            self.selected = (row - top - 1) as usize;
            return Outcome::Chosen(self.selected);
        }
        if col + 2 < left || col > right + 2 || row + 1 < top || row > bottom + 1 { Outcome::Closed } else { Outcome::Open }
    }

    pub fn paint(&mut self, grid: &mut Grid) {
        let longest = self.items.iter().map(|i| i.chars().count()).max().unwrap_or(0).max(self.title.chars().count() + 2) as u16;
        let menu_width = 5 + longest;
        let (scr_x, scr_y) = (grid.cols.saturating_sub(1), grid.rows.saturating_sub(1));
        let left = (scr_x.saturating_sub(menu_width) / 2).max(2);
        let right = (left + menu_width).min(scr_x.saturating_sub(2));
        let top = scr_y.saturating_sub(self.items.len() as u16 + 2) / 2;
        let bottom = top + self.items.len() as u16 + 1;
        self.frame = (left, top, right, bottom);
        let inner = (right - left - 1) as usize;

        let margin_left = left.saturating_sub(2);
        let margin_top = top.saturating_sub(1);
        grid.fill_with(margin_left, margin_top, right + 3 - margin_left, bottom + 2 - margin_top, TEXT, BG);
        let horizontal = "\u{2550}".repeat(inner);
        grid.put(left, top, &format!("\u{2554}{horizontal}\u{2557}"), TEXT, BG);
        grid.put(left, bottom, &format!("\u{255A}{horizontal}\u{255D}"), TEXT, BG);
        let title = format!(" {} ", self.title);
        grid.put(left + 1 + (inner.saturating_sub(title.chars().count()) / 2) as u16, top, &title, TEXT, BG);
        for (i, item) in self.items.iter().enumerate() {
            let y = top + 1 + i as u16;
            grid.put(left, y, "\u{2551}", TEXT, BG);
            grid.put(right, y, "\u{2551}", TEXT, BG);
            let bg = if i == self.selected { SELECTED_BG } else { BG };
            grid.fill_with(left + 1, y, inner as u16, 1, TEXT, bg);
            grid.put(left + 3, y, item, TEXT, bg);
        }
        palette::drop_shadow(grid, margin_left, margin_top, right + 3 - margin_left, bottom + 2 - margin_top);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

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
}
