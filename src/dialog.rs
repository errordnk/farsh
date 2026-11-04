//! Far style dialogs: a grey (or red, for warnings) box with a shadow, a
//! double frame with the title in it, separators, input lines, radio
//! buttons, drop-downs, check boxes and buttons, with hot keys.
//!
//! The geometry is Far Manager 3's own: a box `W` columns wide with the
//! frame 3 columns and 1 row in (message.cpp, dialog.cpp), the controls
//! from column 5 of the box to `W - 6`, a history arrow at `W - 5`, button
//! rows centered as DIF_CENTERGROUP does. Messages (message.cpp) and
//! DialogBuilder dialogs are `MaxLength + 10` wide; the dialogs Far lays
//! out by hand (copy.cpp, mkdir.cpp) have their fixed width.

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
    /// An input line's text before the first key (Far's "unchanged" color).
    unchanged_fg: Color,
    /// A focused drop-down.
    selected_bg: Color,
    selected_fg: Color,
    dim: Color,
}

impl Style {
    fn palette(self) -> Palette {
        let common = Palette {
            bg: palette::bg(palette::LIGHT_GRAY),
            fg: palette::fg(palette::BLACK),
            hot: palette::fg(palette::YELLOW),
            focus_bg: palette::bg(palette::CYAN),
            focus_fg: palette::fg(palette::BLACK),
            edit_bg: palette::bg(palette::CYAN),
            edit_fg: palette::fg(palette::BLACK),
            unchanged_fg: palette::fg(palette::LIGHT_GRAY),
            selected_bg: palette::bg(palette::BLACK),
            selected_fg: palette::fg(palette::WHITE),
            dim: palette::fg(palette::DARK_GRAY),
        };
        match self {
            // Far's COL_DIALOG*: black on light gray, yellow hotkeys, cyan focus and edits
            Style::Normal => common,
            // COL_WARNDIALOG*: white on red, the focus black on light gray
            Style::Warning => Palette {
                bg: palette::bg(palette::RED),
                fg: palette::fg(palette::WHITE),
                focus_bg: palette::bg(palette::LIGHT_GRAY),
                ..common
            },
        }
    }
}

pub struct Button {
    /// With `&` before the hot key, if it has one.
    pub label: &'static str,
    pub id: &'static str,
    pub default: bool,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
}

