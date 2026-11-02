//! Far style dialogs: a grey box with a double border, an input line and buttons.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::Color;

use crate::fileops::{Job, Kind};
use crate::grid::Grid;

const GREY: Color = Color::Rgb { r: 170, g: 170, b: 170 };
const BLACK: Color = Color::Rgb { r: 0, g: 0, b: 0 };
const CYAN: Color = Color::Rgb { r: 0, g: 170, b: 170 };
const SHADOW: Color = Color::Rgb { r: 0, g: 0, b: 0 };

/// What an accepted dialog asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Copy(Vec<PathBuf>),
    Move(Vec<PathBuf>),
    MkDir,
    Delete(Vec<PathBuf>),
    Quit,
}

/// What a click can land on.
enum Hit {
    Ok,
    Cancel,
    /// The input line; the payload is the index of its first visible character.
    Input(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Input,
    Ok,
    Cancel,
}

pub enum Dialog {
    /// A line to edit (a destination, a folder name).
    Prompt { title: String, label: String, input: Vec<char>, cursor: usize, ok: &'static str, action: Action, focus: Focus },
    /// A question with a button for yes and one for no.
    Confirm { title: String, lines: Vec<String>, yes: &'static str, action: Action, yes_focused: bool },
    /// Something to read, closed with Enter.
    Message { title: String, lines: Vec<String> },
}

pub enum Outcome {
    Open,
    Closed,
    /// Accepted: what to do, and the text typed (empty for a confirmation).
    Accepted(Action, String),
}

impl Dialog {
    pub fn prompt(title: &str, label: String, text: &str, ok: &'static str, action: Action) -> Self {
        let input: Vec<char> = text.chars().collect();
        let cursor = input.len();
        Dialog::Prompt { title: title.into(), label, input, cursor, ok, action, focus: Focus::Input }
    }

    pub fn confirm(title: &str, lines: Vec<String>, yes: &'static str, action: Action) -> Self {
        Dialog::Confirm { title: title.into(), lines, yes, action, yes_focused: true }
    }

    pub fn message(title: &str, text: &str) -> Self {
        Dialog::Message { title: title.into(), lines: text.lines().map(String::from).collect() }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if key.code == KeyCode::Esc {
            return Outcome::Closed;
        }
        match self {
            Dialog::Message { .. } => match key.code {
                KeyCode::Enter | KeyCode::Char(' ') => Outcome::Closed,
                _ => Outcome::Open,
            },
            Dialog::Confirm { action, yes_focused, .. } => match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                    *yes_focused = !*yes_focused;
                    Outcome::Open
                }
                KeyCode::Enter if *yes_focused => Outcome::Accepted(action.clone(), String::new()),
                KeyCode::Enter => Outcome::Closed,
                KeyCode::Char('y' | 'Y') => Outcome::Accepted(action.clone(), String::new()),
                KeyCode::Char('n' | 'N') => Outcome::Closed,
                _ => Outcome::Open,
            },
            Dialog::Prompt { input, cursor, action, focus, .. } => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Tab => {
                        *focus = match focus {
                            Focus::Input => Focus::Ok,
                            Focus::Ok => Focus::Cancel,
                            Focus::Cancel => Focus::Input,
                        }
                    }
                    KeyCode::BackTab => {
                        *focus = match focus {
                            Focus::Input => Focus::Cancel,
                            Focus::Ok => Focus::Input,
                            Focus::Cancel => Focus::Ok,
                        }
                    }
                    KeyCode::Enter if *focus == Focus::Cancel => return Outcome::Closed,
                    KeyCode::Enter => return Outcome::Accepted(action.clone(), input.iter().collect()),
                    KeyCode::Left if *focus == Focus::Input => *cursor = cursor.saturating_sub(1),
                    KeyCode::Right if *focus == Focus::Input => *cursor = (*cursor + 1).min(input.len()),
                    KeyCode::Home if *focus == Focus::Input => *cursor = 0,
                    KeyCode::End if *focus == Focus::Input => *cursor = input.len(),
                    KeyCode::Backspace if *focus == Focus::Input && *cursor > 0 => {
                        *cursor -= 1;
                        input.remove(*cursor);
                    }
                    KeyCode::Delete if *focus == Focus::Input && *cursor < input.len() => {
                        input.remove(*cursor);
                    }
                    KeyCode::Char('u') if ctrl && *focus == Focus::Input => {
                        input.clear();
                        *cursor = 0;
                    }
                    KeyCode::Char(c) if !ctrl && *focus == Focus::Input => {
                        input.insert(*cursor, c);
                        *cursor += 1;
                    }
                    _ => {}
                }
                Outcome::Open
            }
        }
    }

    /// What a click at (`col`, `row`) does, on a screen of `size`.
    pub fn click(&mut self, col: u16, row: u16, size: (u16, u16)) -> Outcome {
        for (hit, x, y, w) in self.hits(size) {
            if row != y || col < x || col >= x + w {
                continue;
            }
            return match hit {
                Hit::Cancel => Outcome::Closed,
                Hit::Ok => self.accept(),
                Hit::Input(start) => {
                    if let Dialog::Prompt { input, cursor, focus, .. } = self {
                        *focus = Focus::Input;
                        *cursor = (start + (col - x) as usize).min(input.len());
                    }
                    Outcome::Open
                }
            };
        }
        Outcome::Open
    }

    /// The same as Enter on the default button.
    fn accept(&self) -> Outcome {
        match self {
            Dialog::Message { .. } => Outcome::Closed,
            Dialog::Confirm { action, .. } => Outcome::Accepted(action.clone(), String::new()),
            Dialog::Prompt { action, input, .. } => Outcome::Accepted(action.clone(), input.iter().collect()),
        }
    }

    /// The places that react to a click: (what, x, y, width).
    fn hits(&self, size: (u16, u16)) -> Vec<(Hit, u16, u16, u16)> {
        let (area, labels): (Area, Vec<&str>) = match self {
            Dialog::Message { title, lines } => (Area::centered(size, title, lines.len() as u16 + 3, text_width(lines, 20)), vec!["OK"]),
            Dialog::Confirm { title, lines, yes, .. } => {
                (Area::centered(size, title, lines.len() as u16 + 3, text_width(lines, 30)), vec![*yes, "Cancel"])
            }
            Dialog::Prompt { title, ok, .. } => (Area::centered(size, title, 6, (size.0 * 3 / 5).clamp(30, 76)), vec![*ok, "Cancel"]),
        };
        let mut hits: Vec<(Hit, u16, u16, u16)> = area
            .button_slots(&labels)
            .into_iter()
            .enumerate()
            .map(|(i, (x, y, w))| (if i == 0 { Hit::Ok } else { Hit::Cancel }, x, y, w))
            .collect();
        if let Dialog::Prompt { cursor, .. } = self {
            let width = area.w.saturating_sub(4);
            hits.push((Hit::Input((*cursor + 1).saturating_sub(width as usize)), area.x + 2, area.y + 3, width));
        }
        hits
    }

    /// Draws the dialog centered on `grid`; returns where the text cursor goes.
    pub fn paint(&self, grid: &mut Grid) -> Option<(u16, u16)> {
        match self {
            Dialog::Message { title, lines } => {
                let area = Area::centered((grid.cols, grid.rows), title, lines.len() as u16 + 3, text_width(lines, 20));
                area.draw(grid, title);
                for (i, line) in lines.iter().enumerate() {
                    grid.put(area.x + 2, area.y + 2 + i as u16, line, BLACK, GREY);
                }
                area.buttons(grid, &[("OK", true)]);
                None
            }
            Dialog::Confirm { title, lines, yes, yes_focused, .. } => {
                let area = Area::centered((grid.cols, grid.rows), title, lines.len() as u16 + 3, text_width(lines, 30));
                area.draw(grid, title);
                for (i, line) in lines.iter().enumerate() {
                    let col = area.x + (area.w.saturating_sub(line.chars().count() as u16)) / 2;
                    grid.put(col, area.y + 2 + i as u16, line, BLACK, GREY);
                }
                area.buttons(grid, &[(yes, *yes_focused), ("Cancel", !*yes_focused)]);
                None
            }
            Dialog::Prompt { title, label, input, cursor, ok, focus, .. } => {
                let area = Area::centered((grid.cols, grid.rows), title, 6, (grid.cols * 3 / 5).clamp(30, 76));
                area.draw(grid, title);
                grid.put(area.x + 2, area.y + 2, label, BLACK, GREY);
                let width = area.w.saturating_sub(4) as usize;
                // The input scrolls so that the cursor stays in view.
                let start = (*cursor + 1).saturating_sub(width);
                let shown: String = input.iter().skip(start).take(width).collect();
                grid.fill(area.x + 2, area.y + 3, width as u16, 1, CYAN);
                grid.put(area.x + 2, area.y + 3, &shown, BLACK, CYAN);
                area.buttons(grid, &[(ok, *focus == Focus::Ok), ("Cancel", *focus == Focus::Cancel)]);
                (*focus == Focus::Input).then(|| (area.x + 2 + (*cursor - start) as u16, area.y + 3))
            }
        }
    }
}

