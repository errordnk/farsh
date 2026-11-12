//! The colors Far Manager 3 reaches the terminal with. Far paints with
//! console attributes; through ConPTY they arrive as the classic console
//! table in RGB, except that light gray text (7) and a black background (0)
//! — the console's default attribute — arrive as the terminal's own default
//! colors. Measured in Som: blue is (0,0,128), cyan (0,128,128), light cyan
//! (0,255,255); the key bar numbers take Som's own foreground and background.
//! That is the `default` theme; another theme (`src/theme.rs`) has its own 16
//! colors and its own default attribute.

use crossterm::style::Color;

use crate::grid::Grid;

pub const BLACK: u8 = 0;
#[allow(dead_code)]
pub const BLUE: u8 = 1;
pub const CYAN: u8 = 3;
#[allow(dead_code)]
pub const RED: u8 = 4;
pub const BROWN: u8 = 6;
pub const LIGHT_GRAY: u8 = 7;
pub const DARK_GRAY: u8 = 8;
pub const LIGHT_BLUE: u8 = 9;
pub const LIGHT_GREEN: u8 = 10;
#[allow(dead_code)]
pub const LIGHT_CYAN: u8 = 11;
pub const LIGHT_MAGENTA: u8 = 13;
pub const YELLOW: u8 = 14;
pub const WHITE: u8 = 15;

/// A Far foreground color as the terminal gets it: the theme's console color,
/// light gray (the default attribute's) the theme's default foreground.
pub fn fg(index: u8) -> Color {
    crate::theme::with(|t| if index == LIGHT_GRAY { t.default_fg } else { t.console[(index & 15) as usize] })
}

/// A Far background color as the terminal gets it (black: the default background).
pub fn bg(index: u8) -> Color {
    crate::theme::with(|t| if index == BLACK { t.default_bg } else { t.console[(index & 15) as usize] })
}

/// The Far color index of a terminal color (`Reset`: the default attribute).
fn index_of(color: Color, foreground: bool) -> Option<u8> {
    crate::theme::with(|t| {
        let default = if foreground { (t.default_fg, LIGHT_GRAY) } else { (t.default_bg, BLACK) };
        if color == default.0 {
            return Some(default.1);
        }
        match color {
            Color::Reset => Some(if foreground { LIGHT_GRAY } else { BLACK }),
            _ => t.console.iter().position(|&c| c == color).map(|i| i as u8),
        }
    })
}

/// Far's shadow (scrbuf.cpp `apply_nt_index_shadow`): a bright color loses
/// its brightness, light gray becomes dark gray, anything else black.
pub fn shadow(color: Color, foreground: bool) -> Color {
    let paint = |i: u8| if foreground { fg(i) } else { bg(i) };
    match index_of(color, foreground) {
        Some(i) if i & 8 != 0 => paint(i & 7),
        Some(LIGHT_GRAY) => paint(DARK_GRAY),
        Some(_) => paint(BLACK),
        // a color the shell drew itself: half as bright
        None => match color {
            Color::Rgb { r, g, b } => Color::Rgb { r: r / 2, g: g / 2, b: b / 2 },
            _ => paint(BLACK),
        },
    }
}

/// Far's `DropShadow` for a box at (`x`, `y`) of `w` x `h`: two columns on
/// the right and a row below, shifted by two and one, darkened.
pub fn drop_shadow(grid: &mut Grid, x: u16, y: u16, w: u16, h: u16) {
    let (right, bottom) = (x + w.max(1) - 1, y + h.max(1) - 1);
    let mut darken = |col: u16, row: u16| {
        if let Some(cell) = grid.cell_mut(col, row) {
            cell.fg = shadow(cell.fg, true);
            cell.bg = shadow(cell.bg, false);
        }
    };
    for col in x + 2..=right + 2 {
        darken(col, bottom + 1);
    }
    for row in y + 1..=bottom {
        darken(right + 1, row);
        darken(right + 2, row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_shadows_follow_far() {
        assert_eq!(fg(LIGHT_GRAY), Color::Reset);
        assert_eq!(bg(BLACK), Color::Reset);
        assert_eq!(bg(BLUE), Color::Rgb { r: 0, g: 0, b: 128 });
        assert_eq!(shadow(fg(WHITE), true), Color::Reset);
        assert_eq!(shadow(fg(LIGHT_CYAN), true), fg(CYAN));
        assert_eq!(shadow(fg(CYAN), true), fg(BLACK));
        assert_eq!(shadow(Color::Reset, true), fg(DARK_GRAY));
        assert_eq!(shadow(bg(CYAN), false), Color::Reset);
    }
}
