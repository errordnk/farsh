//! The colors Far Manager 3 reaches the terminal with. Far paints with
//! console attributes; through ConPTY they arrive as the classic console
//! table in RGB, except that light gray text (7) and a black background (0)
//! — the console's default attribute — arrive as the terminal's own default
//! colors. Measured in Som: blue is (0,0,128), cyan (0,128,128), light cyan
//! (0,255,255); the key bar numbers take Som's own foreground and background.

use crossterm::style::Color;

const TABLE: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (0, 0, 128),
    (0, 128, 0),
    (0, 128, 128),
    (128, 0, 0),
    (128, 0, 128),
    (128, 128, 0),
    (192, 192, 192),
    (128, 128, 128),
    (0, 0, 255),
    (0, 255, 0),
    (0, 255, 255),
    (255, 0, 0),
    (255, 0, 255),
    (255, 255, 0),
    (255, 255, 255),
];

pub const BLACK: u8 = 0;
pub const BLUE: u8 = 1;
pub const CYAN: u8 = 3;
pub const BROWN: u8 = 6;
pub const LIGHT_GRAY: u8 = 7;
pub const DARK_GRAY: u8 = 8;
pub const LIGHT_GREEN: u8 = 10;
pub const LIGHT_CYAN: u8 = 11;
pub const LIGHT_MAGENTA: u8 = 13;
pub const YELLOW: u8 = 14;
pub const WHITE: u8 = 15;

const fn rgb(index: u8) -> Color {
    let (r, g, b) = TABLE[(index & 15) as usize];
    Color::Rgb { r, g, b }
}

/// A Far foreground color as the terminal gets it.
pub const fn fg(index: u8) -> Color {
    if index == LIGHT_GRAY { Color::Reset } else { rgb(index) }
}

/// A Far background color as the terminal gets it.
pub const fn bg(index: u8) -> Color {
    if index == BLACK { Color::Reset } else { rgb(index) }
}

/// The Far color index of a terminal color (`Reset`: the default attribute).
fn index_of(color: Color, foreground: bool) -> Option<u8> {
    match color {
        Color::Reset => Some(if foreground { LIGHT_GRAY } else { BLACK }),
        Color::Rgb { r, g, b } => TABLE.iter().position(|&c| c == (r, g, b)).map(|i| i as u8),
        _ => None,
    }
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