pub enum Item {
    /// Plain text: `&` is just a character (DIF_SHOWAMPERSAND). A
    /// highlighted line is in the hot color (the names Delete lists).
    Text { text: String, align: Align, highlight: bool },
    /// A text with a hot key (`&`, `&&` for a plain one) that focuses the next control.
    Label(String),
    /// An input line from column `x` of the box to `W - 6`, its label (if
    /// any) at column 5; `history` puts Far's `↓` after it. Until the first
    /// key the text is "unchanged": a typed character replaces all of it.
    Edit { id: &'static str, label: Option<&'static str>, x: u16, text: Vec<char>, cursor: usize, unchanged: bool, enabled: bool, history: bool },
    Radio { id: &'static str, label: &'static str, options: Vec<&'static str>, selected: usize },
    Check { id: &'static str, label: &'static str, checked: bool, enabled: bool },
    /// A drop-down list from column `x` of the box, its label at column 5.
    Combo { id: &'static str, label: &'static str, x: u16, options: Vec<&'static str>, selected: usize, unchanged: bool },
    Separator(Option<&'static str>),
    Buttons(Vec<Button>),
}

/// How the box is sized and placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// Far's Message(): as wide as the content, one row higher than centered.
    Message,
    /// A DialogBuilder dialog: as wide as the content, centered.
    Builder,
    /// A dialog with its own width, centered.
    Fixed(u16),
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
    layout: Layout,
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
    Item::Text { text: text.into(), align: Align::Left, highlight: false }
}

fn centered(text: impl Into<String>) -> Item {
    Item::Text { text: text.into(), align: Align::Center, highlight: false }
}

fn edit(id: &'static str, value: &str) -> Item {
    let text: Vec<char> = value.chars().collect();
    let cursor = text.len();
    let unchanged = !text.is_empty();
    Item::Edit { id, label: None, x: 5, text, cursor, unchanged, enabled: true, history: true }
}

fn button(label: &'static str, id: &'static str, default: bool) -> Button {
    Button { label, id, default, enabled: true }
}

/// `"name"` only when it starts or ends with a space (Far's QuoteOuterSpace).
pub fn quote_outer_space(name: &str) -> String {
    if name.starts_with(' ') || name.ends_with(' ') {
        format!("\"{name}\"")
    } else {
        name.to_string()
    }
}

/// `&` doubled, to show it in a label with a hot key.
pub fn escape_ampersands(text: &str) -> String {
    text.replace('&', "&&")
}

impl Dialog {
    /// F5 and F6: where to, and how (copy.cpp: 76 wide).
    pub fn copy_or_move(kind: Kind, label: String, dest: &str, action: Action) -> Self {
        let moving = kind == Kind::Move;
        let items = vec![
            Item::Label(label),
            edit("dest", dest),
            Item::Separator(None),
            Item::Radio { id: "access", label: "&Access rights:", options: vec!["Defau&lt", "Co&py", "&Inherit"], selected: 0 },
            Item::Separator(None),
            Item::Combo { id: "existing", label: "Already e&xisting files:", x: 29, options: vec!["Ask", "Overwrite", "Skip"], selected: 0, unchanged: true },
            Item::Check { id: "timestamps", label: "Preserve all timestamps", checked: false, enabled: true },
            Item::Check { id: "links", label: "Cop&y contents of symbolic links", checked: false, enabled: true },
            // a move goes to one place only
            Item::Check { id: "multiple", label: "Process &multiple destinations", checked: false, enabled: !moving },
            Item::Separator(None),
            Item::Check { id: "filter", label: "&Use filter", checked: false, enabled: false },
            Item::Separator(None),
            Item::Buttons(vec![
                button(if moving { "&Rename" } else { "&Copy" }, "ok", true),
                Button { label: "&Filter", id: "filter", default: false, enabled: false },
                button("Ca&ncel", "cancel", false),
            ]),
        ];
        Self { title: if moving { "Rename/Move" } else { "Copy" }.into(), style: Style::Normal, layout: Layout::Fixed(76), items, focus: 1, button: 0, action }
    }

    /// F7 (mkdir.cpp: 76 wide). Links are not made yet: the link type is
    /// only "none" and the target stays disabled, as Far shows it then.
    pub fn mkdir() -> Self {
        Self {
            title: "Make folder".into(),
            style: Style::Normal,
            layout: Layout::Fixed(76),
            items: vec![
                Item::Label("Create the &folder:".into()),
                edit("name", ""),
                Item::Separator(None),
                Item::Combo { id: "link", label: "&Link type:", x: 20, options: vec!["none"], selected: 0, unchanged: true },
                Item::Edit { id: "target", label: Some("&Target:"), x: 20, text: vec![], cursor: 0, unchanged: false, enabled: false, history: true },
                Item::Check { id: "multiple", label: "Process &multiple names", checked: false, enabled: true },
                Item::Separator(None),
                Item::Buttons(vec![button("OK", "ok", true), button("Cancel", "cancel", false)]),
            ],
            focus: 1,
            button: 0,
            action: Action::MkDir,
        }
    }

    /// F8 (delete.cpp): for one item the question and its name centered
    /// (and where a link points); for several, the question, then the names
    /// highlighted.
    pub fn delete(question: String, names: Vec<String>, single: bool, action: Action) -> Self {
        let mut items = vec![if single { centered(question) } else { text(question) }];
        if !single {
            items.push(Item::Separator(None));
        }
        for name in names {
            let align = if single { Align::Center } else { Align::Left };
            items.push(Item::Text { text: name, align, highlight: !single });
        }
        items.push(Item::Separator(None));
        items.push(Item::Buttons(vec![button("Delete", "ok", true), button("Cancel", "cancel", false)]));
        let focus = items.len() - 1;
        Self { title: "Delete".into(), style: Style::Warning, layout: Layout::Builder, items, focus, button: 0, action }
    }

    /// A question with Yes and No (Far's Message with no hot keys).
    pub fn confirm(title: &str, line: &str, action: Action) -> Self {
        let items = vec![centered(line), Item::Separator(None), Item::Buttons(vec![button("Yes", "ok", true), button("No", "cancel", false)])];
        Self { title: title.into(), style: Style::Normal, layout: Layout::Message, items, focus: 2, button: 0, action }
    }

    /// An error to read.
    pub fn error(text: &str) -> Self {
        let mut items: Vec<Item> = text.lines().map(centered).collect();
        items.push(Item::Separator(None));
        items.push(Item::Buttons(vec![button("OK", "ok", true)]));
        let focus = items.len() - 1;
        Self { title: "Error".into(), style: Style::Warning, layout: Layout::Message, items, focus, button: 0, action: Action::Info }
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
        Self { title: "Warning".into(), style: Style::Warning, layout: Layout::Fixed(76), items, focus: 8, button: 0, action: Action::Conflict }
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
    /// A box `w` by `h` centered on a screen of `size`; a message sits one row higher.
    fn new(size: (u16, u16), w: u16, h: u16, message: bool) -> Self {
        let (cols, rows) = size;
        let (w, h) = (w.min(cols), h.min(rows));
        let x = (cols - w) / 2;
        let y = if message { rows.saturating_sub(1).saturating_sub(h) / 2 } else { (rows - h) / 2 };
        let (fx_off, fy_off) = (3.min(w / 2), 1.min(h / 2));
        Self { x, y, w, h, fx: x + fx_off, fy: y + fy_off, fw: w - 2 * fx_off, fh: h - 2 * fy_off }
    }

    /// The progress boxes' old sizing: a frame at least `want` wide for `rows` rows.
    fn sized(size: (u16, u16), title: &str, want: u16, rows: u16) -> Self {
        Self::new(size, want.max(title.chars().count() as u16 + 8) + 6, rows + 4, false)
    }

    fn text_x(&self) -> u16 {
        self.x + 5.min(self.w)
    }

    fn text_w(&self) -> u16 {
        self.w.saturating_sub(10)
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
        grid.put(self.fx, self.fy + self.fh.saturating_sub(1), &format!("\u{255A}{line}\u{255D}"), p.fg, p.bg);
        for row in self.fy + 1..(self.fy + self.fh).saturating_sub(1) {
            grid.put(self.fx, row, "\u{2551}", p.fg, p.bg);
            grid.put(self.fx + self.fw.saturating_sub(1), row, "\u{2551}", p.fg, p.bg);
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

/// The length of a label as shown: `&` marks the hot key, `&&` is one `&`.
fn label_len(label: &str) -> u16 {
    shown(label).chars().count() as u16
}

/// A label without its hot key mark.
fn shown(label: &str) -> String {
    let mut out = String::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                out.push('&');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Buttons in a row, each `[ label ]`, one space apart (DIF_CENTERGROUP).
fn buttons_len(buttons: &[Button]) -> u16 {
    let total: u16 = buttons.iter().map(|b| label_len(b.label) + 5).sum();
    total.saturating_sub(1)
}

/// The parts of an item along its row.
fn spans(item: &Item, frame: &Frame) -> Vec<Span> {
    let x = frame.text_x();
    // input lines and drop-downs end at column W - 6
    let field = |at: u16| {
        let start = frame.x + at;
        (start, (frame.x + frame.w).saturating_sub(5).saturating_sub(start))
    };
    match item {
        Item::Text { .. } | Item::Label(_) | Item::Separator(_) => vec![],
        Item::Edit { x: at, .. } | Item::Combo { x: at, .. } => {
            let (start, width) = field(*at);
            vec![(0, start, width)]
        }
        Item::Check { label, .. } => vec![(0, x, 4 + label_len(label))],
        Item::Radio { label, options, .. } => {
            let mut at = x + label_len(label) + 1;
            options
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let width = 4 + label_len(name);
                    let span = (i, at, width);
                    at += width + 1;
                    span
                })
                .collect()
        }
        Item::Buttons(buttons) => {
            let mut at = frame.x + frame.w.saturating_sub(buttons_len(buttons)) / 2;
            buttons
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let width = label_len(b.label) + 4;
                    let span = (i, at, width);
                    at += width + 1;
                    span
                })
                .collect()
        }
    }
}

/// The width the content wants (Far's MaxLength), for the dialogs sized by it.
fn natural_width(item: &Item) -> u16 {
    match item {
        Item::Text { text, .. } => text.chars().count() as u16,
        Item::Label(label) => label_len(label),
        Item::Edit { .. } | Item::Separator(_) => 0,
        Item::Check { label, .. } => 4 + label_len(label),
        Item::Radio { label, options, .. } => label_len(label) + 1 + options.iter().map(|o| 5 + label_len(o)).sum::<u16>(),
        Item::Combo { label, options, .. } => label_len(label) + 3 + options.iter().map(|o| o.chars().count() as u16).max().unwrap_or(0),
        Item::Buttons(buttons) => buttons_len(buttons),
    }
}

/// Draws `label` with the letter after `&` in the hot color (`&&` is a plain `&`).
fn put_label(grid: &mut Grid, x: u16, y: u16, label: &str, fg: Color, bg: Color, hot: Color) {
    let mut col = x;
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        let (c, color) = match c {
            '&' if chars.peek() == Some(&'&') => {
                chars.next();
                ('&', fg)
            }
            '&' => match chars.next() {
                Some(next) => (next, hot),
                None => break,
            },
            c => (c, fg),
        };
        grid.put(col, y, &c.to_string(), color, bg);
        col += 1;
    }
}

fn hot_key(label: &str) -> Option<char> {
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.next() {
                Some('&') => continue,
                Some(next) => return next.to_lowercase().next(),
                None => return None,
            }
        }
    }
    None
}

