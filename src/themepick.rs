//! Options > Themes: a dialog with the themes on the left and, on the right,
//! an example of farsh in the theme under the cursor - a file panel and the
//! editor (fresh, read-only) with "hello, world" in C, the key bar under them.
//! The example is painted with the theme previewed (`theme::previewing`), not
//! made the theme in use; Enter makes it that, Esc leaves the one there was.

use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::colors::{bg, fg};
use crate::fresh_view::EditorView;
use crate::grid::Grid;
use crate::lang::tr;
use crate::palette;
use crate::theme::{self, Theme};

/// The example's C file.
const HELLO: &str = "#include <stdio.h>\n\n/* The first program of K&R */\nint main(void)\n{\n    printf(\"hello, world\\n\");\n    return 0;\n}\n";

/// The list's width (its frame's inside).
const LIST_W: u16 = 24;
const BUTTONS: [&str; 2] = ["OK", "Cancel"];

pub enum Outcome {
    Open,
    Closed,
    /// The theme of this id was chosen.
    Chosen(String),
}

struct Row {
    id: String,
    user: bool,
    theme: Result<Arc<Theme>, String>,
}

pub struct Picker {
    rows: Vec<Row>,
    selected: usize,
    top: usize,
    /// The editor of the example, the theme (row) and size it was opened for.
    editor: Option<(EditorView, usize, (u16, u16))>,
    /// Where the dialog was drawn (x, y, w, h), and its list's first row.
    frame: (u16, u16, u16, u16),
}

fn example_file() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("farsh-theme-example");
    let file = dir.join("hello.c");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(HELLO) {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&file, HELLO).ok()?;
    }
    Some(file)
}

/// `text` cut or padded to `width` columns.
fn fit(text: &str, width: usize) -> String {
    let mut out: String = text.chars().take(width).collect();
    let n = out.chars().count();
    out.extend(std::iter::repeat_n(' ', width - n));
    out
}

impl Picker {
    pub fn new() -> Self {
        let chosen = theme::chosen();
        let rows: Vec<Row> = theme::load_all().into_iter().map(|(id, user, theme)| Row { id, user, theme }).collect();
        let selected = rows.iter().position(|r| r.id == chosen).unwrap_or(0);
        Self { rows, selected, top: 0, editor: None, frame: (0, 0, 0, 0) }
    }

    fn name(row: &Row) -> String {
        match &row.theme {
            Ok(theme) => theme.name.clone(),
            Err(_) => format!("{} ({})", row.id, tr("cannot be read")),
        }
    }

    /// The rows of the list: themes, and one empty row between the built-in and the user's.
    fn lines(&self) -> Vec<Option<usize>> {
        let mut out = Vec::new();
        for (i, row) in self.rows.iter().enumerate() {
            if row.user && i > 0 && !self.rows[i - 1].user {
                out.push(None);
            }
            out.push(Some(i));
        }
        out
    }

