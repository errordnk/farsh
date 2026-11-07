//! fresh, our fork of the editor (GPL-3.0, as farsh is), drawn into a
//! rectangle of farsh's screen: the internal editor (F4), the viewer's text
//! (F3) and the quick view's (Ctrl+Q) - see docs/fresh-plan.md.
//!
//! fresh draws with ratatui; [`GridBackend`] is a ratatui backend that keeps
//! the frame as farsh [`Cell`]s instead of writing it to a terminal, and
//! [`EditorView`] puts the session, that backend and the host's side of the
//! loop together: events in (offset to the rectangle), frames out into the
//! [`Grid`].

use std::io;
use std::path::Path;
use std::time::Instant;

use crossterm::event::{Event, MouseEvent};
use crossterm::style::Color;
use fresh::embed::{EmbedOptions, Session};
use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell as RatCell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color as RatColor, Modifier};
use ratatui::Terminal;

use crate::grid::{text_width, Cell, Grid};
use crate::{colors, palette};

/// A ratatui color as the terminal color farsh's cells carry.
fn color(color: RatColor) -> Color {
    match color {
        RatColor::Reset => Color::Reset,
        RatColor::Black => Color::Black,
        RatColor::Red => Color::DarkRed,
        RatColor::Green => Color::DarkGreen,
        RatColor::Yellow => Color::DarkYellow,
        RatColor::Blue => Color::DarkBlue,
        RatColor::Magenta => Color::DarkMagenta,
        RatColor::Cyan => Color::DarkCyan,
        RatColor::Gray => Color::Grey,
        RatColor::DarkGray => Color::DarkGrey,
        RatColor::LightRed => Color::Red,
        RatColor::LightGreen => Color::Green,
        RatColor::LightYellow => Color::Yellow,
        RatColor::LightBlue => Color::Blue,
        RatColor::LightMagenta => Color::Magenta,
        RatColor::LightCyan => Color::Cyan,
        RatColor::White => Color::White,
        RatColor::Rgb(r, g, b) => Color::Rgb { r, g, b },
        RatColor::Indexed(n) => Color::AnsiValue(n),
    }
}

/// A ratatui cell as a farsh one.
fn cell(source: &RatCell) -> Cell {
    let m = source.modifier;
    Cell {
        text: if source.symbol().is_empty() { " ".into() } else { source.symbol().to_string() },
        fg: color(source.fg),
        bg: color(source.bg),
        bold: m.contains(Modifier::BOLD),
        underline: m.contains(Modifier::UNDERLINED),
        reverse: m.contains(Modifier::REVERSED),
        italic: m.contains(Modifier::ITALIC),
        dim: m.contains(Modifier::DIM),
        strike: m.contains(Modifier::CROSSED_OUT),
        ul: match source.underline_color {
            RatColor::Reset => None,
            other => Some(color(other)),
        },
    }
}

/// A ratatui backend whose "terminal" is a block of farsh cells and a cursor.
pub struct GridBackend {
    cols: u16,
    rows: u16,
    cells: Vec<Cell>,
    cursor: Position,
    cursor_shown: bool,
}

impl GridBackend {
    fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows, cells: vec![Cell::default(); usize::from(cols) * usize::from(rows)], cursor: Position::ORIGIN, cursor_shown: false }
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.cols && y < self.rows).then(|| usize::from(y) * usize::from(self.cols) + usize::from(x))
    }
}

