//! Far's quick search: Alt and a letter over a file panel opens a small "Search"
//! box (fastfind.cpp of Far 3.0.6446) and what is typed after it moves the
//! cursor to the first name that begins with it (`FileList::FindPartName`).

use crate::grid::Grid;

/// What fits in the edit line of the box.
const EDIT_WIDTH: u16 = 18;
/// The box: 22 columns, 3 rows.
pub const WIDTH: u16 = 22;
pub const HEIGHT: u16 = 3;

/// What the typed text matches names with: the text and a `*` (so a prefix), `*`
/// and `?` as wildcards, brackets as themselves (Far's `exclude_sets`); a closing
/// separator means folders only.
pub fn mask(typed: &str) -> (String, bool) {
    let (name, folders_only) = match typed.strip_suffix(['\\', '/']) {
        Some(name) => (name, true),
        None => (typed, false),
    };
    let mut mask = String::new();
    for c in name.chars() {
        match c {
            '[' => mask.push_str("[[]"),
            ']' => mask.push_str("[]]"),
            _ => mask.push(c),
        }
    }
    mask.push('*');
    (mask, folders_only)
}

/// The typed text after one more character, as Far's edit does with it: `**`
/// is `*`, a leading quote is dropped.
pub fn with_char(typed: &str, c: char) -> String {
    let mut text = format!("{typed}{c}");
    if text.chars().count() > 1 && text.ends_with("**") {
        text.pop();
    }
    if let Some(rest) = text.strip_prefix('"') {
        text = rest.to_string();
    }
    text
}

/// Where the box goes for a panel whose left column is `left` and whose last row
/// is `bottom`: from the ninth column of the panel, over its bottom border, the
/// command line and the key bar.
pub fn place(left: u16, bottom: u16, cols: u16, rows: u16) -> (u16, u16) {
    let x = (left + 9).min(cols.saturating_sub(WIDTH + 1));
    let y = bottom.min(rows.saturating_sub(3));
    (x, y)
}

/// Draws the box with `text` in it at (`x`, `y`); returns where the cursor goes.
pub fn paint(grid: &mut Grid, x: u16, y: u16, text: &str) -> (u16, u16) {
    use crate::colors::{bg, fg};
    let (box_fg, box_bg) = (fg("Dialog.Box"), bg("Dialog.Box"));
    let (text_fg, text_bg) = (fg("Dialog.Text"), bg("Dialog.Text"));
    let (edit_fg, edit_bg) = (fg("Dialog.Edit"), bg("Dialog.Edit"));
    let line = "\u{2550}".repeat(WIDTH as usize - 2);
    grid.put(x, y, &format!("\u{2554}{line}\u{2557}"), box_fg, box_bg);
    grid.put(x, y + 2, &format!("\u{255A}{line}\u{255D}"), box_fg, box_bg);
    let title = format!(" {} ", crate::lang::tr("Search"));
    grid.put(x + 7, y, &title, fg("Dialog.Box.Title"), bg("Dialog.Box.Title"));
    grid.put(x, y + 1, "\u{2551}", box_fg, box_bg);
    grid.put(x + WIDTH - 1, y + 1, "\u{2551}", box_fg, box_bg);
    grid.put(x + 1, y + 1, " ", text_fg, text_bg);
    grid.put(x + WIDTH - 2, y + 1, " ", text_fg, text_bg);
    grid.fill_with(x + 2, y + 1, EDIT_WIDTH, 1, edit_fg, edit_bg);
    // an edit line shows the end of what does not fit
    let chars: Vec<char> = text.chars().collect();
    let shown: String = chars[chars.len().saturating_sub(EDIT_WIDTH as usize)..].iter().collect();
    grid.put(x + 2, y + 1, &shown, edit_fg, edit_bg);
    (x + 2 + (shown.chars().count() as u16).min(EDIT_WIDTH - 1), y + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masks::name_matches;

    fn hits(typed: &str, name: &str) -> bool {
        name_matches(&mask(typed).0, name)
    }

    #[test]
    fn what_is_typed_is_the_beginning_of_a_name_in_any_case() {
        assert!(hits("rea", "README.md"));
        assert!(hits("README.MD", "readme.md"));
        assert!(!hits("eadme", "README.md"));
        assert!(hits("", "anything"), "nothing typed matches the name the cursor is on");
    }

    #[test]
    fn wildcards_work_and_brackets_are_plain() {
        assert!(hits("r*d", "readme"));
        assert!(hits("r?ad", "readme"));
        assert!(!hits("r?d", "readme"));
        assert!(hits("[a", "[abc]"));
        assert!(!hits("[a", "bc"));
        assert!(hits("a]", "a].txt"));
    }

    #[test]
    fn a_closing_separator_asks_for_folders() {
        assert_eq!(mask("src\\"), ("src*".to_string(), true));
        assert_eq!(mask("src/"), ("src*".to_string(), true));
        assert_eq!(mask("src"), ("src*".to_string(), false));
    }

    #[test]
    fn a_double_star_is_one_and_a_leading_quote_goes() {
        assert_eq!(with_char("a*", '*'), "a*");
        assert_eq!(with_char("", '"'), "");
        assert_eq!(with_char("ab", 'c'), "abc");
    }

    #[test]
    fn the_box_sits_over_the_panels_bottom_and_the_cursor_ends_the_text() {
        assert_eq!(place(0, 20, 120, 30), (9, 20));
        assert_eq!(place(60, 20, 80, 30), (57, 20), "kept inside the screen");
        assert_eq!(place(0, 29, 120, 30), (9, 27), "kept inside the screen");
        let mut grid = Grid::new(40, 6);
        let cursor = paint(&mut grid, 3, 1, "abc");
        assert_eq!(cursor, (3 + 2 + 3, 2));
        assert_eq!(grid.cell_mut(3, 1).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(3 + WIDTH - 1, 3).unwrap().text, "\u{255D}");
        assert_eq!(grid.cell_mut(5, 2).unwrap().text, "a");
        let long = paint(&mut grid, 3, 1, "abcdefghijklmnopqrstuvwxyz");
        assert_eq!(long.0, 3 + 2 + EDIT_WIDTH - 1);
        assert_eq!(grid.cell_mut(5, 2).unwrap().text, "i", "the end of a long text is shown");
    }
}
