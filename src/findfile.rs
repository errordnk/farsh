//! Alt+F7, Far's Find file (findfile.cpp). The question is FindAskDlg (80
//! by 21): the masks, the text to look for, the options, where to look,
//! "Use filter" (the Find file marks of the filters menu); Find, Drive (the
//! drive menu, then the dialog again), Filter (the filters menu for Find
//! file), Cancel. The search runs in the background while the results
//! window (FindDlg: the whole screen) fills: a line a file (date, size,
//! attributes, name, Far's default "D,S,A"), each folder's files after a
//! line with the folder, the folder being searched below, then "Search
//! done". Enter or Go to takes the active panel to the file, F3 / View
//! shows it, F4 edits it, New search asks again, Stop ends the search
//! (then it is Cancel). Not done: hex search, code pages other than UTF-8
//! and UTF-16, fuzzy search, archives, alternate streams, the advanced
//! options; those controls are disabled.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::dialog::{self, Action, Dialog, Event, Item, Place};
use crate::grid::Grid;
use crate::listing::Entry;
use crate::masks::Masks;
use crate::palette;

/// The search area combo, in Far's FINDAREA order.
pub const AREAS: [&str; 7] = [
    "In all non-removable drives",
    "In all local drives",
    "In PATH folders",
    "",
    "From the current folder",
    "The current folder only",
    "Selected folders",
];
const FROM_ROOT: usize = 3;
pub const FROM_CURRENT: usize = 4;
const CURRENT_ONLY: usize = 5;
const SELECTED: usize = 6;

/// What the dialog asked for (kept for the next Alt+F7, as Far's
/// LastFindMask and FindOpt).
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub mask: String,
    pub text: String,
    pub case: bool,
    pub whole_words: bool,
    pub not_containing: bool,
    pub folders: bool,
    pub symlinks: bool,
    pub area: usize,
    pub use_filter: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { mask: "*.*".into(), text: String::new(), case: false, whole_words: false, not_containing: false, folders: true, symlinks: true, area: FROM_CURRENT, use_filter: false }
    }
}

impl Options {
    pub fn from_values(values: &dialog::Values) -> Self {
        Self {
            mask: values.text("mask"),
            text: values.text("text"),
            case: values.checked("case"),
            whole_words: values.checked("words"),
            not_containing: values.checked("not"),
            folders: values.checked("folders"),
            symlinks: values.checked("symlinks"),
            area: values.choice("area"),
            use_filter: values.checked("filter"),
        }
    }
}

/// PrepareDriveNameStr: "From the root of C:", or "From the root folder".
fn from_root_label(dir: &Path) -> &'static str {
    let root = root_of(dir);
    let text = if cfg!(windows) { format!("From the root of {}", root.display().to_string().trim_end_matches(['\\', '/'])) } else { "From the root folder".to_string() };
    Box::leak(text.into_boxed_str())
}

fn root_of(dir: &Path) -> PathBuf {
    dir.ancestors().last().map(Path::to_path_buf).unwrap_or_else(|| dir.to_path_buf())
}