impl Backend for GridBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a RatCell)>,
    {
        for (x, y, source) in content {
            let Some(at) = self.index(x, y) else { continue };
            let new = cell(source);
            let wide = text_width(&new.text) == 2;
            self.cells[at] = new;
            // the cell after a wide character is its other half: no text of its own
            if wide && let Some(next) = self.index(x + 1, y) {
                self.cells[next] = Cell { text: String::new(), ..Cell::default() };
            }
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.cursor_shown = false;
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.cursor_shown = true;
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.cursor = position.into();
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.cells.fill(Cell::default());
        Ok(())
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        let cursor = self.index(self.cursor.x, self.cursor.y).unwrap_or(0);
        let line = usize::from(self.cursor.y) * usize::from(self.cols);
        let (from, to) = match clear_type {
            ClearType::All => (0, self.cells.len()),
            ClearType::AfterCursor => (cursor + 1, self.cells.len()),
            ClearType::BeforeCursor => (0, cursor),
            ClearType::CurrentLine => (line, line + usize::from(self.cols)),
            ClearType::UntilNewLine => (cursor, line + usize::from(self.cols)),
        };
        let to = to.min(self.cells.len());
        if from < to {
            self.cells[from..to].fill(Cell::default());
        }
        Ok(())
    }

    fn size(&self) -> io::Result<Size> {
        Ok(Size { width: self.cols, height: self.rows })
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        // farsh does not ask the terminal for its pixels: an ordinary cell
        Ok(WindowSize { columns_rows: Size { width: self.cols, height: self.rows }, pixels: Size { width: self.cols * 8, height: self.rows * 16 } })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// How the viewer's text looks (the quick view's, and F3's): Far's viewer has
/// no line numbers, no current-line band and no scroll bar by default (its
/// `Viewer.ShowScrollbar`), and wraps lines in the quick view. No animations: a
/// viewer that follows the cursor of a panel would slide and trail on every file
/// (the editor, F4, keeps them).
fn viewer_config(config: &mut fresh::config::Config) {
    config.editor.animations = false;
    config.editor.line_numbers = false;
    config.editor.line_wrap = true;
    config.editor.show_vertical_scrollbar = false;
    config.editor.highlight_current_line = false;
    // Viewer settings: Tab size
    config.editor.tab_size = crate::settings::int("Viewer.TabSize").clamp(1, 512) as usize;
}

/// A read-only session for looking at a file.
pub fn viewer_options() -> EmbedOptions {
    viewer_options_in("Panel.Text", "Viewer.Text.Selected")
}

/// The same, in the colors named `text` and `selected` (Options > Colors).
pub fn viewer_options_in(text: &str, selected: &str) -> EmbedOptions {
    EmbedOptions {
        read_only: true,
        configure: Some(viewer_config),
        theme_json: Some(theme_json(text, selected)),
        dirs: dirs(),
        ..EmbedOptions::default()
    }
}

/// What fresh's syntax categories look like on Far's blue: the ONE table of
/// them (console color indices; farsh's own theme format will replace it).
const SYNTAX: [(&str, u8); 9] = [
    ("keyword", palette::WHITE),
    ("string", palette::LIGHT_GREEN),
    ("comment", palette::DARK_GRAY),
    ("function", palette::YELLOW),
    ("type", palette::LIGHT_MAGENTA),
    ("variable", palette::LIGHT_CYAN),
    ("variable_builtin", palette::LIGHT_MAGENTA),
    ("constant", palette::YELLOW),
    ("operator", palette::WHITE),
];

fn json_color(color: Color) -> serde_json::Value {
    match color {
        Color::Rgb { r, g, b } => serde_json::json!([r, g, b]),
        _ => serde_json::json!("Default"),
    }
}

/// A fresh theme (JSON, over its dark one) that paints text and selection in
/// farsh's colors `text` and `selected`: the viewer's panel looks like the file
/// panel next to it, and syntax categories take the table above.
pub fn theme_json(text: &str, selected: &str) -> String {
    let (fg, bg) = (json_color(colors::fg(text)), json_color(colors::bg(text)));
    let (sel_fg, sel_bg) = (json_color(colors::fg(selected)), json_color(colors::bg(selected)));
    let syntax: serde_json::Map<String, serde_json::Value> =
        SYNTAX.iter().map(|(name, index)| ((*name).to_string(), json_color(palette::fg(*index)))).collect();
    serde_json::json!({
        "name": "farsh",
        "extends": "builtin://dark",
        "editor": {
            "bg": bg, "after_eof_bg": bg, "fg": fg, "current_line_bg": bg,
            "selection_bg": sel_bg, "selection_fg": sel_fg,
            "line_number_bg": bg, "line_number_fg": fg,
        },
        "syntax": syntax,
    })
    .to_string()
}

/// Where the embedded editor keeps its config and data: the user's fresh
/// folders (`~/.config/fresh`, ...), and in tests a temporary tree of the
/// thread's own, so a test never reads or writes the user's.
#[cfg(not(test))]
fn dirs() -> Option<fresh::config_io::DirectoryContext> {
    None
}

#[cfg(test)]
fn dirs() -> Option<fresh::config_io::DirectoryContext> {
    thread_local! {
        static TREE: tempfile::TempDir = tempfile::tempdir().unwrap();
    }
    TREE.with(|tree| Some(fresh::config_io::DirectoryContext::for_testing(tree.path())))
}

/// One fresh session in a rectangle of the screen.
pub struct EditorView {
    session: Session,
    terminal: Terminal<GridBackend>,
}

impl EditorView {
    /// Opens `path` for a rectangle of `cols` x `rows`.
    pub fn open(path: &Path, cols: u16, rows: u16, options: EmbedOptions) -> anyhow::Result<Self> {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let session = Session::open(path, cols, rows, options)?;
        Ok(Self { session, terminal: Terminal::new(GridBackend::new(cols, rows))? })
    }

    pub fn session(&mut self) -> &mut Session {
        &mut self.session
    }

    /// The rectangle's size changed.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let size = self.terminal.backend().size().unwrap_or_default();
        if (size.width, size.height) == (cols, rows) {
            return;
        }
        self.session.resize(cols, rows);
        // a new "terminal" of the new size: the next frame is drawn whole
        if let Ok(terminal) = Terminal::new(GridBackend::new(cols, rows)) {
            self.terminal = terminal;
        }
    }

    /// An event of the host's terminal; mouse positions are the screen's, and
    /// `(x, y)` is where this view's rectangle starts. `true` when the view
    /// changed. Resizes and focus events are the host's, not passed on.
    pub fn event(&mut self, event: Event, x: u16, y: u16) -> anyhow::Result<bool> {
        let event = match event {
            Event::Mouse(MouseEvent { kind, column, row, modifiers }) => {
                Event::Mouse(MouseEvent { kind, column: column.saturating_sub(x), row: row.saturating_sub(y), modifiers })
            }
            Event::Key(_) | Event::Paste(_) => event,
            _ => return Ok(false),
        };
        self.session.handle(fresh_input_parser::Event::from(event))
    }

    /// The session's housekeeping; `true` when the next frame differs.
    pub fn tick(&mut self) -> anyhow::Result<bool> {
        self.session.tick()
    }

    /// When the session wants a frame with no input (an animation).
    pub fn next_frame(&mut self) -> Option<Instant> {
        self.session.next_frame()
    }

    /// Draws the session into `grid` from (`x`, `y`), clipped to the screen;
    /// returns the cursor's place on the screen, if the editor shows one.
    pub fn paint(&mut self, grid: &mut Grid, x: u16, y: u16) -> io::Result<Option<(u16, u16)>> {
        // what the editor has to settle first (a file just opened) is settled
        // before the frame, so that the frame is the file's, not the one before
        let _ = self.session.tick();
        self.terminal.draw(|frame| self.session.render(frame))?;
        let backend = self.terminal.backend();
        for row in 0..backend.rows {
            for col in 0..backend.cols {
                if let (Some(source), Some(target)) = (backend.index(col, row).map(|i| &backend.cells[i]), grid.cell_mut(x + col, y + row)) {
                    *target = source.clone();
                }
            }
        }
        Ok(backend.cursor_shown.then_some((x + backend.cursor.x, y + backend.cursor.y)).filter(|&(cx, cy)| cx < grid.cols && cy < grid.rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use fresh::config_io::DirectoryContext;

    fn options(dirs: &Path, read_only: bool) -> EmbedOptions {
        EmbedOptions { read_only, dirs: Some(DirectoryContext::for_testing(dirs)), ..EmbedOptions::default() }
    }

    fn rows(grid: &mut Grid) -> Vec<String> {
        (0..grid.rows).map(|y| (0..grid.cols).map(|x| grid.cell_mut(x, y).unwrap().text.clone()).collect()).collect()
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn the_viewer_text_area_has_the_file_panels_background_down_to_the_last_row() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("text.txt");
        std::fs::write(&file, "one line
").unwrap();
        let mut options = viewer_options();
        options.dirs = Some(DirectoryContext::for_testing(&temp.path().join("dirs")));
        let mut view = EditorView::open(&file, 30, 6, options).unwrap();
        view.tick().unwrap();
        let mut grid = Grid::new(30, 6);
        view.paint(&mut grid, 0, 0).unwrap();
        let panel = colors::bg("Panel.Text");
        assert_eq!(panel, Color::Rgb { r: 0, g: 0, b: 128 }, "Far's blue");
        // the line, the empty rows past the file and the rest of the line: all the panel's
        for (x, y) in [(0, 0), (5, 0), (29, 0), (0, 3), (29, 5)] {
            assert_eq!(grid.cell_mut(x, y).unwrap().bg, panel, "cell ({x}, {y})");
        }
    }

    #[test]
    fn the_editor_is_drawn_into_its_rectangle_of_the_screen() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("text.txt");
        std::fs::write(&file, "hello from fresh\nsecond line\n").unwrap();
        let mut view = EditorView::open(&file, 30, 6, options(&temp.path().join("dirs"), false)).unwrap();
        view.tick().unwrap();
        let mut grid = Grid::new(60, 10);
        let cursor = view.paint(&mut grid, 20, 2).unwrap();
        let screen = rows(&mut grid);
        // the text is inside the rectangle (20.., 2..), nothing outside it is touched
        assert!(screen[2][20..].contains("hello from fresh"), "{screen:#?}");
        assert!(screen[3][20..].contains("second line"));
        assert!(screen[..2].iter().all(|r| r.trim().is_empty()) && screen[8..].iter().all(|r| r.trim().is_empty()));
        assert!(screen.iter().all(|r| r[..20].trim().is_empty()), "left of the rectangle is the host's");
        // the editor has a cursor, inside its rectangle
        let (cx, cy) = cursor.expect("an editor shows a cursor");
        assert!((20..50).contains(&cx) && (2..8).contains(&cy), "{cursor:?}");
        // no status bar of fresh's: the file's name is nowhere
        assert!(screen.iter().all(|r| !r.contains("text.txt")), "{screen:#?}");
    }

    #[test]
    fn typing_goes_in_mouse_clicks_are_taken_in_the_rectangles_coordinates() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("text.txt");
        std::fs::write(&file, "abc\ndef\n").unwrap();
        let mut view = EditorView::open(&file, 30, 6, options(&temp.path().join("dirs"), false)).unwrap();
        view.tick().unwrap();
        let mut grid = Grid::new(60, 10);
        view.paint(&mut grid, 20, 2).unwrap();
        assert!(view.event(key(KeyCode::Char('X')), 20, 2).unwrap());
        assert!(view.session().modified());
        view.paint(&mut grid, 20, 2).unwrap();
        assert!(rows(&mut grid)[2][20..].contains("Xabc"));
        // a click on the second line, by screen coordinates: the cursor goes there
        let click = Event::Mouse(MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 21,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        view.event(click, 20, 2).unwrap();
        view.tick().unwrap();
        let cursor = view.paint(&mut grid, 20, 2).unwrap().unwrap();
        assert_eq!(cursor.1, 3, "the cursor is on the second line: {cursor:?}");
    }

    #[test]
    fn a_resize_makes_a_new_frame_of_the_new_size() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("text.txt");
        std::fs::write(&file, "resize me\n").unwrap();
        let mut view = EditorView::open(&file, 30, 6, options(&temp.path().join("dirs"), true)).unwrap();
        let mut grid = Grid::new(60, 10);
        view.paint(&mut grid, 0, 0).unwrap();
        view.resize(50, 8);
        view.tick().unwrap();
        let mut bigger = Grid::new(60, 10);
        view.paint(&mut bigger, 0, 0).unwrap();
        assert!(rows(&mut bigger)[0].contains("resize me"));
        // the viewer is read-only
        assert!(view.session().read_only());
        view.event(key(KeyCode::Char('Q')), 0, 0).unwrap();
        assert!(!view.session().modified());
    }

    #[test]
    fn colors_and_attributes_become_farsh_cells() {
        assert_eq!(color(RatColor::Rgb(1, 2, 3)), Color::Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(color(RatColor::Red), Color::DarkRed);
        assert_eq!(color(RatColor::LightRed), Color::Red);
        assert_eq!(color(RatColor::Indexed(7)), Color::AnsiValue(7));
        let mut source = RatCell::default();
        source.set_symbol("w").set_fg(RatColor::Rgb(9, 8, 7)).set_bg(RatColor::Black);
        source.modifier = Modifier::BOLD | Modifier::ITALIC | Modifier::CROSSED_OUT;
        let converted = cell(&source);
        assert_eq!((converted.text.as_str(), converted.fg, converted.bg), ("w", Color::Rgb { r: 9, g: 8, b: 7 }, Color::Black));
        assert!(converted.bold && converted.italic && converted.strike && !converted.underline && !converted.dim);
    }
}
