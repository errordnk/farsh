//! Far style dialogs: a grey (or red, for warnings) box with a shadow, a
//! double frame with the title in it, separators, an input line, radio
//! buttons, a drop-down, check boxes and buttons, with hot keys.

use std::path::PathBuf;
use std::time::SystemTime;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::Color;

use crate::fileops::{Conflict, Job, Kind};
use crate::grid::Grid;
use crate::palette;


/// What an accepted dialog asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Copy(Vec<PathBuf>),
    Move(Vec<PathBuf>),
    MkDir,
    Delete(Vec<PathBuf>),
    Quit,
    /// The answer to a file that already exists.
    Conflict,
    /// Just something to read.
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Normal,
    /// Red: deleting, errors, a file in the way.
    Warning,
}

struct Palette {
    bg: Color,
    fg: Color,
    hot: Color,
    focus_bg: Color,
    focus_fg: Color,
    edit_bg: Color,
    edit_fg: Color,
    dim: Color,
}

impl Style {
    fn palette(self) -> Palette {
        match self {
            // Far's COL_DIALOG*: black on light gray, yellow hotkeys, cyan focus and edits
            Style::Normal => Palette {
                bg: palette::bg(palette::LIGHT_GRAY),
                fg: palette::fg(palette::BLACK),
                hot: palette::fg(palette::YELLOW),
                focus_bg: palette::bg(palette::CYAN),
                focus_fg: palette::fg(palette::BLACK),
                edit_bg: palette::bg(palette::CYAN),
                edit_fg: palette::fg(palette::BLACK),
                dim: palette::fg(palette::DARK_GRAY),
            },
            // COL_WARNDIALOG*: white on red, the focus black on light gray
            Style::Warning => Palette {
                bg: palette::bg(palette::RED),
                fg: palette::fg(palette::WHITE),
                hot: palette::fg(palette::YELLOW),
                focus_bg: palette::bg(palette::LIGHT_GRAY),
                focus_fg: palette::fg(palette::BLACK),
                edit_bg: palette::bg(palette::CYAN),
                edit_fg: palette::fg(palette::BLACK),
                dim: palette::fg(palette::DARK_GRAY),
            },
        }
    }
}

pub struct Button {
    /// With `&` before the hot key.
    pub label: &'static str,
    pub id: &'static str,
    pub default: bool,
    pub enabled: bool,
}

