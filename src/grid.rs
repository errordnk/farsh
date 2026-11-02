//! A screen of cells with a diff against what the terminal shows now.

use std::io::Write;

use crossterm::style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::{cursor, queue};

#[derive(Clone, PartialEq)]
pub struct Cell {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self { text: " ".into(), fg: Color::Reset, bg: Color::Reset, bold: false, underline: false, reverse: false }
    }
}

pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    cells: Vec<Cell>,
}

impl Grid {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows, cells: vec![Cell::default(); cols as usize * rows as usize] }
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
    }

    pub fn cell_mut(&mut self, col: u16, row: u16) -> Option<&mut Cell> {
        (col < self.cols && row < self.rows).then(|| &mut self.cells[row as usize * self.cols as usize + col as usize])
    }

    /// Writes `text` from (`col`, `row`) in one style; stops at the edge.
    pub fn put(&mut self, mut col: u16, row: u16, text: &str, fg: Color, bg: Color) {
        for ch in text.chars() {
            if let Some(cell) = self.cell_mut(col, row) {
                *cell = Cell { text: ch.to_string(), fg, bg, ..Cell::default() };
            }
            col += 1;
        }
    }

    /// Paints the rest of `row`, from `col` on, in `bg`.
    pub fn fill_from(&mut self, col: u16, row: u16, bg: Color) {
        self.fill(col, row, self.cols.saturating_sub(col), 1, bg);
    }

    pub fn fill(&mut self, col: u16, row: u16, width: u16, height: u16, bg: Color) {
        for r in row..row.saturating_add(height) {
            for c in col..col.saturating_add(width) {
                if let Some(cell) = self.cell_mut(c, r) {
                    *cell = Cell { bg, ..Cell::default() };
                }
            }
        }
    }

    /// Sends to `out` only the cells that differ from `shown`, then updates
    /// `shown` to match.
    pub fn flush(&self, shown: &mut Grid, cursor: Option<(u16, u16)>, out: &mut impl Write) -> std::io::Result<()> {
        queue!(out, cursor::Hide)?;
        let mut last: Option<(Color, Color, bool, bool, bool)> = None;
        for row in 0..self.rows {
            let mut col = 0;
            while col < self.cols {
                let index = row as usize * self.cols as usize + col as usize;
                let cell = &self.cells[index];
                let width = if cell.text.is_empty() { 1 } else { text_width(&cell.text) };
                if shown.cells[index] != *cell {
                    queue!(out, cursor::MoveTo(col, row))?;
                    let style = (cell.fg, cell.bg, cell.bold, cell.underline, cell.reverse);
                    if last != Some(style) {
                        queue!(out, SetAttribute(Attribute::Reset), SetForegroundColor(cell.fg), SetBackgroundColor(cell.bg))?;
                        if cell.bold {
                            queue!(out, SetAttribute(Attribute::Bold))?;
                        }
                        if cell.underline {
                            queue!(out, SetAttribute(Attribute::Underlined))?;
                        }
                        if cell.reverse {
                            queue!(out, SetAttribute(Attribute::Reverse))?;
                        }
                        last = Some(style);
                    }
                    queue!(out, Print(if cell.text.is_empty() { " " } else { &cell.text }))?;
                }
                col += width.max(1);
            }
        }
        queue!(out, SetAttribute(Attribute::Reset))?;
        if let Some((col, row)) = cursor {
            queue!(out, cursor::MoveTo(col, row), cursor::Show)?;
        }
        shown.cells.clone_from(&self.cells);
        Ok(())
    }
}

/// Columns a cell's text takes: 2 for the common wide (CJK, emoji) ranges.
fn text_width(text: &str) -> u16 {
    match text.chars().next() {
        Some(c)
            if matches!(
                c as u32,
                0x1100..=0x115F
                    | 0x2E80..=0xA4CF
                    | 0xAC00..=0xD7A3
                    | 0xF900..=0xFAFF
                    | 0xFE30..=0xFE6F
                    | 0xFF00..=0xFF60
                    | 0xFFE0..=0xFFE6
                    | 0x1F300..=0x1FAFF
                    | 0x20000..=0x3FFFD
            ) =>
        {
            2
        }
        _ => 1,
    }
}