/// The progress box of a running copy, move or delete.
pub fn paint_progress(job: &Job, grid: &mut Grid) {
    let progress = &job.progress;
    let title = job.kind.title();
    let area = Area::centered((grid.cols, grid.rows), title, if job.kind == Kind::Delete { 8 } else { 12 }, (grid.cols * 3 / 5).clamp(40, 76));
    area.draw(grid, title);
    let inner = area.w.saturating_sub(4) as usize;
    let fit = |text: &str| -> String {
        let count = text.chars().count();
        if count <= inner {
            text.to_string()
        } else {
            format!("...{}", text.chars().skip(count - inner + 3).collect::<String>())
        }
    };
    let mut row = area.y + 2;
    let verb = match job.kind {
        Kind::Copy => "Copying the file",
        Kind::Move => "Moving the file",
        Kind::Delete => "Deleting",
    };
    grid.put(area.x + 2, row, verb, BLACK, GREY);
    grid.put(area.x + 2, row + 1, &fit(&progress.from), BLACK, GREY);
    row += 2;
    if job.kind != Kind::Delete {
        grid.put(area.x + 2, row, "to", BLACK, GREY);
        grid.put(area.x + 2, row + 1, &fit(&progress.to), BLACK, GREY);
        bar(grid, area.x + 2, row + 2, inner, ratio(progress.file_done, progress.file_size));
        row += 4;
    }
    grid.put(area.x + 2, row, &format!("Files: {} / {}", progress.files_done, progress.files_total), BLACK, GREY);
    bar(grid, area.x + 2, row + 1, inner, ratio(progress.bytes_done, progress.bytes_total));
    let elapsed = job.elapsed().as_secs();
    let speed = progress.bytes_done as f64 / job.elapsed().as_secs_f64().max(0.001);
    let left = if speed > 1.0 { ((progress.bytes_total.saturating_sub(progress.bytes_done)) as f64 / speed) as u64 } else { 0 };
    let line = format!(
        "Time: {}  Remaining: {}  {} MB/s",
        clock(elapsed),
        clock(left),
        (speed / 1_048_576.0) as u64
    );
    grid.put(area.x + 2, row + 2, &line, BLACK, GREY);
    grid.put(area.x + 2, area.y + area.h - 2, "Esc cancels", BLACK, GREY);
}