pub enum Item {
    Text { text: String, center: bool },
    /// The `↓` at its end says it has a history in Far; here it only shows that it is a field.
    Edit { id: &'static str, text: Vec<char>, cursor: usize },
    Radio { id: &'static str, label: &'static str, options: Vec<&'static str>, selected: usize },
    Check { id: &'static str, label: &'static str, checked: bool, enabled: bool },
    Combo { id: &'static str, label: &'static str, options: Vec<&'static str>, selected: usize },
    Separator(Option<&'static str>),
    Buttons(Vec<Button>),
}

/// What the dialog's controls say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values {
    texts: Vec<(&'static str, String)>,
    checks: Vec<(&'static str, bool)>,
    choices: Vec<(&'static str, usize)>,
}

impl Values {
    pub fn text(&self, id: &str) -> String {
        self.texts.iter().find(|(k, _)| *k == id).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    pub fn checked(&self, id: &str) -> bool {
        self.checks.iter().find(|(k, _)| *k == id).is_some_and(|(_, v)| *v)
    }

    pub fn choice(&self, id: &str) -> usize {
        self.choices.iter().find(|(k, _)| *k == id).map_or(0, |(_, v)| *v)
    }
}

pub struct Dialog {
    title: String,
    style: Style,
    /// The least frame width wanted.
    width: u16,
    items: Vec<Item>,
    focus: usize,
    /// The focused button, when the focus is on a row of buttons.
    button: usize,
    action: Action,
}

pub enum Outcome {
    Open,
    Closed,
    /// A button was pressed (its id), with what the controls said.
    Pressed(Action, &'static str, Values),
}

// ---------------------------------------------------------------- building

fn text(text: impl Into<String>) -> Item {
    Item::Text { text: text.into(), center: false }
}

fn centered(text: impl Into<String>) -> Item {
    Item::Text { text: text.into(), center: true }
}

fn edit(id: &'static str, value: &str) -> Item {
    let text: Vec<char> = value.chars().collect();
    let cursor = text.len();
    Item::Edit { id, text, cursor }
}

fn button(label: &'static str, id: &'static str, default: bool) -> Button {
    Button { label, id, default, enabled: true }
}

impl Dialog {
    /// F5 and F6: where to, and how.
    pub fn copy_or_move(kind: Kind, label: String, dest: &str, action: Action) -> Self {
        let moving = kind == Kind::Move;
        let mut items = vec![text(label), edit("dest", dest), Item::Separator(None)];
        items.push(Item::Radio { id: "access", label: "&Access rights:", options: vec!["Default", "Copy", "Inherit"], selected: 0 });
        items.push(Item::Separator(None));
        items.push(Item::Combo { id: "existing", label: "Already e&xisting files:", options: vec!["Ask", "Overwrite", "Skip"], selected: 0 });
        if !moving {
            items.push(Item::Check { id: "timestamps", label: "&Preserve all timestamps", checked: false, enabled: true });
            items.push(Item::Check { id: "links", label: "Cop&y contents of symbolic links", checked: false, enabled: true });
        }
        items.push(Item::Check { id: "multiple", label: "Process &multiple destinations", checked: false, enabled: true });
        items.push(Item::Separator(None));
        items.push(Item::Check { id: "filter", label: "&Use filter", checked: false, enabled: false });
        items.push(Item::Separator(None));
        let ok = if moving { "&Rename" } else { "&Copy" };
        items.push(Item::Buttons(vec![
            button(ok, "ok", true),
            Button { label: "&Filter", id: "filter", default: false, enabled: false },
            button("Ca&ncel", "cancel", false),
        ]));
        Self { title: if moving { "Rename/Move" } else { "Copy" }.into(), style: Style::Normal, width: 70, items, focus: 1, button: 0, action }
    }

    /// F7.
    pub fn mkdir() -> Self {
        Self {
            title: "Make folder".into(),
            style: Style::Normal,
            width: 62,
            items: vec![
                text("Create the folder"),
                edit("name", ""),
                Item::Separator(None),
                Item::Check { id: "multiple", label: "Process &multiple names", checked: false, enabled: true },
                Item::Separator(None),
                Item::Buttons(vec![button("&OK", "ok", true), button("&Cancel", "cancel", false)]),
            ],
            focus: 1,
            button: 0,
            action: Action::MkDir,
        }
    }

    /// F8: the question is the `lines`.
    pub fn delete(lines: Vec<String>, action: Action) -> Self {
        let mut items: Vec<Item> = lines.into_iter().map(centered).collect();
        items.push(Item::Separator(None));
        items.push(Item::Buttons(vec![button("&Delete", "ok", true), button("&Cancel", "cancel", false)]));
        let focus = items.len() - 1;
        Self { title: "Delete".into(), style: Style::Warning, width: 0, items, focus, button: 0, action }
    }

    /// A question with Yes and No.
    pub fn confirm(title: &str, line: &str, action: Action) -> Self {
        let items = vec![centered(line), Item::Separator(None), Item::Buttons(vec![button("&Yes", "ok", true), button("&No", "cancel", false)])];
        Self { title: title.into(), style: Style::Normal, width: 0, items, focus: 2, button: 0, action }
    }

    /// An error to read.
    pub fn error(text: &str) -> Self {
        let mut items: Vec<Item> = text.lines().map(centered).collect();
        items.push(Item::Separator(None));
        items.push(Item::Buttons(vec![button("&OK", "ok", true)]));
        let focus = items.len() - 1;
        Self { title: "Error".into(), style: Style::Warning, width: 0, items, focus, button: 0, action: Action::Info }
    }

    /// Something to read, in the normal colors.
    pub fn message(title: &str, text: &str) -> Self {
        let mut dialog = Self::error(text);
        dialog.title = title.into();
        dialog.style = Style::Normal;
        dialog
    }

    /// A file is in the way of a copy.
    pub fn conflict(conflict: &Conflict) -> Self {
        let info = |label: &str, size: u64, modified: Option<SystemTime>| format!("{label:<9}{:>16} bytes  {}", group(size), date(modified));
        let items = vec![
            centered("File already exists"),
            centered(conflict.dst.display().to_string()),
            Item::Separator(None),
            text(info("New", conflict.new_size, conflict.new_modified)),
            text(info("Existing", conflict.old_size, conflict.old_modified)),
            Item::Separator(None),
            Item::Check { id: "remember", label: "&Remember choice", checked: false, enabled: true },
            Item::Separator(None),
            Item::Buttons(vec![button("&Overwrite", "overwrite", true), button("&Skip", "skip", false), button("&Cancel", "cancel", false)]),
        ];
        Self { title: "Warning".into(), style: Style::Warning, width: 60, items, focus: 8, button: 0, action: Action::Conflict }
    }

    pub fn values(&self) -> Values {
        let mut values = Values::default();
        for item in &self.items {
            match item {
                Item::Edit { id, text, .. } => values.texts.push((id, text.iter().collect())),
                Item::Check { id, checked, .. } => values.checks.push((id, *checked)),
                Item::Radio { id, selected, .. } | Item::Combo { id, selected, .. } => values.choices.push((id, *selected)),
                _ => {}
            }
        }
        values
    }
}

// ---------------------------------------------------------------- layout

/// Where a dialog sits and where its parts are.
struct Frame {
    /// The grey box, with the shadow beside it.
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    /// The double frame inside it.
    fx: u16,
    fy: u16,
    fw: u16,
    fh: u16,
}

impl Frame {
    /// A box for `rows` rows of content in a frame at least `want` wide.
    fn new(size: (u16, u16), title: &str, want: u16, rows: u16) -> Self {
        let (cols, screen_rows) = size;
        let w = (want.max(title.chars().count() as u16 + 8) + 6).min(cols);
        let h = (rows + 4).min(screen_rows);
        let (fx_off, fy_off) = (3.min(w / 2), 1);
        Self {
            x: (cols - w) / 2,
            y: (screen_rows - h) / 2,
            w,
            h,
            fx: (cols - w) / 2 + fx_off,
            fy: (screen_rows - h) / 2 + fy_off,
            fw: w.saturating_sub(2 * fx_off),
            fh: h.saturating_sub(2 * fy_off),
        }
    }

    fn text_x(&self) -> u16 {
        self.fx + 2
    }

    fn text_w(&self) -> u16 {
        self.fw.saturating_sub(4)
    }

    fn row_y(&self, index: usize) -> u16 {
        self.fy + 1 + index as u16
    }

    /// The shadow, the box, the double frame and the title in it.
    fn draw(&self, grid: &mut Grid, title: &str, p: &Palette) {
        grid.fill_with(self.x, self.y, self.w, self.h, p.fg, p.bg);
        palette::drop_shadow(grid, self.x, self.y, self.w, self.h);
        let line = "\u{2550}".repeat(self.fw.saturating_sub(2) as usize);
        grid.put(self.fx, self.fy, &format!("\u{2554}{line}\u{2557}"), p.fg, p.bg);
        grid.put(self.fx, self.fy + self.fh - 1, &format!("\u{255A}{line}\u{255D}"), p.fg, p.bg);
        for row in self.fy + 1..self.fy + self.fh - 1 {
            grid.put(self.fx, row, "\u{2551}", p.fg, p.bg);
            grid.put(self.fx + self.fw - 1, row, "\u{2551}", p.fg, p.bg);
        }
        let title = format!(" {title} ");
        grid.put(self.fx + self.fw.saturating_sub(title.chars().count() as u16) / 2, self.fy, &title, p.fg, p.bg);
    }

    /// A separator from edge to edge of the frame, with a title in it if given.
    fn separator(&self, grid: &mut Grid, row: u16, title: Option<&str>, p: &Palette) {
        let line = "\u{2500}".repeat(self.fw.saturating_sub(2) as usize);
        grid.put(self.fx, row, &format!("\u{255F}{line}\u{2562}"), p.fg, p.bg);
        if let Some(title) = title {
            let title = format!(" {title} ");
            grid.put(self.fx + self.fw.saturating_sub(title.chars().count() as u16) / 2, row, &title, p.fg, p.bg);
        }
    }
}

/// What a click or the focus can land on: (sub item, x, width).
type Span = (usize, u16, u16);

fn label_len(label: &str) -> u16 {
    label.replace('&', "").chars().count() as u16
}

/// The parts of an item along its row.
fn spans(item: &Item, frame: &Frame) -> Vec<Span> {
    let (x, w) = (frame.text_x(), frame.text_w());
    match item {
        Item::Text { .. } | Item::Separator(_) => vec![],
        Item::Edit { .. } => vec![(0, x, w)],
        Item::Check { label, .. } => vec![(0, x, 4 + label_len(label))],
        Item::Radio { label, options, .. } => {
            let mut at = x + label_len(label) + 1;
            options
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let width = 4 + name.chars().count() as u16;
                    let span = (i, at, width);
                    at += width + 1;
                    span
                })
                .collect()
        }
        Item::Combo { label, .. } => {
            let start = x + label_len(label) + 1;
            vec![(0, start, (x + w).saturating_sub(start))]
        }
        Item::Buttons(buttons) => {
            let widths: Vec<u16> = buttons.iter().map(|b| label_len(b.label) + 4).collect();
            let total: u16 = widths.iter().sum::<u16>() + widths.len().saturating_sub(1) as u16;
            let mut at = x + w.saturating_sub(total) / 2;
            widths
                .into_iter()
                .enumerate()
                .map(|(i, width)| {
                    let span = (i, at, width);
                    at += width + 1;
                    span
                })
                .collect()
        }
    }
}

/// The width the content wants, without the frame's margins.
fn natural_width(item: &Item) -> u16 {
    match item {
        Item::Text { text, .. } => text.chars().count() as u16,
        Item::Edit { .. } | Item::Separator(_) => 0,
        Item::Check { label, .. } => 4 + label_len(label),
        Item::Radio { label, options, .. } => {
            label_len(label) + 1 + options.iter().map(|o| 5 + o.chars().count() as u16).sum::<u16>()
        }
        Item::Combo { label, options, .. } => label_len(label) + 3 + options.iter().map(|o| o.chars().count() as u16).max().unwrap_or(0),
        Item::Buttons(buttons) => buttons.iter().map(|b| label_len(b.label) + 5).sum(),
    }
}

/// Draws `label` with the letter after `&` in the hot color.
fn put_label(grid: &mut Grid, x: u16, y: u16, label: &str, fg: Color, bg: Color, hot: Color) {
    let mut col = x;
    let mut next_is_hot = false;
    for c in label.chars() {
        if c == '&' && !next_is_hot {
            next_is_hot = true;
            continue;
        }
        grid.put(col, y, &c.to_string(), if next_is_hot { hot } else { fg }, bg);
        next_is_hot = false;
        col += 1;
    }
}

fn hot_key(label: &str) -> Option<char> {
    let at = label.find('&')?;
    label[at + 1..].chars().next().map(|c| c.to_lowercase().next().unwrap_or(c))
}

// ---------------------------------------------------------------- drawing

impl Dialog {
    fn frame(&self, size: (u16, u16)) -> Frame {
        let natural = self.items.iter().map(natural_width).max().unwrap_or(0) + 4;
        let want = natural.max(self.width).min(size.0.saturating_sub(6));
        Frame::new(size, &self.title, want, self.items.len() as u16)
    }

    /// Draws the dialog centered on `grid`; returns where the text cursor goes.
    pub fn paint(&self, grid: &mut Grid) -> Option<(u16, u16)> {
        let p = self.style.palette();
        let frame = self.frame((grid.cols, grid.rows));
        frame.draw(grid, &self.title, &p);
        let mut cursor = None;
        for (i, item) in self.items.iter().enumerate() {
            let y = frame.row_y(i);
            let focused = i == self.focus;
            let spans = spans(item, &frame);
            match item {
                Item::Text { text, center } => {
                    let x = if *center {
                        frame.text_x() + frame.text_w().saturating_sub(text.chars().count() as u16) / 2
                    } else {
                        frame.text_x()
                    };
                    grid.put(x, y, text, p.fg, p.bg);
                }
                Item::Separator(title) => frame.separator(grid, y, *title, &p),
                Item::Edit { text, cursor: at, .. } => {
                    let (x, w) = (spans[0].1, spans[0].2);
                    let visible = w.saturating_sub(1) as usize;
                    let start = (*at + 1).saturating_sub(visible);
                    grid.fill(x, y, w, 1, p.edit_bg);
                    grid.put(x, y, &text.iter().skip(start).take(visible).collect::<String>(), p.edit_fg, p.edit_bg);
                    grid.put((x + w).saturating_sub(1), y, "\u{2193}", p.edit_fg, p.edit_bg);
                    if focused {
                        cursor = Some((x + at.saturating_sub(start) as u16, y));
                    }
                }
                Item::Check { label, checked, enabled, .. } => {
                    let (fg, bg) = if !enabled {
                        (p.dim, p.bg)
                    } else if focused {
                        (p.focus_fg, p.focus_bg)
                    } else {
                        (p.fg, p.bg)
                    };
                    let hot = if focused && *enabled { p.focus_fg } else { p.hot };
                    grid.put(spans[0].1, y, &format!("[{}] ", if *checked { 'x' } else { ' ' }), fg, bg);
                    put_label(grid, spans[0].1 + 4, y, label, fg, bg, if *enabled { hot } else { p.dim });
                }
                Item::Radio { label, options, selected, .. } => {
                    put_label(grid, frame.text_x(), y, label, p.fg, p.bg, p.hot);
                    for ((i, x, _), name) in spans.iter().zip(options) {
                        let on = i == selected;
                        let (fg, bg) = if focused && on { (p.focus_fg, p.focus_bg) } else { (p.fg, p.bg) };
                        grid.put(*x, y, &format!("({}) {name}", if on { '\u{2022}' } else { ' ' }), fg, bg);
                    }
                }
                Item::Combo { label, options, selected, .. } => {
                    put_label(grid, frame.text_x(), y, label, p.fg, p.bg, p.hot);
                    let (x, w) = (spans[0].1, spans[0].2);
                    let (fg, bg) = if focused { (p.focus_fg, p.focus_bg) } else { (p.edit_fg, p.edit_bg) };
                    grid.fill(x, y, w, 1, bg);
                    grid.put(x, y, options[*selected], fg, bg);
                    grid.put((x + w).saturating_sub(1), y, "\u{2193}", fg, bg);
                }
                Item::Buttons(buttons) => {
                    for ((i, x, _), b) in spans.iter().zip(buttons) {
                        let on = focused && *i == self.button;
                        let (fg, bg) = if !b.enabled {
                            (p.dim, p.bg)
                        } else if on {
                            (p.focus_fg, p.focus_bg)
                        } else {
                            (p.fg, p.bg)
                        };
                        let (open, close) = if b.default { ('{', '}') } else { ('[', ']') };
                        grid.put(*x, y, &format!("{open} "), fg, bg);
                        put_label(grid, *x + 2, y, b.label, fg, bg, if !b.enabled { p.dim } else if on { p.focus_fg } else { p.hot });
                        grid.put(*x + 2 + label_len(b.label), y, &format!(" {close}"), fg, bg);
                    }
                }
            }
        }
        cursor
    }
}

// ---------------------------------------------------------------- input

impl Dialog {
    fn focusable(&self, index: usize) -> bool {
        match &self.items[index] {
            Item::Edit { .. } | Item::Radio { .. } | Item::Combo { .. } => true,
            Item::Check { enabled, .. } => *enabled,
            Item::Buttons(buttons) => buttons.iter().any(|b| b.enabled),
            Item::Text { .. } | Item::Separator(_) => false,
        }
    }

    fn move_focus(&mut self, forward: bool) {
        let count = self.items.len();
        let mut at = self.focus;
        for _ in 0..count {
            at = if forward { (at + 1) % count } else { (at + count - 1) % count };
            if self.focusable(at) {
                self.focus = at;
                self.button = self.first_enabled_button();
                return;
            }
        }
    }

    fn first_enabled_button(&self) -> usize {
        match &self.items[self.focus] {
            Item::Buttons(buttons) => buttons.iter().position(|b| b.enabled && b.default).or_else(|| buttons.iter().position(|b| b.enabled)).unwrap_or(0),
            _ => 0,
        }
    }

    /// Presses the default button (Enter anywhere but on a button).
    fn press_default(&self) -> Outcome {
        for item in &self.items {
            if let Item::Buttons(buttons) = item {
                if let Some(b) = buttons.iter().find(|b| b.default && b.enabled) {
                    return self.press(b.id);
                }
            }
        }
        Outcome::Open
    }

    fn press(&self, id: &'static str) -> Outcome {
        if id == "cancel" {
            Outcome::Closed
        } else {
            Outcome::Pressed(self.action.clone(), id, self.values())
        }
    }

    fn cycle(&mut self, step: isize) {
        match &mut self.items[self.focus] {
            Item::Radio { options, selected, .. } | Item::Combo { options, selected, .. } => {
                *selected = (*selected as isize + step).rem_euclid(options.len() as isize) as usize;
            }
            _ => {}
        }
    }

    /// A hot key: focuses the control, toggles a check box, presses a button.
    fn hot(&mut self, c: char) -> Option<Outcome> {
        let c = c.to_lowercase().next().unwrap_or(c);
        for index in 0..self.items.len() {
            match &mut self.items[index] {
                Item::Check { label, checked, enabled, .. } if *enabled && hot_key(label) == Some(c) => {
                    *checked = !*checked;
                    self.focus = index;
                    return Some(Outcome::Open);
                }
                Item::Radio { label, .. } | Item::Combo { label, .. } if hot_key(label) == Some(c) => {
                    self.focus = index;
                    return Some(Outcome::Open);
                }
                Item::Buttons(buttons) => {
                    let id = buttons.iter().find(|b| b.enabled && hot_key(b.label) == Some(c)).map(|b| b.id);
                    if let Some(id) = id {
                        return Some(self.press(id));
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if key.code == KeyCode::Esc {
            return Outcome::Closed;
        }
        let on_edit = matches!(self.items[self.focus], Item::Edit { .. });
        if let KeyCode::Char(c) = key.code {
            // Alt+letter always, a bare letter where nothing is being typed.
            if (alt || (!on_edit && !ctrl)) && !(c == ' ') {
                if let Some(outcome) = self.hot(c) {
                    return outcome;
                }
            }
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.move_focus(true),
            KeyCode::BackTab | KeyCode::Up => self.move_focus(false),
            KeyCode::Enter => {
                if let Item::Buttons(buttons) = &self.items[self.focus] {
                    return match buttons.get(self.button).filter(|b| b.enabled) {
                        Some(b) => self.press(b.id),
                        None => Outcome::Open,
                    };
                }
                return self.press_default();
            }
            KeyCode::Left | KeyCode::Right => {
                let step = if key.code == KeyCode::Left { -1 } else { 1 };
                match &mut self.items[self.focus] {
                    Item::Edit { text, cursor, .. } => *cursor = (*cursor as isize + step).clamp(0, text.len() as isize) as usize,
                    Item::Buttons(buttons) => {
                        let count = buttons.len() as isize;
                        let mut at = self.button as isize;
                        for _ in 0..count {
                            at = (at + step).rem_euclid(count);
                            if buttons[at as usize].enabled {
                                self.button = at as usize;
                                break;
                            }
                        }
                    }
                    Item::Radio { .. } | Item::Combo { .. } => self.cycle(step),
                    _ => {}
                }
            }
            KeyCode::Home => {
                if let Item::Edit { cursor, .. } = &mut self.items[self.focus] {
                    *cursor = 0;
                }
            }
            KeyCode::End => {
                if let Item::Edit { text, cursor, .. } = &mut self.items[self.focus] {
                    *cursor = text.len();
                }
            }
            KeyCode::Char(' ') => match &mut self.items[self.focus] {
                Item::Check { checked, .. } => *checked = !*checked,
                Item::Radio { .. } | Item::Combo { .. } => self.cycle(1),
                Item::Edit { text, cursor, .. } => {
                    text.insert(*cursor, ' ');
                    *cursor += 1;
                }
                Item::Buttons(buttons) => {
                    let id = buttons.get(self.button).filter(|b| b.enabled).map(|b| b.id);
                    if let Some(id) = id {
                        return self.press(id);
                    }
                }
                _ => {}
            },
            KeyCode::Backspace => {
                if let Item::Edit { text, cursor, .. } = &mut self.items[self.focus] {
                    if *cursor > 0 {
                        *cursor -= 1;
                        text.remove(*cursor);
                    }
                }
            }
            KeyCode::Delete => {
                if let Item::Edit { text, cursor, .. } = &mut self.items[self.focus] {
                    if *cursor < text.len() {
                        text.remove(*cursor);
                    }
                }
            }
            KeyCode::Char('u') if ctrl => {
                if let Item::Edit { text, cursor, .. } = &mut self.items[self.focus] {
                    text.clear();
                    *cursor = 0;
                }
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                if let Item::Edit { text, cursor, .. } = &mut self.items[self.focus] {
                    text.insert(*cursor, c);
                    *cursor += 1;
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    /// What a click at (`col`, `row`) does, on a screen of `size`.
    pub fn click(&mut self, col: u16, row: u16, size: (u16, u16)) -> Outcome {
        let frame = self.frame(size);
        for index in 0..self.items.len() {
            if row != frame.row_y(index) {
                continue;
            }
            for (sub, x, w) in spans(&self.items[index], &frame) {
                if col < x || col >= x + w {
                    continue;
                }
                if !self.focusable(index) {
                    return Outcome::Open;
                }
                self.focus = index;
                match &mut self.items[index] {
                    Item::Edit { text, cursor, .. } => {
                        let visible = w.saturating_sub(1) as usize;
                        let start = (*cursor + 1).saturating_sub(visible);
                        *cursor = (start + (col - x) as usize).min(text.len());
                    }
                    Item::Check { checked, .. } => *checked = !*checked,
                    Item::Radio { selected, .. } => *selected = sub,
                    Item::Combo { .. } => self.cycle(1),
                    Item::Buttons(buttons) => {
                        self.button = sub;
                        let id = buttons.get(sub).filter(|b| b.enabled).map(|b| b.id);
                        if let Some(id) = id {
                            return self.press(id);
                        }
                    }
                    _ => {}
                }
                return Outcome::Open;
            }
        }
        Outcome::Open
    }
}

// ---------------------------------------------------------------- progress

/// The progress box of a running copy, move or delete.
pub fn paint_progress(job: &Job, grid: &mut Grid) {
    let p = Style::Normal.palette();
    let progress = &job.progress;
    let copying = job.kind != Kind::Delete;
    let rows: u16 = if copying { 11 } else { 7 };
    let frame = Frame::new((grid.cols, grid.rows), job.kind.title(), (grid.cols * 3 / 4).clamp(40, 74), rows);
    frame.draw(grid, job.kind.title(), &p);
    let (x, w) = (frame.text_x(), frame.text_w() as usize);
    let fit = |text: &str| -> String {
        let count = text.chars().count();
        if count <= w {
            text.to_string()
        } else {
            format!("\u{2026}{}", text.chars().skip(count - w + 1).collect::<String>())
        }
    };
    let mut row = 0;
    let mut line = |grid: &mut Grid, text: &str| {
        grid.put(x, frame.row_y(row), text, p.fg, p.bg);
        row += 1;
    };
    let verb = match job.kind {
        Kind::Copy => "Copying the file",
        Kind::Move => "Moving the file",
        Kind::Delete => "Deleting the file",
    };
    line(grid, verb);
    line(grid, &fit(&progress.from));
    if copying {
        line(grid, "to");
        line(grid, &fit(&progress.to));
        bar(grid, x, frame.row_y(row), w, progress.file_done, progress.file_size, &p);
        row += 1;
    }
    frame.separator(grid, frame.row_y(row), Some("Total"), &p);
    row += 1;
    let right = |grid: &mut Grid, y: u16, label: &str, value: &str| {
        grid.put(x, y, label, p.fg, p.bg);
        grid.put(x + (w as u16).saturating_sub(value.chars().count() as u16), y, value, p.fg, p.bg);
    };
    right(grid, frame.row_y(row), "Files:", &format!("{} / {}", progress.files_done, progress.files_total));
    row += 1;
    if copying {
        right(grid, frame.row_y(row), "Bytes:", &format!("{} / {}", group(progress.bytes_done), group(progress.bytes_total)));
        row += 1;
        bar(grid, x, frame.row_y(row), w, progress.bytes_done, progress.bytes_total, &p);
    } else {
        bar(grid, x, frame.row_y(row), w, progress.files_done as u64, progress.files_total as u64, &p);
    }
    row += 1;
    frame.separator(grid, frame.row_y(row), None, &p);
    row += 1;
    let seconds = job.elapsed().as_secs_f64().max(0.001);
    let speed = progress.bytes_done as f64 / seconds;
    let left = if speed > 1.0 { (progress.bytes_total.saturating_sub(progress.bytes_done) as f64 / speed) as u64 } else { 0 };
    let y = frame.row_y(row);
    let time = format!("Time: {}", clock(seconds as u64));
    let remaining = format!("Remaining: {}", clock(left));
    let rate = format!("{} MB/s", (speed / 1_048_576.0) as u64);
    grid.put(x, y, &time, p.fg, p.bg);
    grid.put(x + (w as u16).saturating_sub(remaining.chars().count() as u16) / 2, y, &remaining, p.fg, p.bg);
    grid.put(x + (w as u16).saturating_sub(rate.chars().count() as u16), y, &rate, p.fg, p.bg);
}

/// A bar `width` cells wide with the percentage at its end.
fn bar(grid: &mut Grid, x: u16, y: u16, width: usize, done: u64, total: u64, p: &Palette) {
    let fraction = if total == 0 { 0.0 } else { (done as f64 / total as f64).min(1.0) };
    let percent = format!("{:>3}%", (fraction * 100.0) as u32);
    let cells = width.saturating_sub(percent.len() + 1);
    let filled = (cells as f64 * fraction) as usize;
    let text: String = (0..cells).map(|i| if i < filled { '\u{2588}' } else { '\u{2591}' }).collect();
    grid.put(x, y, &text, p.fg, p.bg);
    grid.put(x + cells as u16 + 1, y, &percent, p.fg, p.bg);
}

fn clock(seconds: u64) -> String {
    format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// 1234567 as `1 234 567`.
pub fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// `03.11.26 00:02`, in the machine's local time.
fn date(time: Option<SystemTime>) -> String {
    time.map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%d.%m.%y %H:%M").to_string()).unwrap_or_default()
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

    fn copy_dialog() -> Dialog {
        Dialog::copy_or_move(Kind::Copy, "Copy file \"a\" to:".into(), "/tmp/x", Action::Copy(vec![]))
    }

    fn pressed(outcome: Outcome) -> (&'static str, Values) {
        match outcome {
            Outcome::Pressed(_, id, values) => (id, values),
            _ => panic!("a button was pressed"),
        }
    }

    #[test]
    fn the_input_line_is_edited_and_enter_presses_the_default_button() {
        let mut dialog = copy_dialog();
        press(&mut dialog, KeyCode::Backspace);
        typed(&mut dialog, "yz");
        press(&mut dialog, KeyCode::Home);
        press(&mut dialog, KeyCode::Delete);
        let (id, values) = pressed(press(&mut dialog, KeyCode::Enter));
        assert_eq!(id, "ok");
        assert_eq!(values.text("dest"), "tmp/yz");
    }

    #[test]
    fn escape_and_the_cancel_button_close_the_dialog() {
        let mut dialog = copy_dialog();
        assert!(matches!(press(&mut dialog, KeyCode::Esc), Outcome::Closed));
        // Alt+N is the hot key of Cancel
        assert!(matches!(dialog.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT)), Outcome::Closed));
    }

    #[test]
    fn the_controls_are_reached_with_tab_and_changed_with_the_keys() {
        let mut dialog = copy_dialog();
        press(&mut dialog, KeyCode::Tab); // access rights
        press(&mut dialog, KeyCode::Right); // Copy
        press(&mut dialog, KeyCode::Tab); // existing files
        press(&mut dialog, KeyCode::Right); // Overwrite
        press(&mut dialog, KeyCode::Tab); // timestamps
        press(&mut dialog, KeyCode::Char(' '));
        let (_, values) = pressed(dialog.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        assert_eq!(values.choice("access"), 1);
        assert_eq!(values.choice("existing"), 1);
        assert!(values.checked("timestamps"));
        assert!(!values.checked("links"));
    }

    #[test]
    fn hot_keys_toggle_check_boxes_and_a_disabled_one_is_skipped() {
        let mut dialog = copy_dialog();
        dialog.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::ALT));
        assert!(dialog.values().checked("multiple"));
        dialog.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::ALT));
        assert!(!dialog.values().checked("filter"), "the disabled filter stays off");
        // Tab goes round without ever stopping on it or on its button
        let mut stops = Vec::new();
        for _ in 0..8 {
            press(&mut dialog, KeyCode::Tab);
            stops.push(dialog.focus);
        }
        assert!(stops.iter().all(|&i| dialog.focusable(i)));
    }

    #[test]
    fn a_question_is_answered_with_y_and_n_without_alt() {
        let mut dialog = Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit);
        assert!(matches!(press(&mut dialog, KeyCode::Char('n')), Outcome::Closed));
        let (id, _) = pressed(press(&mut dialog, KeyCode::Char('y')));
        assert_eq!(id, "ok");
        let mut dialog = Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit);
        press(&mut dialog, KeyCode::Right);
        assert!(matches!(press(&mut dialog, KeyCode::Enter), Outcome::Closed), "focus moved to No");
    }

    #[test]
    fn typing_letters_into_the_input_does_not_trigger_hot_keys() {
        let mut dialog = Dialog::mkdir();
        typed(&mut dialog, "ocm");
        let (_, values) = pressed(press(&mut dialog, KeyCode::Enter));
        assert_eq!(values.text("name"), "ocm");
        assert!(!values.checked("multiple"));
    }

    #[test]
    fn the_conflict_dialog_offers_overwrite_skip_cancel_and_remembers() {
        let conflict = Conflict { dst: "/x/y.txt".into(), new_size: 12_345, new_modified: None, old_size: 7, old_modified: None };
        let mut dialog = Dialog::conflict(&conflict);
        dialog.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT));
        let (id, values) = pressed(dialog.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)));
        assert_eq!(id, "skip");
        assert!(values.checked("remember"));
    }

    #[test]
    fn clicks_press_buttons_toggle_checks_and_place_the_cursor() {
        let size = (100, 30);
        let mut dialog = copy_dialog();
        let frame = dialog.frame(size);
        // the input line is row 1
        let (_, ix, _) = spans(&dialog.items[1], &frame)[0];
        assert!(matches!(dialog.click(ix + 2, frame.row_y(1), size), Outcome::Open));
        typed(&mut dialog, "X");
        // the multiple-destinations check box
        let row = dialog.items.iter().position(|i| matches!(i, Item::Check { id: "multiple", .. })).unwrap();
        let (_, cx, _) = spans(&dialog.items[row], &frame)[0];
        dialog.click(cx + 1, frame.row_y(row), size);
        // the Copy button
        let last = dialog.items.len() - 1;
        let (_, bx, _) = spans(&dialog.items[last], &frame)[0];
        let (id, values) = pressed(dialog.click(bx + 3, frame.row_y(last), size));
        assert_eq!(id, "ok");
        assert_eq!(values.text("dest"), "/tXmp/x");
        assert!(values.checked("multiple"));
        assert!(matches!(dialog.click(0, 0, size), Outcome::Open), "outside does nothing");
    }

    #[test]
    fn dialogs_paint_inside_the_screen_with_a_cursor_in_the_input() {
        let mut grid = Grid::new(100, 30);
        let cursor = copy_dialog().paint(&mut grid).expect("the cursor is in the input");
        assert!(cursor.0 < 100 && cursor.1 < 30);
        Dialog::mkdir().paint(&mut grid);
        Dialog::error("It failed\nfor a reason").paint(&mut grid);
        Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit).paint(&mut grid);
        Dialog::delete(vec!["Do you wish to delete".into(), "file \"a\"".into()], Action::Quit).paint(&mut grid);
        // a tiny screen must not panic
        let mut tiny = Grid::new(10, 4);
        copy_dialog().paint(&mut tiny);
        Dialog::mkdir().paint(&mut tiny);
    }

    #[test]
    fn the_frame_has_far_margins_and_a_titled_top() {
        let mut grid = Grid::new(100, 30);
        Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit).paint(&mut grid);
        let frame = Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit).frame((100, 30));
        assert_eq!(frame.fx - frame.x, 3, "three columns of grey before the frame");
        assert_eq!(frame.fy - frame.y, 1);
        assert_eq!(grid.cell_mut(frame.fx, frame.fy).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(frame.fx, frame.fy + 2).unwrap().text, "\u{255F}", "the separator joins the frame");
    }

    #[test]
    fn numbers_are_grouped_and_dates_are_read_from_the_epoch() {
        assert_eq!(group(0), "0");
        assert_eq!(group(1_234_567), "1 234 567");
        assert_eq!(group(114_021_279_984), "114 021 279 984");
        let shown = date(Some(SystemTime::now()));
        assert_eq!(shown.len(), 14, "{shown}");
        assert_eq!(date(None), "");
    }
}
