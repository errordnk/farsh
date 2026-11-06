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
    /// The underline color (SGR 58), which a placement's cells carry its file id in.
    pub ul: Option<Color>,
}

impl Default for Cell {
    fn default() -> Self {
        Self { text: " ".into(), fg: Color::Reset, bg: Color::Reset, bold: false, underline: false, reverse: false, ul: None }
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

    /// A grid that matches no screen: flushing against it sends every cell.
    /// After a resize the terminal shows something we do not know (its own
    /// reflow of the old picture), so nothing on it may be taken as drawn.
    pub fn unknown(cols: u16, rows: u16) -> Self {
        let cell = Cell { text: "\u{FFFF}".into(), ..Cell::default() };
        Self { cols, rows, cells: vec![cell; cols as usize * rows as usize] }
    }

    /// A copy of one row's cells.
    pub fn row(&self, row: u16) -> Vec<Cell> {
        let start = row as usize * self.cols as usize;
        self.cells.get(start..start + self.cols as usize).map(<[Cell]>::to_vec).unwrap_or_default()
    }

    /// Replaces the cells of `row` with `cells` (as many as fit).
    pub fn set_row(&mut self, row: u16, cells: &[Cell]) {
        for (col, cell) in cells.iter().enumerate() {
            if let Some(target) = self.cell_mut(col as u16, row) {
                *target = cell.clone();
            }
        }
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

    /// A placement of Som's rich-content protocol over a rectangle: placeholder
    /// cells whose foreground color is the session id and whose underline color
    /// is the file id (at most 297 by 297 cells; the rest stays as it was).
    pub fn place(&mut self, col: u16, row: u16, width: u16, height: u16, ids: (u32, u32)) {
        self.place_from(col, row, width, height, 0, ids);
    }

    /// The same, showing the placement from its row `first_row` on (a document
    /// taller than the screen is scrolled by drawing a window of its rows).
    pub fn place_from(&mut self, col: u16, row: u16, width: u16, height: u16, first_row: u16, ids: (u32, u32)) {
        let color = |id| {
            let (r, g, b) = srp_client::placeholder::id_to_rgb(id);
            Color::Rgb { r, g, b }
        };
        let (fg, ul) = (color(ids.0), color(ids.1));
        for r in 0..height {
            for c in 0..width {
                let Some(text) = srp_client::placeholder::cell_text(u32::from(first_row.saturating_add(r)), u32::from(c)) else { continue };
                if let Some(cell) = self.cell_mut(col + c, row + r) {
                    *cell = Cell { text, fg, ul: Some(ul), ..Cell::default() };
                }
            }
        }
    }

    pub fn fill(&mut self, col: u16, row: u16, width: u16, height: u16, bg: Color) {
        self.fill_with(col, row, width, height, Color::Reset, bg);
    }

    /// Blanks in `fg` on `bg` (the colors Far gives a box's empty cells).
    pub fn fill_with(&mut self, col: u16, row: u16, width: u16, height: u16, fg: Color, bg: Color) {
        for r in row..row.saturating_add(height) {
            for c in col..col.saturating_add(width) {
                if let Some(cell) = self.cell_mut(c, r) {
                    *cell = Cell { fg, bg, ..Cell::default() };
                }
            }
        }
    }

    /// Sends to `out` only the cells that differ from `shown`, then updates
    /// `shown` to match.
    pub fn flush(&self, shown: &mut Grid, cursor: Option<(u16, u16)>, out: &mut impl Write) -> std::io::Result<()> {
        queue!(out, cursor::Hide)?;
        let mut last: Option<(Color, Color, bool, bool, bool, Option<Color>)> = None;
        for row in 0..self.rows {
            let mut col = 0;
            while col < self.cols {
                let index = row as usize * self.cols as usize + col as usize;
                let cell = &self.cells[index];
                let width = if cell.text.is_empty() { 1 } else { text_width(&cell.text) };
                if shown.cells[index] != *cell {
                    queue!(out, cursor::MoveTo(col, row))?;
                    let style = (cell.fg, cell.bg, cell.bold, cell.underline, cell.reverse, cell.ul);
                    if last != Some(style) {
                        queue!(out, SetAttribute(Attribute::Reset), SetForegroundColor(cell.fg), SetBackgroundColor(cell.bg))?;
                        if let Some(Color::Rgb { r, g, b }) = cell.ul {
                            // A placement's colors are its ids, not a style: they go out
                            // as they are, also where crossterm leaves colors out (NO_COLOR)
                            queue!(out, Print(format!("\x1b[58;2;{r};{g};{b}m")))?;
                            if let Color::Rgb { r, g, b } = cell.fg {
                                queue!(out, Print(format!("\x1b[38;2;{r};{g};{b}m")))?;
                            }
                        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_placement_is_placeholder_cells_with_the_ids_in_their_colors() {
        let mut grid = Grid::new(6, 4);
        grid.place(1, 1, 3, 2, (0x01_02_03, 0x0A_0B_0C));
        let cell = grid.cell_mut(1, 1).unwrap().clone();
        assert_eq!(cell.text, "\u{10EEEE}\u{0305}\u{0305}");
        assert_eq!(cell.fg, Color::Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(cell.ul, Some(Color::Rgb { r: 10, g: 11, b: 12 }));
        assert_eq!(grid.cell_mut(3, 2).unwrap().text, "\u{10EEEE}\u{030D}\u{030E}");
        // outside the rectangle nothing changed
        assert_eq!(grid.cell_mut(0, 0).unwrap().text, " ");
        assert_eq!(grid.cell_mut(4, 1).unwrap().text, " ");
    }

    #[test]
    fn a_window_of_a_placement_starts_at_its_row() {
        let mut grid = Grid::new(4, 3);
        grid.place_from(0, 0, 2, 2, 5, (1, 2));
        // the first drawn row is the placement's row 5, the next its row 6
        assert_eq!(grid.cell_mut(0, 0).unwrap().text, srp_client::placeholder::cell_text(5, 0).unwrap());
        assert_eq!(grid.cell_mut(1, 1).unwrap().text, srp_client::placeholder::cell_text(6, 1).unwrap());
        assert_eq!(grid.cell_mut(0, 2).unwrap().text, " ");
    }

    #[test]
    fn the_underline_color_reaches_the_terminal_as_sgr_58() {
        let mut grid = Grid::new(3, 1);
        grid.place(0, 0, 2, 1, (1, 0x0A_0B_0C));
        let mut shown = Grid::unknown(3, 1);
        let mut out = Vec::new();
        grid.flush(&mut shown, None, &mut out).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("\x1b[58;2;10;11;12m"), "{text:?}");
        // the session id is the foreground color, there even under NO_COLOR
        assert!(text.contains("\x1b[38;2;0;0;1m"), "{text:?}");
        assert!(text.contains('\u{10EEEE}'));
        // flushed again nothing is sent: a placement stays on the screen as it is
        let mut again = Vec::new();
        grid.flush(&mut shown, None, &mut again).unwrap();
        assert!(!String::from_utf8_lossy(&again).contains('\u{10EEEE}'));
    }
}