fn ratio(done: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).min(1.0)
    }
}

fn clock(seconds: u64) -> String {
    format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// A bar `width` cells wide with the percentage at its end.
fn bar(grid: &mut Grid, x: u16, y: u16, width: usize, fraction: f64) {
    let percent = format!("{:>3}%", (fraction * 100.0) as u32);
    let cells = width.saturating_sub(percent.len() + 1);
    let filled = (cells as f64 * fraction) as usize;
    let text: String = (0..cells).map(|i| if i < filled { '\u{2588}' } else { '\u{2591}' }).collect();
    grid.put(x, y, &text, BLACK, GREY);
    grid.put(x + cells as u16 + 1, y, &percent, BLACK, GREY);
}

fn text_width(lines: &[String], least: u16) -> u16 {
    (lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16 + 8).max(least)
}

/// Where a box sits.
struct Area {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

impl Area {
    fn centered(size: (u16, u16), title: &str, height: u16, width: u16) -> Self {
        let (cols, rows) = size;
        let w = width.max(title.chars().count() as u16 + 6).min(cols);
        let h = (height + 2).min(rows);
        Self { x: (cols - w) / 2, y: (rows - h) / 2, w, h }
    }

    /// The grey box, its shadow, the double border and the title.
    fn draw(&self, grid: &mut Grid, title: &str) {
        grid.fill(self.x + 2, self.y + 1, self.w, self.h, SHADOW);
        grid.fill(self.x, self.y, self.w, self.h, GREY);
        let (left, top, width, height) = (self.x + 1, self.y + 1, self.w - 2, self.h - 2);
        let line = "\u{2550}".repeat(width.saturating_sub(2) as usize);
        grid.put(left, top, &format!("\u{2554}{line}\u{2557}"), BLACK, GREY);
        grid.put(left, top + height - 1, &format!("\u{255A}{line}\u{255D}"), BLACK, GREY);
        for row in top + 1..top + height - 1 {
            grid.put(left, row, "\u{2551}", BLACK, GREY);
            grid.put(left + width - 1, row, "\u{2551}", BLACK, GREY);
        }
        let title = format!(" {title} ");
        grid.put(left + (width.saturating_sub(title.chars().count() as u16)) / 2, top, &title, BLACK, GREY);
    }

    /// Where the buttons of a row sit: (x, y, width) of each `[ label ]`.
    fn button_slots(&self, labels: &[&str]) -> Vec<(u16, u16, u16)> {
        let widths: Vec<u16> = labels.iter().map(|l| l.chars().count() as u16 + 4).collect();
        let total: u16 = widths.iter().sum::<u16>() + 2 * (widths.len() as u16 - 1);
        let mut col = self.x + self.w.saturating_sub(total) / 2;
        let row = self.y + self.h - 2;
        widths
            .into_iter()
            .map(|w| {
                let slot = (col, row, w);
                col += w + 2;
                slot
            })
            .collect()
    }

    /// A row of buttons at the bottom; the focused one is on cyan.
    fn buttons(&self, grid: &mut Grid, buttons: &[(&str, bool)]) {
        let labels: Vec<&str> = buttons.iter().map(|(name, _)| *name).collect();
        for ((x, y, _), (name, focused)) in self.button_slots(&labels).into_iter().zip(buttons) {
            grid.put(x, y, &format!("[ {name} ]"), BLACK, if *focused { CYAN } else { GREY });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(dialog: &mut Dialog, code: KeyCode) -> Outcome {
        dialog.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(dialog: &mut Dialog, text: &str) {
        for c in text.chars() {
            press(dialog, KeyCode::Char(c));
        }
    }

    #[test]
    fn a_prompt_edits_its_line_and_hands_it_over_on_enter() {
        let mut dialog = Dialog::prompt("Copy", "Copy to:".into(), "/tmp/x", "Copy", Action::MkDir);
        press(&mut dialog, KeyCode::Backspace);
        typed(&mut dialog, "yz");
        press(&mut dialog, KeyCode::Home);
        press(&mut dialog, KeyCode::Delete);
        match press(&mut dialog, KeyCode::Enter) {
            Outcome::Accepted(Action::MkDir, text) => assert_eq!(text, "tmp/yz"),
            _ => panic!("accepted with the edited text"),
        }
    }

    #[test]
    fn escape_and_the_cancel_button_close_a_prompt() {
        let mut dialog = Dialog::prompt("Copy", "to:".into(), "a", "Copy", Action::MkDir);
        assert!(matches!(press(&mut dialog, KeyCode::Esc), Outcome::Closed));
        press(&mut dialog, KeyCode::Tab);
        press(&mut dialog, KeyCode::Tab);
        assert!(matches!(press(&mut dialog, KeyCode::Enter), Outcome::Closed));
    }

    #[test]
    fn a_confirmation_needs_a_yes() {
        let mut dialog = Dialog::confirm("Delete", vec!["Delete x?".into()], "Delete", Action::Quit);
        assert!(matches!(press(&mut dialog, KeyCode::Right), Outcome::Open));
        assert!(matches!(press(&mut dialog, KeyCode::Enter), Outcome::Closed), "focus moved to Cancel");
        let mut dialog = Dialog::confirm("Delete", vec!["Delete x?".into()], "Delete", Action::Quit);
        assert!(matches!(press(&mut dialog, KeyCode::Enter), Outcome::Accepted(Action::Quit, _)));
    }

    #[test]
    fn clicks_press_buttons_and_place_the_cursor() {
        let size = (80, 24);
        let mut dialog = Dialog::prompt("Copy", "to:".into(), "abcdef", "Copy", Action::MkDir);
        let hits = dialog.hits(size);
        let (_, ix, iy, _) = hits.iter().find(|(h, ..)| matches!(h, Hit::Input(_))).map(|(_, x, y, w)| (0, *x, *y, *w)).unwrap();
        assert!(matches!(dialog.click(ix + 2, iy, size), Outcome::Open));
        press(&mut dialog, KeyCode::Char('X'));
        match dialog.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            Outcome::Accepted(_, text) => assert_eq!(text, "abXcdef"),
            _ => panic!("accepted"),
        }
        let (_, ox, oy, _) = hits.iter().find(|(h, ..)| matches!(h, Hit::Ok)).map(|(_, x, y, w)| (0, *x, *y, *w)).unwrap();
        assert!(matches!(dialog.click(ox + 1, oy, size), Outcome::Accepted(Action::MkDir, _)));
        let (_, cx, cy, _) = hits.iter().find(|(h, ..)| matches!(h, Hit::Cancel)).map(|(_, x, y, w)| (0, *x, *y, *w)).unwrap();
        assert!(matches!(dialog.click(cx + 1, cy, size), Outcome::Closed));
        assert!(matches!(dialog.click(0, 0, size), Outcome::Open), "outside does nothing");
    }

    #[test]
    fn dialogs_paint_inside_the_screen_and_a_prompt_shows_its_cursor() {
        let mut grid = Grid::new(80, 24);
        let dialog = Dialog::prompt("Copy", "Copy file \"a\" to:".into(), "C:\\x", "Copy", Action::MkDir);
        let cursor = dialog.paint(&mut grid).expect("the cursor is in the input");
        assert!(cursor.0 < 80 && cursor.1 < 24);
        Dialog::message("Error", "It failed\nfor a reason").paint(&mut grid);
        Dialog::confirm("Delete", vec!["Delete 3 items?".into()], "Delete", Action::Quit).paint(&mut grid);
        // a tiny screen must not panic
        let mut tiny = Grid::new(10, 4);
        dialog.paint(&mut tiny);
    }
}