/// FindAskDlg.
pub fn dialog(options: &Options, dir: &Path) -> Dialog {
    let edit = |id: &'static str, value: &str| {
        let text: Vec<char> = value.chars().collect();
        let cursor = text.len();
        Item::Edit { id, label: None, x: 0, text, cursor, unchanged: !value.is_empty(), enabled: true, history: true, secret: false }
    };
    let check = |id: &'static str, label: &'static str, checked: bool, enabled: bool| Item::Check { id, label, checked, enabled };
    let mut areas = AREAS.to_vec();
    areas[FROM_ROOT] = from_root_label(dir);
    // Containing, then &Text and &Hex right of it (ContainingX1..)
    let containing_w = "Containing".len() as i16;
    let text_x = 5 + containing_w + 2;
    let hex_x = text_x + "Text".len() as i16 + 4 + 2;
    let items_places: Vec<(Item, Place)> = vec![
        (Item::Label("A file &mask or several file masks:".into()), dialog::at(5, 2)),
        (edit("mask", &options.mask), dialog::span(5, 3, 74)),
        (Item::Separator(None), dialog::at(-1, 4)),
        (dialog::text("Containing"), dialog::at(5, 5)),
        (edit("text", &options.text), dialog::span(5, 6, 74)),
        (Item::Option { id: "kind", index: 0, label: "&Text", on: true }, dialog::at(text_x, 5)),
        (Item::Option { id: "kind", index: 1, label: "&Hex", on: false }, dialog::at(hex_x, 5)),
        (Item::Label("Using code pa&ge:".into()), dialog::at(5, 7)),
        (Item::Combo { id: "codepage", label: "", x: 5, options: vec!["All standard code pages"], selected: 0, unchanged: true }, dialog::span(5, 8, 74)),
        (Item::Separator(None), dialog::at(-1, 9)),
        (check("case", "&Case sensitive", options.case, true), dialog::at(5, 10)),
        (check("words", "&Whole words", options.whole_words, true), dialog::at(5, 11)),
        (check("fuzzy", "Fu&zzy search", false, false), dialog::at(5, 12)),
        (check("not", "&Not containing", options.not_containing, true), dialog::at(5, 13)),
        (check("archives", "Search in arch&ives", false, false), dialog::at(41, 10)),
        (check("folders", "Search for f&olders", options.folders, true), dialog::at(41, 11)),
        (check("symlinks", "Search in symbolic lin&ks", options.symlinks, true), dialog::at(41, 12)),
        (check("streams", "Search in a&lternate streams", false, false), dialog::at(41, 13)),
        (Item::Separator(None), dialog::at(-1, 14)),
        (Item::Line(['\u{252C}', '\u{2502}', '\u{2534}']), Place { x: 39, y: 9, x2: 0, y2: 14 }),
        (Item::Label("Select search &area:".into()), dialog::at(5, 15)),
        (Item::Combo { id: "area", label: "", x: 5, options: areas, selected: options.area, unchanged: true }, dialog::span(5, 16, 36)),
        (check("filter", "&Use filter", options.use_filter, true), dialog::at(41, 16)),
        (Item::Separator(None), dialog::at(-1, 17)),
        (
            Item::Buttons(vec![
                dialog::button("Find", "ok", true),
                dialog::Button { label: "Dri&ve", id: "drive", default: false, enabled: cfg!(windows), stay: false },
                dialog::Button { label: "Filt&er", id: "filterbtn", default: false, enabled: options.use_filter, stay: false },
                dialog::Button { label: "Advance&d", id: "advanced", default: false, enabled: false, stay: false },
                dialog::button("Cancel", "cancel", false),
            ]),
            dialog::at(-1, 18),
        ),
    ];
    let (items, places): (Vec<Item>, Vec<Place>) = items_places.into_iter().unzip();
    let mut dialog = Dialog::placed("Find file".into(), (80, 21), items, places, 1, Some(hook), Action::FindFile);
    // hex search is not done
    dialog.enable(6, false);
    dialog
}