    fn select(&mut self, index: usize) {
        self.selected = index.min(self.rows.len().saturating_sub(1));
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let last = self.rows.len().saturating_sub(1);
        let page = self.frame.3.saturating_sub(6).max(1) as usize;
        match key.code {
            KeyCode::Esc | KeyCode::F(10) => return Outcome::Closed,
            KeyCode::Enter => return self.choose(),
            KeyCode::Up => self.select(if self.selected == 0 { last } else { self.selected - 1 }),
            KeyCode::Down => self.select(if self.selected == last { 0 } else { self.selected + 1 }),
            KeyCode::Home | KeyCode::PageUp if key.code == KeyCode::Home || self.selected < page => self.select(0),
            KeyCode::PageUp => self.select(self.selected - page),
            KeyCode::End | KeyCode::PageDown => self.select(if key.code == KeyCode::End { last } else { self.selected + page }),
            KeyCode::Char(c) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
                // the next theme whose name starts with the letter
                let c = c.to_lowercase().next().unwrap_or(c);
                let n = self.rows.len();
                if let Some(i) = (1..=n).map(|d| (self.selected + d) % n).find(|&i| Self::name(&self.rows[i]).to_lowercase().starts_with(c)) {
                    self.select(i);
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    fn choose(&self) -> Outcome {
        match self.rows.get(self.selected) {
            Some(row) => Outcome::Chosen(row.id.clone()),
            None => Outcome::Closed,
        }
    }

    pub fn click(&mut self, event: MouseEvent) -> Outcome {
        let (x0, y0, w, h) = self.frame;
        let inside = event.column >= x0 && event.column < x0 + w && event.row >= y0 && event.row < y0 + h;
        match event.kind {
            MouseEventKind::ScrollUp if inside => return self.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            MouseEventKind::ScrollDown if inside => return self.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            MouseEventKind::Down(MouseButton::Left) => {}
            _ => return Outcome::Open,
        }
        if !inside {
            return Outcome::Closed;
        }
        let (x, y) = (event.column - x0, event.row - y0);
        // the list: from (3, 2), LIST_W wide, down to the separator above the buttons
        if (3..3 + LIST_W).contains(&x) && y >= 2 && y < h - 4 {
            let lines = self.lines();
            if let Some(Some(i)) = lines.get(self.top + (y - 2) as usize) {
                self.select(*i);
            }
            return Outcome::Open;
        }
        if y == h - 3 {
            for (i, (bx, bw)) in self.button_spans(w).into_iter().enumerate() {
                if x >= bx && x < bx + bw {
                    return if i == 0 { self.choose() } else { Outcome::Closed };
                }
            }
        }
        Outcome::Open
    }

    fn button_spans(&self, w: u16) -> Vec<(u16, u16)> {
        let widths: Vec<u16> = BUTTONS.iter().map(|b| tr(b).chars().count() as u16 + 4).collect();
        let total = widths.iter().sum::<u16>() + widths.len() as u16 - 1;
        let mut x = w.saturating_sub(total) / 2;
        widths
            .into_iter()
            .map(|bw| {
                let span = (x, bw);
                x += bw + 1;
                span
            })
            .collect()
    }

    pub fn paint(&mut self, grid: &mut Grid) {
        let (fgc, bgc) = (fg("Dialog.Text"), bg("Dialog.Text"));
        let (box_fg, title_fg) = (fg("Dialog.Box"), fg("Dialog.Box.Title"));
        let w = grid.cols.saturating_sub(4).min(110).max(60);
        let h = grid.rows.saturating_sub(2).min(26).max(16);
        let x0 = grid.cols.saturating_sub(w) / 2;
        let y0 = grid.rows.saturating_sub(h) / 2;
        self.frame = (x0, y0, w, h);
        grid.fill_with(x0, y0, w, h, fgc, bgc);
        palette::drop_shadow(grid, x0, y0, w, h);
        let put = |grid: &mut Grid, x: u16, y: u16, text: &str, f: Color, b: Color| grid.put(x0 + x, y0 + y, text, f, b);
        // the frame (2, 1) .. (w - 3, h - 2), the list's column and the line over the buttons
        let inner = w - 6;
        let split = 3 + LIST_W;
        put(grid, 2, 1, &format!("\u{2554}{}\u{2557}", "\u{2550}".repeat(inner as usize)), box_fg, bgc);
        put(grid, 2, h - 2, &format!("\u{255A}{}\u{255D}", "\u{2550}".repeat(inner as usize)), box_fg, bgc);
        for y in 2..h - 2 {
            put(grid, 2, y, "\u{2551}", box_fg, bgc);
            put(grid, w - 3, y, "\u{2551}", box_fg, bgc);
        }
        put(grid, 2, h - 4, &format!("\u{255F}{}\u{2562}", "\u{2500}".repeat(inner as usize)), box_fg, bgc);
        put(grid, split, 1, "\u{2564}", box_fg, bgc);
        for y in 2..h - 4 {
            put(grid, split, y, "\u{2502}", box_fg, bgc);
        }
        put(grid, split, h - 4, "\u{2534}", box_fg, bgc);
        let title = format!(" {} ", tr("Themes"));
        put(grid, (w - title.chars().count() as u16) / 2, 1, &title, title_fg, bgc);

        // the list
        let lines = self.lines();
        let shown = (h - 6) as usize;
        if let Some(at) = lines.iter().position(|l| *l == Some(self.selected)) {
            if at < self.top {
                self.top = at;
            } else if at >= self.top + shown {
                self.top = at + 1 - shown;
            }
        }
        let chosen = theme::chosen();
        for (row, line) in lines.iter().skip(self.top).take(shown).enumerate() {
            let y = 2 + row as u16;
            let Some(i) = *line else {
                put(grid, 3, y, &"\u{2500}".repeat(LIST_W as usize), box_fg, bgc);
                continue;
            };
            let entry = &self.rows[i];
            let mark = if entry.id == chosen { '\u{221A}' } else { ' ' };
            let text = fit(&format!("{mark}{}", Self::name(entry)), LIST_W as usize);
            let (f, b) = if i == self.selected {
                (fg("Dialog.List.Text.Selected"), bg("Dialog.List.Text.Selected"))
            } else if entry.theme.is_err() {
                (fg("Dialog.Disabled"), bgc)
            } else {
                (fg("Dialog.List.Text"), bg("Dialog.List.Text"))
            };
            put(grid, 3, y, &text, f, b);
        }

        // the buttons
        for (i, (label, (bx, _))) in BUTTONS.iter().zip(self.button_spans(w)).enumerate() {
            let (open, close) = if i == 0 { ('{', '}') } else { ('[', ']') };
            put(grid, bx, h - 3, &format!("{open} {} {close}", tr(label)), fgc, bgc);
        }

        // the example, in the theme under the cursor
        let (ex, ey, ew, eh) = (x0 + split + 1, y0 + 2, w - 3 - split - 1, h - 6);
        match self.rows.get(self.selected).map(|r| r.theme.clone()) {
            Some(Ok(theme)) => {
                let editor = &mut self.editor;
                let row = self.selected;
                theme::previewing(&theme, || example(grid, ex, ey, ew, eh, editor, row));
            }
            Some(Err(error)) => {
                let file = theme::user_dir().join(format!("{}.json", self.rows[self.selected].id));
                for (n, line) in [file.display().to_string(), error].iter().enumerate() {
                    for (k, chunk) in line.chars().collect::<Vec<_>>().chunks(ew as usize).enumerate() {
                        let y = ey + 1 + (n * 2 + k) as u16;
                        if y < ey + eh {
                            grid.put(ex, y, &chunk.iter().collect::<String>(), fg("Dialog.Text"), bgc);
                        }
                    }
                }
            }
            None => {}
        }
    }
}

/// farsh in miniature at (`x`, `y`), `w` by `h`: a file panel, the editor beside it, the key bar.
fn example(grid: &mut Grid, x: u16, y: u16, w: u16, h: u16, editor: &mut Option<(EditorView, usize, (u16, u16))>, row: usize) {
    let panel_w = (w * 2 / 5).max(20).min(w);
    file_panel(grid, x, y, panel_w, h - 1);
    if w > panel_w + 4 {
        editor_part(grid, x + panel_w, y, w - panel_w, h - 1, editor, row);
    }
    key_bar(grid, x, y + h - 1, w);
}

fn file_panel(grid: &mut Grid, x: u16, y: u16, w: u16, h: u16) {
    let (text_fg, text_bg) = (fg("Panel.Text"), bg("Panel.Text"));
    let (box_fg, box_bg) = (fg("Panel.Box"), bg("Panel.Box"));
    grid.fill_with(x, y, w, h, text_fg, text_bg);
    let inner = (w - 2) as usize;
    let size_w = 6usize;
    let name_w = inner.saturating_sub(size_w + 1);
    // the frame, the column divider, the line over the status
    grid.put(x, y, &format!("\u{2554}{}\u{2557}", "\u{2550}".repeat(inner)), box_fg, box_bg);
    grid.put(x, y + h - 1, &format!("\u{255A}{}\u{255D}", "\u{2550}".repeat(inner)), box_fg, box_bg);
    for r in 1..h - 1 {
        grid.put(x, y + r, "\u{2551}", box_fg, box_bg);
        grid.put(x + w - 1, y + r, "\u{2551}", box_fg, box_bg);
        if r < h - 3 {
            grid.put(x + 1 + name_w as u16, y + r, "\u{2502}", box_fg, box_bg);
        }
    }
    grid.put(x, y + h - 3, &format!("\u{255F}{}\u{2562}", "\u{2500}".repeat(inner)), box_fg, box_bg);
    grid.put(x + 1 + name_w as u16, y, "\u{2564}", box_fg, box_bg);
    grid.put(x + 1 + name_w as u16, y + h - 3, "\u{2534}", box_fg, box_bg);
    // the active panel's folder, the column titles
    let title = fit(" C:\\src\\hello ", inner.min(14));
    grid.put(x + (w - title.chars().count() as u16) / 2, y, &title, fg("Panel.Title.Selected"), bg("Panel.Title.Selected"));
    let column = |grid: &mut Grid, text: &str, at: u16, width: usize| {
        let pad = width.saturating_sub(text.chars().count()) / 2;
        grid.put(x + 1 + at + pad as u16, y + 1, text, fg("Panel.Title.Column"), bg("Panel.Title.Column"));
    };
    column(grid, tr("Name"), 0, name_w);
    column(grid, tr("Size"), name_w as u16 + 1, size_w);
    // the files: Far's default highlighting (folders white, hidden cyan, executables
    // light green, archives light magenta, temporary files brown), one marked, the cursor
    let files: [(&str, &str, Option<u8>, u8); 10] = [
        ("..", "Up", None, 0),
        ("src", "Folder", Some(palette::WHITE), 0),
        ("docs", "Folder", Some(palette::WHITE), 0),
        (".git", "Folder", Some(palette::CYAN), 0),
        ("build.cmd", "312", Some(palette::LIGHT_GREEN), 0),
        ("hello.c", "118", None, 2),
        ("hello.h", "64", None, 1),
        ("Makefile", "201", None, 0),
        ("release.zip", "4 096", Some(palette::LIGHT_MAGENTA), 0),
        ("notes.tmp", "17", Some(palette::BROWN), 0),
    ];
    for (i, (name, size, color, state)) in files.iter().enumerate() {
        let r = 2 + i as u16;
        if r >= h - 3 {
            break;
        }
        let base = ["Panel.Text", "Panel.Text.Selected", "Panel.Cursor"][*state as usize];
        let (mut f, b) = (fg(base), bg(base));
        if let Some(c) = color {
            f = palette::fg(*c);
        }
        grid.put(x + 1, y + r, &fit(name, name_w), f, b);
        let size = format!("{size:>size_w$}");
        grid.put(x + 2 + name_w as u16, y + r, &fit(&size, size_w), if *state == 2 { f } else { fg(base) }, if *state == 2 { b } else { bg("Panel.Text") });
        if *state == 2 {
            grid.put(x + 1 + name_w as u16, y + r, "\u{2502}", fg("Panel.Cursor"), b);
        }
    }
    // the status line and the totals
    grid.put(x + 1, y + h - 2, &fit("hello.c", inner.saturating_sub(4)), text_fg, text_bg);
    grid.put(x + w - 4, y + h - 2, "118", text_fg, text_bg);
    let total = format!(" {} ", tr("1 file selected"));
    if total.chars().count() + 2 < w as usize {
        grid.put(x + (w - total.chars().count() as u16) / 2, y + h - 3, &total, fg("Panel.Info.Selected"), bg("Panel.Info.Selected"));
    }
    let total = " 4 808 bytes in 7 files ";
    if total.chars().count() + 2 < w as usize {
        grid.put(x + (w - total.chars().count() as u16) / 2, y + h - 1, total, fg("Panel.Info.Total"), bg("Panel.Info.Total"));
    }
}

fn editor_part(grid: &mut Grid, x: u16, y: u16, w: u16, h: u16, editor: &mut Option<(EditorView, usize, (u16, u16))>, row: usize) {
    // Far's editor status bar on top
    let status = fit(" hello.c          Line 1   Col 1   UTF-8", w as usize);
    grid.put(x, y, &status, fg("Editor.Status"), bg("Editor.Status"));
    let (ew, eh) = (w, h.saturating_sub(1).max(1));
    grid.fill_with(x, y + 1, ew, eh, fg("Editor.Text"), bg("Editor.Text"));
    let stale = !matches!(editor, Some((_, r, size)) if *r == row && *size == (ew, eh));
    if stale {
        *editor = example_file().and_then(|file| {
            let mut options = crate::fresh_view::editor_options();
            options.read_only = true;
            EditorView::open(&file, ew, eh, options).ok().map(|view| (view, row, (ew, eh)))
        });
    }
    if let Some((view, _, _)) = editor {
        let _ = view.paint(grid, x, y + 1);
    }
}

fn key_bar(grid: &mut Grid, x: u16, y: u16, w: u16) {
    let keys = ["Help", "UserMn", "View", "Edit", "Copy", "RenMov", "MkFold", "Delete", "ConfMn", "Quit"];
    grid.fill_with(x, y, w, 1, fg("Keybar.Background"), bg("Keybar.Background"));
    // even widths over the whole row, the leftover columns one each to the first keys (as farsh's key bar)
    let n = keys.len() as u16;
    let (base, extra) = (w / n, w % n);
    let mut at = x;
    for (i, label) in keys.iter().enumerate() {
        let each = base + u16::from((i as u16) < extra);
        let num = format!("{}", i + 1);
        grid.put(at, y, &num, fg("Keybar.Num"), bg("Keybar.Num"));
        let text = fit(label, (each as usize).saturating_sub(num.len()));
        grid.put(at + num.len() as u16, y, &text, fg("Keybar.Text"), bg("Keybar.Text"));
        at += each;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn the_example_is_in_the_theme_under_the_cursor_and_enter_chooses_it() {
        let mut picker = Picker::new();
        assert_eq!(picker.rows[picker.selected].id, "default", "the theme in use is under the cursor");
        let mut grid = Grid::new(120, 30);
        picker.paint(&mut grid);
        let (x0, y0, _, _) = picker.frame;
        // the example's file panel: the cell right of its frame's corner, on Far's blue
        let panel = (x0 + 3 + LIST_W + 2, y0 + 3);
        assert_eq!(grid.cell_mut(panel.0, panel.1).unwrap().bg, Color::Rgb { r: 0, g: 0, b: 128 });
        // Nord: its panel background, while the theme in use stays Far's
        let nord = picker.rows.iter().position(|r| r.id == "nord").unwrap();
        while picker.selected != nord {
            picker.key(key(KeyCode::Down));
        }
        picker.paint(&mut grid);
        assert_eq!(grid.cell_mut(panel.0, panel.1).unwrap().bg, Color::Rgb { r: 0x2e, g: 0x34, b: 0x40 });
        assert_eq!(crate::colors::bg("Panel.Text"), Color::Rgb { r: 0, g: 0, b: 128 }, "the theme in use did not change");
        // the editor shows the C example
        let text: String = (0..grid.rows).map(|y| (0..grid.cols).map(|x| grid.cell_mut(x, y).unwrap().text.clone()).collect::<String>() + "\n").collect();
        assert!(text.contains("printf(\"hello, world\\n\");"), "{text}");
        assert!(text.contains("hello.c"));
        assert!(matches!(picker.key(key(KeyCode::Enter)), Outcome::Chosen(id) if id == "nord"));
        assert!(matches!(picker.key(key(KeyCode::Esc)), Outcome::Closed));
    }
}