/// `text` cut in the middle with `…` to `width` (Far's truncate_center).
fn truncate_center(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let left = (width - 1) / 2;
    let right = width - 1 - left;
    let mut out: String = chars[..left].iter().collect();
    out.push('\u{2026}');
    out.extend(&chars[chars.len() - right..]);
    out
}

// ---------------------------------------------------------------- drawing

impl Dialog {
    fn frame(&self, size: (u16, u16)) -> Frame {
        let h = self.items.len() as u16 + 4;
        let content = |cap: u16| {
            let widest = self.items.iter().map(natural_width).max().unwrap_or(0);
            let buttons = self.items.iter().map(|i| if let Item::Buttons(b) = i { buttons_len(b) } else { 0 }).max().unwrap_or(0);
            widest.max(self.title.chars().count() as u16 + 2).min(cap.max(buttons)) + 10
        };
        match self.layout {
            // message.cpp: no wider than the screen less 10
            Layout::Message => Frame::new(size, content(size.0.saturating_sub(11)), h, true),
            // delete.cpp truncates its lines to the screen less 12
            Layout::Builder => Frame::new(size, content(size.0.saturating_sub(12)), h, false),
            Layout::Fixed(w) => Frame::new(size, w, h, false),
        }
    }

    /// Draws the dialog centered on `grid`; returns where the text cursor goes.
    pub fn paint(&self, grid: &mut Grid) -> Option<(u16, u16)> {
        let p = self.style.palette();
        let frame = self.frame((grid.cols, grid.rows));
        frame.draw(grid, &self.title, &p);
        let mut cursor = None;
        let (x, text_w) = (frame.text_x(), frame.text_w() as usize);
        for (i, item) in self.items.iter().enumerate() {
            let y = frame.row_y(i);
            let focused = i == self.focus;
            let spans = spans(item, &frame);
            match item {
                Item::Text { text, align, highlight } => {
                    let text = truncate_center(text, text_w);
                    let len = text.chars().count() as u16;
                    let at = match align {
                        Align::Left => x,
                        Align::Center => frame.x + frame.w.saturating_sub(len) / 2,
                    };
                    grid.put(at, y, &text, if *highlight { p.hot } else { p.fg }, p.bg);
                }
                Item::Label(label) => put_label(grid, x, y, label, p.fg, p.bg, p.hot),
                Item::Separator(title) => frame.separator(grid, y, *title, &p),
                Item::Edit { label, text, cursor: at, unchanged, enabled, history, .. } => {
                    if let Some(label) = label {
                        // Far keeps the label's colors when the line is disabled
                        put_label(grid, x, y, label, p.fg, p.bg, p.hot);
                    }
                    let (fx, w) = (spans[0].1, spans[0].2);
                    let start = (*at + 1).saturating_sub(w as usize);
                    let visible: String = text.iter().skip(start).take(w as usize).collect();
                    let fg = if *enabled { p.edit_fg } else { p.dim };
                    grid.fill_with(fx, y, w, 1, fg, p.edit_bg);
                    grid.put(fx, y, &visible, fg, p.edit_bg);
                    if *unchanged && *enabled {
                        grid.put(fx, y, &visible, p.unchanged_fg, p.edit_bg);
                    }
                    if *history {
                        grid.put(fx + w, y, "\u{2193}", if *enabled { p.fg } else { p.dim }, p.bg);
                    }
                    if focused {
                        cursor = Some((fx + at.saturating_sub(start) as u16, y));
                    }
                }
                Item::Check { label, checked, enabled, .. } => {
                    let (fg, hot) = if *enabled { (p.fg, p.hot) } else { (p.dim, p.dim) };
                    grid.put(spans[0].1, y, &format!("[{}] ", if *checked { 'x' } else { ' ' }), fg, p.bg);
                    put_label(grid, spans[0].1 + 4, y, label, fg, p.bg, hot);
                    if focused {
                        cursor = Some((spans[0].1 + 1, y));
                    }
                }
                Item::Radio { label, options, selected, .. } => {
                    put_label(grid, x, y, label, p.fg, p.bg, p.hot);
                    for ((i, ox, _), name) in spans.iter().zip(options) {
                        let on = i == selected;
                        grid.put(*ox, y, &format!("({}) ", if on { '\u{2022}' } else { ' ' }), p.fg, p.bg);
                        put_label(grid, *ox + 4, y, name, p.fg, p.bg, p.hot);
                        if focused && on {
                            cursor = Some((*ox + 1, y));
                        }
                    }
                }
                Item::Combo { label, options, selected, unchanged, .. } => {
                    put_label(grid, x, y, label, p.fg, p.bg, p.hot);
                    let (fx, w) = (spans[0].1, spans[0].2);
                    let (fg, bg) = if focused {
                        (p.selected_fg, p.selected_bg)
                    } else if *unchanged {
                        (p.unchanged_fg, p.edit_bg)
                    } else {
                        (p.edit_fg, p.edit_bg)
                    };
                    let value = shown(options[*selected]);
                    grid.fill_with(fx, y, w, 1, fg, bg);
                    grid.put(fx, y, &value, fg, bg);
                    grid.put(fx + w, y, "\u{2193}", p.fg, p.bg);
                    if focused {
                        cursor = Some((fx + value.chars().count() as u16, y));
                    }
                }
                Item::Buttons(buttons) => {
                    for ((i, bx, _), b) in spans.iter().zip(buttons) {
                        let on = focused && *i == self.button;
                        let (fg, bg, hot) = if !b.enabled {
                            (p.dim, p.bg, p.dim)
                        } else if on {
                            (p.focus_fg, p.focus_bg, p.hot)
                        } else {
                            (p.fg, p.bg, p.hot)
                        };
                        let (open, close) = if b.default { ('{', '}') } else { ('[', ']') };
                        grid.put(*bx, y, &format!("{open} "), fg, bg);
                        put_label(grid, *bx + 2, y, b.label, fg, bg, hot);
                        grid.put(*bx + 2 + label_len(b.label), y, &format!(" {close}"), fg, bg);
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
            Item::Radio { .. } | Item::Combo { .. } => true,
            Item::Edit { enabled, .. } | Item::Check { enabled, .. } => *enabled,
            Item::Buttons(buttons) => buttons.iter().any(|b| b.enabled),
            Item::Text { .. } | Item::Label(_) | Item::Separator(_) => false,
        }
    }

    fn move_focus(&mut self, forward: bool) {
        // each button is a control of its own: Tab goes through them in turn
        if let Item::Buttons(buttons) = &self.items[self.focus] {
            let next = if forward {
                (self.button + 1..buttons.len()).find(|&i| buttons[i].enabled)
            } else {
                (0..self.button).rev().find(|&i| buttons[i].enabled)
            };
            if let Some(next) = next {
                self.button = next;
                return;
            }
        }
        let count = self.items.len();
        let mut at = self.focus;
        for _ in 0..count {
            at = if forward { (at + 1) % count } else { (at + count - 1) % count };
            if self.focusable(at) {
                self.focus = at;
                if let Item::Buttons(buttons) = &self.items[at] {
                    let mut enabled = buttons.iter().enumerate().filter(|(_, b)| b.enabled).map(|(i, _)| i);
                    self.button = if forward { enabled.next() } else { enabled.last() }.unwrap_or(0);
                }
                return;
            }
        }
    }

    /// Focuses the first control after `index` (what a label's hot key does).
    fn focus_after(&mut self, index: usize) {
        if let Some(next) = (index + 1..self.items.len()).find(|&i| self.focusable(i)) {
            self.focus = next;
            self.button = self.first_enabled_button();
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
            Item::Radio { options, selected, .. } => {
                *selected = (*selected as isize + step).rem_euclid(options.len() as isize) as usize;
            }
            Item::Combo { options, selected, unchanged, .. } => {
                *selected = (*selected as isize + step).rem_euclid(options.len() as isize) as usize;
                *unchanged = false;
            }
            _ => {}
        }
    }

    /// A hot key: focuses the control, toggles a check box, picks a radio
    /// button, presses a button.
    fn hot(&mut self, c: char) -> Option<Outcome> {
        let c = c.to_lowercase().next().unwrap_or(c);
        for index in 0..self.items.len() {
            match &mut self.items[index] {
                Item::Label(label) if hot_key(label) == Some(c) => {
                    self.focus_after(index);
                    return Some(Outcome::Open);
                }
                Item::Check { label, checked, enabled, .. } if *enabled && hot_key(label) == Some(c) => {
                    *checked = !*checked;
                    self.focus = index;
                    return Some(Outcome::Open);
                }
                Item::Edit { label: Some(label), enabled: true, .. } if hot_key(label) == Some(c) => {
                    self.focus = index;
                    return Some(Outcome::Open);
                }
                Item::Combo { label, .. } if hot_key(label) == Some(c) => {
                    self.focus = index;
                    return Some(Outcome::Open);
                }
                Item::Radio { label, options, selected, .. } => {
                    if let Some(option) = options.iter().position(|o| hot_key(o) == Some(c)) {
                        *selected = option;
                        self.focus = index;
                        return Some(Outcome::Open);
                    }
                    if hot_key(label) == Some(c) {
                        self.focus = index;
                        return Some(Outcome::Open);
                    }
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
            if (alt || (!on_edit && !ctrl)) && c != ' ' {
                if let Some(outcome) = self.hot(c) {
                    return outcome;
                }
            }
        }
        if !matches!(key.code, KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down | KeyCode::Enter) {
            self.edit_key(key.code, ctrl, alt);
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
            KeyCode::Char(' ') => match &mut self.items[self.focus] {
                Item::Check { checked, .. } => *checked = !*checked,
                Item::Radio { .. } | Item::Combo { .. } => self.cycle(1),
                Item::Buttons(buttons) => {
                    let id = buttons.get(self.button).filter(|b| b.enabled).map(|b| b.id);
                    if let Some(id) = id {
                        return self.press(id);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Outcome::Open
    }

    /// What a key does to the focused input line (edit.cpp): the first
    /// character typed into unchanged text replaces it, Del at its end
    /// clears it, and any key makes it changed.
    fn edit_key(&mut self, code: KeyCode, ctrl: bool, alt: bool) {
        let Item::Edit { text, cursor, unchanged, .. } = &mut self.items[self.focus] else { return };
        let was_unchanged = std::mem::replace(unchanged, false);
        match code {
            KeyCode::Left => *cursor = cursor.saturating_sub(1),
            KeyCode::Right => *cursor = (*cursor + 1).min(text.len()),
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = text.len(),
            KeyCode::Backspace => {
                if *cursor > 0 {
                    *cursor -= 1;
                    text.remove(*cursor);
                }
            }
            KeyCode::Delete => {
                if was_unchanged && *cursor >= text.len() {
                    text.clear();
                    *cursor = 0;
                } else if *cursor < text.len() {
                    text.remove(*cursor);
                }
            }
            KeyCode::Char('u') if ctrl => {
                text.clear();
                *cursor = 0;
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                if was_unchanged {
                    text.clear();
                    *cursor = 0;
                }
                text.insert(*cursor, c);
                *cursor += 1;
            }
            _ => *unchanged = was_unchanged,
        }
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
                    Item::Edit { text, cursor, unchanged, .. } => {
                        let start = (*cursor + 1).saturating_sub(w as usize);
                        *cursor = (start + (col - x) as usize).min(text.len());
                        *unchanged = false;
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
    let frame = Frame::sized((grid.cols, grid.rows), job.kind.title(), (grid.cols * 3 / 4).clamp(40, 74), rows);
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
        // Far's Yes and No have no hot keys: letters do nothing, Enter is Yes
        assert!(matches!(press(&mut dialog, KeyCode::Char('n')), Outcome::Open));
        let (id, _) = pressed(press(&mut dialog, KeyCode::Enter));
        assert_eq!(id, "ok");
        let mut dialog = Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit);
        press(&mut dialog, KeyCode::Right);
        assert!(matches!(press(&mut dialog, KeyCode::Enter), Outcome::Closed), "focus moved to No");
    }

    #[test]
    fn tab_goes_through_each_button_and_skips_disabled_ones() {
        let mut dialog = copy_dialog();
        for _ in 0..6 {
            press(&mut dialog, KeyCode::Tab);
        }
        assert_eq!((dialog.focus, dialog.button), (dialog.items.len() - 1, 0), "on Copy");
        press(&mut dialog, KeyCode::Tab);
        assert_eq!(dialog.button, 2, "Filter is disabled: on Cancel");
        press(&mut dialog, KeyCode::Tab);
        assert_eq!(dialog.focus, 1, "round to the input line");
        press(&mut dialog, KeyCode::BackTab);
        assert_eq!(dialog.button, 2, "back to Cancel");
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
        Dialog::delete("Do you wish to delete the file?".into(), vec!["a".into()], true, Action::Quit).paint(&mut grid);
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