/// "Use filter" enables Filter (DIF_AUTOMATION).
fn hook(dialog: &mut Dialog, event: Event) {
    match event {
        Event::Toggled("filter") => {
            let on = dialog.index_of("filter").is_some_and(|i| dialog.check_state(i) == 1);
            dialog.enable_button("filterbtn", on);
        }
        Event::Closing("ok") => {
            let mask = dialog.index_of("mask").map(|i| dialog.text_of(i)).unwrap_or_default();
            if !mask.trim().is_empty() && Masks::new(&mask).is_none() {
                dialog.message_over("Warning", &["File-mask string contains errors"], &[("OK", "ok")]);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- the search

/// What the search tells the results window.
enum News {
    Dir(PathBuf),
    Found(PathBuf, Entry),
    Done,
}

/// The folders to search and whether to go into their subfolders.
pub fn roots(area: usize, dir: &Path, selected: Vec<PathBuf>) -> (Vec<PathBuf>, bool) {
    match area {
        // the non-removable drives, without the network ones for "local"
        0 | 1 if cfg!(windows) => {
            let drives: Vec<PathBuf> = crate::drives::list()
                .into_iter()
                .filter(|d| matches!(d.kind.as_str(), "fixed" | "ramdisk") || (area == 0 && d.kind == "network"))
                .map(|d| d.path)
                .collect();
            (if drives.is_empty() { vec![root_of(dir)] } else { drives }, true)
        }
        0 | 1 => (vec![PathBuf::from("/")], true),
        2 => (std::env::var_os("PATH").map(|p| std::env::split_paths(&p).filter(|p| p.is_dir()).collect()).unwrap_or_default(), false),
        FROM_ROOT => (vec![root_of(dir)], true),
        CURRENT_ONLY => (vec![dir.to_path_buf()], false),
        SELECTED => (selected.into_iter().filter(|p| p.is_dir()).collect(), true),
        _ => (vec![dir.to_path_buf()], true),
    }
}

struct Search {
    options: Options,
    masks: Option<Masks>,
    /// The text's bytes in UTF-8 and UTF-16LE, lowered when the case does not matter.
    patterns: Vec<Vec<u8>>,
    stop: Arc<AtomicBool>,
    news: Sender<News>,
}

impl Search {
    fn walk(&self, dir: &Path, recurse: bool) {
        if self.stop.load(Ordering::Relaxed) {
            return;
        }
        let _ = self.news.send(News::Dir(dir.to_path_buf()));
        let entries = crate::listing::read(dir);
        let now = std::time::SystemTime::now();
        for entry in entries {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            let path = dir.join(&entry.name);
            let filtered = !self.options.use_filter || crate::filter::with(|f| !f.enabled(crate::filter::FIND_AREA) || f.shows(&entry, crate::filter::FIND_AREA, now));
            let named = self.masks.as_ref().is_none_or(|m| m.check(&entry.name));
            if filtered && named && (!entry.is_dir || self.options.folders) && self.contains(&path, entry.is_dir) {
                let _ = self.news.send(News::Found(dir.to_path_buf(), entry.clone()));
            }
            let link = entry.reparse.is_some();
            if entry.is_dir && recurse && filtered && (!link || self.options.symlinks) {
                self.walk(&path, true);
            }
        }
    }

    /// The text is in the file (or not, with "Not containing"); a folder has no text.
    fn contains(&self, path: &Path, is_dir: bool) -> bool {
        if self.patterns.is_empty() {
            return true;
        }
        if is_dir {
            return false;
        }
        let found = self.search_file(path).unwrap_or(false);
        found != self.options.not_containing
    }

    fn search_file(&self, path: &Path) -> std::io::Result<bool> {
        let mut file = std::fs::File::open(path)?;
        let longest = self.patterns.iter().map(Vec::len).max().unwrap_or(0);
        let mut window: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; 1 << 20];
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let n = file.read(&mut chunk)?;
            if n == 0 {
                return Ok(false);
            }
            window.extend_from_slice(&chunk[..n]);
            let data: std::borrow::Cow<[u8]> = if self.options.case { (&window[..]).into() } else { window.to_ascii_lowercase().into() };
            if self.patterns.iter().any(|p| find(&data, p, self.options.whole_words, p.len() != self.patterns[0].len())) {
                return Ok(true);
            }
            let keep = window.len().saturating_sub(longest + 2);
            window.drain(..keep);
        }
    }
}

/// `pattern` in `data`, with "Whole words": no letter, digit or `_` right before or after.
fn find(data: &[u8], pattern: &[u8], whole_words: bool, wide: bool) -> bool {
    if pattern.is_empty() || data.len() < pattern.len() {
        return false;
    }
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80;
    let step = if wide { 2 } else { 1 };
    let mut at = 0;
    while at + pattern.len() <= data.len() {
        match data[at..].windows(pattern.len()).position(|w| w == pattern) {
            None => return false,
            Some(offset) => {
                let start = at + offset;
                let end = start + pattern.len();
                let before = start.checked_sub(step).map(|i| data[i]);
                let after = data.get(end).copied();
                if !whole_words || (!before.is_some_and(word) && !after.is_some_and(word)) {
                    return true;
                }
                at = start + 1;
            }
        }
    }
    false
}

// ---------------------------------------------------------------- the results

/// A line of the results: a folder or a file in it.
#[derive(Clone, Debug)]
pub enum Found {
    Folder(PathBuf),
    File(PathBuf, Entry),
}

pub enum Outcome {
    Open,
    Closed,
    /// Go to: the folder, and the file to put the cursor on.
    GoTo(PathBuf, Option<String>),
    View(PathBuf),
    Edit(PathBuf),
    NewSearch,
}

const BUTTONS: [(&str, &str); 5] = [("&New search", "new"), ("&Go to", "goto"), ("&View", "view"), ("&Panel", "panel"), ("&Stop", "stop")];

pub struct Results {
    pub options: Options,
    title: String,
    lines: Vec<Option<Found>>,
    selected: usize,
    top: usize,
    news: Receiver<News>,
    stop: Arc<AtomicBool>,
    searching: Option<PathBuf>,
    done: bool,
    files: usize,
    folders: usize,
    last_dir: Option<PathBuf>,
    /// The list's rows and the buttons' places at the last paint.
    list: (u16, u16, u16, u16),
    buttons: Vec<(u16, u16, u16, &'static str)>,
}

impl Results {
    /// Starts the search.
    pub fn start(options: Options, dir: &Path, selected: Vec<PathBuf>) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let mut patterns = Vec::new();
        if !options.text.is_empty() {
            let text = if options.case { options.text.clone() } else { options.text.to_lowercase() };
            patterns.push(text.as_bytes().to_vec());
            let wide: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let wide = if options.case { wide } else { wide.to_ascii_lowercase() };
            patterns.push(wide);
        }
        let masks = (!options.mask.trim().is_empty()).then(|| Masks::new(&options.mask)).flatten();
        let search = Search { options: options.clone(), masks, patterns, stop: stop.clone(), news: sender };
        let (roots, recurse) = roots(options.area, dir, selected);
        std::thread::spawn(move || {
            for root in roots {
                search.walk(&root, recurse);
            }
            let _ = search.news.send(News::Done);
        });
        let mut title = "Find file".to_string();
        if !options.mask.is_empty() {
            title.push_str(&format!(": {}", options.mask));
        }
        if options.use_filter {
            title.push_str(" (using filter)");
        }
        Self {
            options,
            title,
            lines: vec![],
            selected: 0,
            top: 0,
            news: receiver,
            stop,
            searching: None,
            done: false,
            files: 0,
            folders: 0,
            last_dir: None,
            list: (0, 0, 0, 0),
            buttons: vec![],
        }
    }

    /// Takes what the search found since; whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok(news) = self.news.try_recv() {
            any = true;
            match news {
                News::Dir(dir) => self.searching = Some(dir),
                News::Done => {
                    self.done = true;
                    self.searching = None;
                }
                News::Found(dir, entry) => {
                    if self.last_dir.as_ref() != Some(&dir) {
                        if !self.lines.is_empty() {
                            self.lines.push(None);
                        }
                        self.lines.push(Some(Found::Folder(dir.clone())));
                        self.last_dir = Some(dir.clone());
                    }
                    if entry.is_dir {
                        self.folders += 1;
                    } else {
                        self.files += 1;
                    }
                    self.lines.push(Some(Found::File(dir.join(&entry.name), entry)));
                }
            }
        }
        any
    }

    pub fn busy(&self) -> bool {
        !self.done
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn current(&self) -> Option<&Found> {
        self.lines.get(self.selected).and_then(Option::as_ref)
    }

    fn step(&mut self, step: isize) {
        let n = self.lines.len() as isize;
        let mut at = self.selected as isize;
        loop {
            at += step;
            if at < 0 || at >= n {
                return;
            }
            if self.lines[at as usize].is_some() {
                self.selected = at as usize;
                return;
            }
        }
    }

    fn press(&mut self, id: &str) -> Outcome {
        match (id, self.current().cloned()) {
            ("new", _) => {
                self.stop();
                Outcome::NewSearch
            }
            ("goto", Some(Found::Folder(dir))) => Outcome::GoTo(dir, None),
            ("goto", Some(Found::File(path, _))) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                Outcome::GoTo(path.parent().map(Path::to_path_buf).unwrap_or_default(), name)
            }
            ("view", Some(Found::File(path, entry))) if !entry.is_dir => Outcome::View(path),
            ("stop", _) if !self.done => {
                self.stop();
                Outcome::Open
            }
            ("stop", _) => Outcome::Closed,
            _ => Outcome::Open,
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let rows = self.list.3.saturating_sub(self.list.1) as isize + 1;
        match key.code {
            KeyCode::Esc => {
                self.stop();
                return Outcome::Closed;
            }
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            KeyCode::PageUp => self.step(-rows.max(1)),
            KeyCode::PageDown => self.step(rows.max(1)),
            KeyCode::Home => {
                self.selected = usize::MAX;
                self.step_from_edge(true);
            }
            KeyCode::End => self.step_from_edge(false),
            KeyCode::Enter => return self.press("goto"),
            KeyCode::F(3) => return self.press("view"),
            KeyCode::F(4) => {
                if let Some(Found::File(path, entry)) = self.current()
                    && !entry.is_dir
                {
                    return Outcome::Edit(path.clone());
                }
            }
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) || !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let c = c.to_ascii_lowercase();
                let stop_label = if self.done { 'c' } else { 's' };
                let hit = BUTTONS.iter().find(|(label, id)| if *id == "stop" { c == stop_label } else { hot(label) == Some(c) });
                if let Some((_, id)) = hit {
                    return self.press(id);
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    fn step_from_edge(&mut self, top: bool) {
        let index = if top { self.lines.iter().position(Option::is_some) } else { self.lines.iter().rposition(Option::is_some) };
        if let Some(index) = index {
            self.selected = index;
        }
    }

    pub fn click(&mut self, event: MouseEvent) -> Outcome {
        let (col, row) = (event.column, event.row);
        match event.kind {
            MouseEventKind::ScrollUp => self.step(-1),
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let (x1, y1, x2, y2) = self.list;
                if col >= x1 && col <= x2 && row >= y1 && row <= y2 {
                    let index = self.top + (row - y1) as usize;
                    if self.lines.get(index).is_some_and(Option::is_some) {
                        self.selected = index;
                    }
                } else if let Some((_, _, _, id)) = self.buttons.iter().find(|(x, y, w, _)| row == *y && col >= *x && col < x + w).copied() {
                    return self.press(id);
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    /// FindDlg: the box from (3, 1) to (W - 4, H - 2) of a screen-sized dialog.
    pub fn paint(&mut self, grid: &mut Grid) {
        let bg = palette::bg(palette::LIGHT_GRAY);
        let fg = palette::fg(palette::BLACK);
        let (cols, rows) = (grid.cols, grid.rows);
        let (dw, dh) = (cols.saturating_sub(2), rows.saturating_sub(2));
        let (x0, y0) = (1u16, 1u16);
        grid.fill_with(x0, y0 - 1, dw, dh, fg, bg);
        palette::drop_shadow(grid, x0, y0 - 1, dw, dh);
        let (left, top, right, bottom) = (x0 + 2, y0, x0 + dw - 3, y0 + dh - 3);
        let inner = (right - left - 1) as usize;
        let line = "\u{2550}".repeat(inner);
        grid.put(left, top, &format!("\u{2554}{line}\u{2557}"), fg, bg);
        grid.put(left, bottom, &format!("\u{255A}{line}\u{255D}"), fg, bg);
        for y in top + 1..bottom {
            grid.put(left, y, "\u{2551}", fg, bg);
            grid.put(right, y, "\u{2551}", fg, bg);
        }
        let title: String = format!(" {} ", self.title).chars().take(inner).collect();
        grid.put(left + 1 + (inner.saturating_sub(title.chars().count()) / 2) as u16, top, &title, fg, bg);
        // the list: rows 2 .. H-7 of the dialog
        let (lx1, ly1, lx2, ly2) = (left + 1, top + 1, right - 1, bottom.saturating_sub(5));
        self.list = (lx1, ly1, lx2, ly2);
        let visible = (ly2 + 1 - ly1) as usize;
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + visible {
            self.top = self.selected + 1 - visible;
        }
        let width = (lx2 + 1 - lx1) as usize;
        for row in 0..visible {
            let y = ly1 + row as u16;
            let index = self.top + row;
            match self.lines.get(index) {
                None => {}
                Some(None) => grid.put(lx1, y, &"\u{2500}".repeat(width), fg, bg),
                Some(Some(found)) => {
                    let text = match found {
                        Found::Folder(dir) => {
                            let mut text = dir.display().to_string();
                            if !text.ends_with(std::path::MAIN_SEPARATOR) {
                                text.push(std::path::MAIN_SEPARATOR);
                            }
                            text
                        }
                        Found::File(_, entry) => {
                            let date = crate::listing::date_time(entry.modified).0;
                            format!(
                                " {date:>8}\u{2502}{}\u{2502}{}\u{2502}{}",
                                crate::listing::size_column(entry, 10),
                                crate::listing::attribute_letters(entry.attributes, 6),
                                entry.name
                            )
                        }
                    };
                    let selected = index == self.selected;
                    let (f, b) = if selected { (palette::fg(palette::BLACK), palette::bg(palette::CYAN)) } else { (fg, bg) };
                    let shown: String = text.chars().take(width).collect();
                    grid.fill_with(lx1, y, width as u16, 1, f, b);
                    grid.put(lx1, y, &shown, f, b);
                }
            }
        }
        // the double separator, the status, a separator, the buttons
        let sep2 = "\u{2550}".repeat(inner);
        grid.put(left, bottom - 4, &format!("\u{2560}{sep2}\u{2563}"), fg, bg);
        let status = match &self.searching {
            Some(dir) => {
                let what = if self.options.text.is_empty() { format!("\"{}\"", self.options.mask) } else { format!("\"{}\"", self.options.text) };
                format!("Searching {what} in {}", dir.display())
            }
            None if self.done => format!("Search done. Found files: {}, folders: {}", self.files, self.folders),
            None => String::new(),
        };
        let status: String = crate::listing::truncate_left(&status, inner.saturating_sub(2));
        grid.put(left + 2, bottom - 3, &status, fg, bg);
        let sep = "\u{2500}".repeat(inner);
        grid.put(left, bottom - 2, &format!("\u{255F}{sep}\u{2562}"), fg, bg);
        let found = self.files + self.folders > 0;
        let labels: Vec<(String, &'static str, bool)> = BUTTONS
            .iter()
            .map(|(label, id)| {
                let label = if *id == "stop" && self.done { "&Cancel" } else { label };
                let enabled = match *id {
                    "goto" | "view" => found,
                    "panel" => false,
                    _ => true,
                };
                (format!("[ {label} ]"), *id, enabled)
            })
            .collect();
        let total: usize = labels.iter().map(|(l, _, _)| crate::vmenu::strip_hot(l).chars().count()).sum::<usize>() + labels.len() - 1;
        let mut x = left + 1 + (inner.saturating_sub(total) / 2) as u16;
        self.buttons.clear();
        for (label, id, enabled) in labels {
            let shown = crate::vmenu::strip_hot(&label);
            let w = shown.chars().count() as u16;
            let color = if enabled { fg } else { palette::fg(palette::DARK_GRAY) };
            let hot_at = label.find('&').map(|i| label[..i].chars().count());
            for (i, c) in shown.chars().enumerate() {
                let hot = enabled && hot_at == Some(i);
                grid.put(x + i as u16, bottom - 1, &c.to_string(), if hot { palette::fg(palette::YELLOW) } else { color }, bg);
            }
            if enabled {
                self.buttons.push((x, bottom - 1, w, id));
            }
            x += w + 1;
        }
    }
}

fn hot(label: &str) -> Option<char> {
    let at = label.find('&')?;
    label[at + 1..].chars().next().map(|c| c.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_names_and_text() {
        let dir = std::env::temp_dir().join(format!("farsh-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "hello World").unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), "worldly").unwrap();
        std::fs::write(dir.join("c.rs"), "world").unwrap();
        let options = Options { mask: "*.txt".into(), text: "world".into(), whole_words: true, folders: false, ..Options::default() };
        let mut results = Results::start(options, &dir, vec![]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while results.busy() && std::time::Instant::now() < deadline {
            results.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let names: Vec<String> = results.lines.iter().flatten().filter_map(|f| if let Found::File(p, _) = f { Some(p.file_name().unwrap().to_string_lossy().into_owned()) } else { None }).collect();
        assert_eq!(names, ["a.txt"], "whole words: not \"worldly\"; masks: not c.rs");
        assert_eq!(results.files, 1);
        assert!(find(b"a world b", b"world", true, false));
        assert!(!find(b"worldly", b"world", true, false));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
