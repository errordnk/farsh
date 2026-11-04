//! Far's file filters (filefilter.cpp, filefilterparams.cpp). Ctrl+I is the
//! "Filters menu" of the active panel: the user's filters, a separator,
//! `Folders`, then `Panel file type` for every extension in the panel (and
//! those already marked). `+` / `-` include or exclude, `I` / `X` do it
//! strongly (nothing after them is asked), Space toggles `+`, BS clears,
//! Shift+BS clears all; Ins and F5 (a copy of the item) make a filter, F4
//! edits one (FileFilterConfig: 80 by 15 without the highlighting part),
//! Del deletes, Ctrl+Up/Down move. Closing the menu applies the marks.
//! A file is shown when the first filter it matches includes it, or when it
//! matches none and no filter includes anything (FileInFilterEx); a folder
//! no filter speaks of is shown unless some filter includes folders. The
//! filters and their marks are kept in the profile. Each panel has its own
//! marks (Far's panel_left / panel_right areas), and Find file has a third.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::attributes::Format;
use crate::dialog::{self, Action, Dialog, Event, Item, Place};
use crate::listing::Entry;
use crate::masks::Masks;
use crate::profile;
use crate::vmenu::{Entry as MenuEntry, Outcome, VMenu};

const FILE: &str = "filters";

/// The marks' areas: the left panel, the right one, Find file.
pub const AREAS: usize = 3;
pub const FIND_AREA: usize = 2;

const DIRECTORY: u32 = 0x10;

/// The attributes the filter can ask about (AttributeMapping), in its order.
pub const ATTRIBUTES: [(u32, &str); 21] = [
    (0x1, "&Read only"),
    (0x20, "&Archive"),
    (0x2, "&Hidden"),
    (0x4, "&System"),
    (0x800, "&Compressed"),
    (0x4000, "&Encrypted"),
    (0x2000, "Not &indexed"),
    (DIRECTORY, "&Directory"),
    (0x200, "S&parse"),
    (0x100, "Temporar&y"),
    (0x1000, "O&ffline"),
    (0x400, "Reparse poin&t"),
    (0x10000, "&Virtual"),
    (0x8000, "Inte&grity stream"),
    (0x20000, "No scru&b data"),
    (0x80000, "Pi&nned"),
    (0x100000, "&Unpinned"),
    (0x40000, "Recal&l on open"),
    (0x400000, "Recall &on data access"),
    (0x20000000, "Strictly se&quential"),
    (0x40, "Device"),
];

/// enum_attributes: the letters of the menu's attribute column.
const LETTERS: [(u32, char); 21] = [
    (0x80, 'N'),
    (0x1, 'R'),
    (0x2, 'H'),
    (0x4, 'S'),
    (DIRECTORY, 'D'),
    (0x20, 'A'),
    (0x100, 'T'),
    (0x200, '$'),
    (0x400, 'L'),
    (0x800, 'C'),
    (0x1000, 'O'),
    (0x2000, 'I'),
    (0x4000, 'E'),
    (0x8000, 'V'),
    (0x10000, '?'),
    (0x20000, 'X'),
    (0x80000, 'P'),
    (0x100000, 'U'),
    (0x40000, '\u{203C}'),
    (0x400000, '!'),
    (0x20000000, 'B'),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub title: String,
    pub mask_on: bool,
    pub mask: String,
    pub size_on: bool,
    pub size_from: String,
    pub size_to: String,
    pub date_on: bool,
    /// 0 write, 1 creation, 2 access, 3 change.
    pub date_type: usize,
    pub relative: bool,
    /// The fields as typed: (date or days, time) after (`>=`) and before (`<=`).
    pub after: (String, String),
    pub before: (String, String),
    pub attr_on: bool,
    /// Per ATTRIBUTES: 0 must be off, 1 must be on, 2 either.
    pub attributes: Vec<u8>,
    pub hardlinks: bool,
    /// The mark in each area: `+`, `-`, `I`, `X` or none.
    pub flags: [Option<char>; AREAS],
}

impl Default for Filter {
    /// A new filter (Ins): everything but folders.
    fn default() -> Self {
        let mut attributes = vec![2; ATTRIBUTES.len()];
        attributes[7] = 0;
        Self {
            title: String::new(),
            mask_on: false,
            mask: String::new(),
            size_on: false,
            size_from: String::new(),
            size_to: String::new(),
            date_on: false,
            date_type: 0,
            relative: false,
            after: Default::default(),
            before: Default::default(),
            attr_on: true,
            attributes,
            hardlinks: false,
            flags: [None; AREAS],
        }
    }
}

impl Filter {
    /// A mask filter of the menu's file types (`*.txt`, no folders).
    fn of_mask(mask: &str) -> Self {
        Self { mask_on: true, mask: mask.into(), ..Self::default() }
    }

    fn attr_masks(&self) -> (u32, u32) {
        let mut set = 0;
        let mut clear = 0;
        if self.attr_on {
            for (i, (attribute, _)) in ATTRIBUTES.iter().enumerate() {
                match self.attributes.get(i) {
                    Some(1) => set |= attribute,
                    Some(0) => clear |= attribute,
                    _ => {}
                }
            }
        }
        (set, clear)
    }

    /// FileFilterParams::FileInFilter.
    pub fn matches(&self, entry: &Entry, now: SystemTime) -> bool {
        let attributes = entry.attributes | if entry.is_dir { DIRECTORY } else { 0 };
        let (set, clear) = self.attr_masks();
        if attributes & set != set || attributes & clear != 0 {
            return false;
        }
        if self.size_on {
            if let Some(from) = parse_size(&self.size_from)
                && entry.size < from
            {
                return false;
            }
            if let Some(to) = parse_size(&self.size_to)
                && entry.size > to
            {
                return false;
            }
        }
        if self.date_on {
            let time = match self.date_type {
                1 => entry.created,
                2 => entry.accessed,
                3 => entry.changed,
                _ => entry.modified,
            };
            if let Some(time) = time {
                let (after, before) = self.bounds(now);
                if after.is_some_and(|a| time < a) || before.is_some_and(|b| time > b) {
                    return false;
                }
            }
        }
        if self.mask_on && !Masks::new(&self.mask).is_some_and(|m| m.check(&entry.name)) {
            return false;
        }
        if self.hardlinks && entry.links.unwrap_or(1) < 2 {
            return false;
        }
        true
    }

    /// The date bounds: a relative one is so long before now.
    fn bounds(&self, now: SystemTime) -> (Option<SystemTime>, Option<SystemTime>) {
        let point = |(date, time): &(String, String)| -> Option<SystemTime> {
            if self.relative {
                parse_duration(date, time).filter(|d| !d.is_zero()).and_then(|d| now.checked_sub(d))
            } else {
                Format::system().parse(date, time, None).ok().flatten()
            }
        };
        (point(&self.after), point(&self.before))
    }

    fn to_line(&self) -> String {
        let b = |on: bool| if on { "1" } else { "0" };
        let attributes: String = self.attributes.iter().map(|s| char::from(b'0' + s)).collect();
        let flags: String = self.flags.iter().map(|f| f.unwrap_or('.')).collect();
        let date_type = self.date_type.to_string();
        profile::join(&[
            "F",
            &self.title,
            b(self.mask_on),
            &self.mask,
            b(self.size_on),
            &self.size_from,
            &self.size_to,
            b(self.date_on),
            &date_type,
            b(self.relative),
            &self.after.0,
            &self.after.1,
            &self.before.0,
            &self.before.1,
            b(self.attr_on),
            &attributes,
            b(self.hardlinks),
            &flags,
        ])
    }

    fn from_fields(f: &[String]) -> Option<Self> {
        if f.len() != 18 || f[0] != "F" {
            return None;
        }
        let on = |s: &String| s == "1";
        let mut attributes: Vec<u8> = f[15].bytes().map(|c| c.saturating_sub(b'0').min(2)).collect();
        attributes.resize(ATTRIBUTES.len(), 2);
        Some(Self {
            title: f[1].clone(),
            mask_on: on(&f[2]),
            mask: f[3].clone(),
            size_on: on(&f[4]),
            size_from: f[5].clone(),
            size_to: f[6].clone(),
            date_on: on(&f[7]),
            date_type: f[8].parse().unwrap_or(0),
            relative: on(&f[9]),
            after: (f[10].clone(), f[11].clone()),
            before: (f[12].clone(), f[13].clone()),
            attr_on: on(&f[14]),
            attributes,
            hardlinks: on(&f[16]),
            flags: parse_flags(&f[17]),
        })
    }
}

fn parse_flags(text: &str) -> [Option<char>; AREAS] {
    let mut flags = [None; AREAS];
    for (slot, c) in flags.iter_mut().zip(text.chars()) {
        *slot = matches!(c, '+' | '-' | 'I' | 'X').then_some(c);
    }
    flags
}

/// ConvertFileSizeString: a number, then K, M, G, T, P or E (powers of 1024).
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let value: u64 = digits.parse().ok()?;
    let power = match text[digits.len()..].trim().chars().next().map(|c| c.to_ascii_uppercase()) {
        None | Some('B') => 0,
        Some('K') => 1,
        Some('M') => 2,
        Some('G') => 3,
        Some('T') => 4,
        Some('P') => 5,
        Some('E') => 6,
        _ => return None,
    };
    value.checked_mul(1u64 << (10 * power))
}

/// ParseDuration: days, then `hh:mm:ss` (the fraction ignored).
fn parse_duration(days: &str, time: &str) -> Option<Duration> {
    let days: u64 = days.trim().parse().unwrap_or(0);
    let parts: Vec<u64> = time.split(|c: char| !c.is_ascii_digit()).take(3).map(|p| p.trim().parse().unwrap_or(0)).collect();
    let seconds = parts.first().unwrap_or(&0) * 3600 + parts.get(1).unwrap_or(&0) * 60 + parts.get(2).unwrap_or(&0);
    let total = days * 86400 + seconds;
    (days > 0 || time.chars().any(|c| c.is_ascii_digit())).then(|| Duration::from_secs(total))
}

/// All the filters and their marks.
pub struct Filters {
    pub custom: Vec<Filter>,
    /// The `Folders` item's marks.
    pub folders: [Option<char>; AREAS],
    /// The file type items marked: the mask and its marks.
    pub auto: Vec<(String, [Option<char>; AREAS])>,
    file: Option<PathBuf>,
}

static FILTERS: Mutex<Option<Filters>> = Mutex::new(None);

/// The filters, read from the profile the first time.
pub fn with<R>(f: impl FnOnce(&mut Filters) -> R) -> R {
    let mut guard = FILTERS.lock().unwrap_or_else(|e| e.into_inner());
    let filters = guard.get_or_insert_with(|| Filters::load_from(profile::path(FILE)));
    f(filters)
}

impl Filters {
    pub fn load_from(file: PathBuf) -> Self {
        let mut filters = Self { custom: vec![], folders: [None; AREAS], auto: vec![], file: Some(file.clone()) };
        for line in std::fs::read_to_string(&file).unwrap_or_default().lines() {
            let f = profile::fields(line);
            match f.first().map(String::as_str) {
                Some("F") => filters.custom.extend(Filter::from_fields(&f)),
                Some("D") if f.len() == 2 => filters.folders = parse_flags(&f[1]),
                Some("A") if f.len() == 3 => filters.auto.push((f[1].clone(), parse_flags(&f[2]))),
                _ => {}
            }
        }
        filters
    }

    pub fn save(&self) {
        let Some(file) = &self.file else { return };
        let flags = |f: &[Option<char>; AREAS]| f.iter().map(|c| c.unwrap_or('.')).collect::<String>();
        let mut lines: Vec<String> = self.custom.iter().map(Filter::to_line).collect();
        if self.folders.iter().any(Option::is_some) {
            lines.push(profile::join(&["D", &flags(&self.folders)]));
        }
        for (mask, marks) in &self.auto {
            lines.push(profile::join(&["A", mask, &flags(marks)]));
        }
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = if lines.is_empty() { std::fs::remove_file(file) } else { std::fs::write(file, lines.join("\n") + "\n") };
    }

    /// IsEnabledOnPanel: some filter is marked in `area`.
    pub fn enabled(&self, area: usize) -> bool {
        self.custom.iter().any(|f| f.flags[area].is_some()) || self.folders[area].is_some() || self.auto.iter().any(|(_, f)| f[area].is_some())
    }

    /// multifilter::FileInFilter.
    pub fn shows(&self, entry: &Entry, area: usize, now: SystemTime) -> bool {
        #[derive(PartialEq)]
        enum Action {
            Ignore,
            Include,
            Exclude,
        }
        let mut action = Action::Ignore;
        let (mut has_file_include, mut has_folder_include) = (false, false);
        let is_dir = entry.is_dir;
        let mut process = |filter: &Filter, flag: Option<char>, fast: bool| -> bool {
            let Some(flag) = flag else { return true };
            let strong = matches!(flag, 'I' | 'X');
            if action != Action::Ignore && !strong {
                return true;
            }
            let include = matches!(flag, '+' | 'I');
            if include {
                let (_, clear) = filter.attr_masks();
                let excludes_folders = clear & DIRECTORY != 0;
                if !filter.attr_on || excludes_folders {
                    has_file_include = true;
                }
                if !filter.attr_on || !excludes_folders {
                    has_folder_include = true;
                }
            }
            if fast || !filter.matches(entry, now) {
                return true;
            }
            action = if include { Action::Include } else { Action::Exclude };
            !strong
        };
        let folders = folders_filter();
        let go_on = self.custom.iter().all(|f| process(f, f.flags[area], false)) && process(&folders, self.folders[area], !is_dir);
        if go_on {
            for (mask, flags) in &self.auto {
                if !process(&Filter::of_mask(mask), flags[area], is_dir) {
                    break;
                }
            }
        }
        match action {
            Action::Include => true,
            Action::Exclude => false,
            // a folder no filter speaks of stays unless folders are included somewhere
            Action::Ignore if is_dir && !has_folder_include => true,
            Action::Ignore => !(has_file_include || has_folder_include),
        }
    }
}

/// The `Folders` item's filter (FoldersFilter): folders only.
fn folders_filter() -> Filter {
    Filter { attr_on: true, attributes: ATTRIBUTES.iter().map(|(a, _)| if *a == DIRECTORY { 1 } else { 2 }).collect(), ..Filter::default() }
}

/// The attribute column of the menu: `+ABC… -DEF…` padded to 11.
fn attributes_string(include: u32, exclude: u32) -> String {
    let mut inc = String::new();
    let mut exc = String::new();
    for (attribute, letter) in LETTERS {
        if include & attribute != 0 {
            inc.push(letter);
        } else if exclude & attribute != 0 {
            exc.push(letter);
        }
    }
    let cut = |s: String| if s.chars().count() > 4 { format!("{}\u{2026}", s.chars().take(3).collect::<String>()) } else { s };
    let mut result = String::new();
    if !inc.is_empty() {
        result = format!("+{}", cut(inc));
    }
    if !exc.is_empty() {
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(&format!("-{}", cut(exc)));
    }
    format!("{result:<11}")
}

/// MenuString: `name │ attributes flags │ mask`, with `&N. ` for the hot key items.
fn menu_string(name: &str, hotkey: Option<char>, panel_type: bool, filter: &Filter) -> String {
    let (set, clear) = filter.attr_masks();
    let other: String = [
        if filter.size_on { 'S' } else { '.' },
        if filter.date_on { if filter.relative { 'R' } else { 'D' } } else { '.' },
        if filter.hardlinks { 'H' } else { '.' },
    ]
    .iter()
    .collect();
    let hot = match hotkey {
        Some(h) => format!("&{h}. "),
        None if panel_type => "   ".into(),
        None => String::new(),
    };
    let width = 21 - crate::vmenu::strip_hot(&hot).chars().count();
    let shown: String = crate::vmenu::strip_hot(name).chars().take(width).collect();
    let pad = width - shown.chars().count();
    let name = if hotkey.is_some() || crate::vmenu::strip_hot(name).chars().count() > width { shown } else { name.to_string() };
    let mask = if filter.mask_on { crate::dialog::escape_ampersands(filter.mask.trim_end()) } else { String::new() };
    format!("{hot}{name}{} \u{2502} {} {other} \u{2502} {mask}", " ".repeat(pad), attributes_string(set, clear))
}

/// What a row of the filters menu is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    Custom(usize),
    Separator,
    Folders,
    Auto(String),
}

pub enum MenuOutcome {
    Open,
    /// The menu closed: apply the marks (if anything changed) and read the panel again.
    Closed(bool),
    /// F4 on filter N, Ins/F5 a new one at N from this.
    Edit(usize),
    New(usize, Box<Filter>),
    /// Del on filter N.
    AskDelete(usize),
    /// F4 / Del on an item that is not the user's.
    CustomOnly(&'static str),
}

pub struct FiltersMenu {
    pub area: usize,
    pub menu: VMenu,
    rows: Vec<Row>,
    /// The marks shown, per row.
    marks: Vec<Option<char>>,
    /// The extensions of the panel's files.
    extensions: Vec<String>,
    changed: bool,
}

impl FiltersMenu {
    /// `names`: the files the panel shows (their extensions make the file type items).
    pub fn new(area: usize, names: &[(String, bool)]) -> Self {
        let extensions = names
            .iter()
            .filter(|(name, is_dir)| !is_dir && name != "..")
            .map(|(name, _)| match crate::subst::extension_of(name) {
                "" => "*.".to_string(),
                ext => format!("*.{ext}"),
            })
            .collect();
        let mut menu = VMenu::with("Filters menu", vec![]);
        menu.max_height_margin = Some(6);
        menu.bottom = "+, -, Space, I, X, BS, Shift+BS, Ins, Del, F4, F5, Ctrl+Up, Ctrl+Down".into();
        let mut filters = Self { area, menu, rows: vec![], marks: vec![], extensions, changed: false };
        filters.fill(0);
        filters
    }

    pub fn fill(&mut self, select: usize) {
        let area = self.area;
        let (rows, entries, marks) = with(|filters| {
            let mut rows = Vec::new();
            let mut entries = Vec::new();
            let mut marks = Vec::new();
            for (i, filter) in filters.custom.iter().enumerate() {
                rows.push(Row::Custom(i));
                entries.push(MenuEntry::new(menu_string(&filter.title, None, false, filter)));
                marks.push(filter.flags[area]);
            }
            {
                rows.push(Row::Separator);
                entries.push(MenuEntry::separator(""));
                marks.push(None);
                let folders = folders_filter();
                rows.push(Row::Folders);
                entries.push(MenuEntry::new(menu_string("Folders", Some('0'), false, &folders)));
                marks.push(filters.folders[area]);
                let mut masks: Vec<(String, Option<char>)> = filters.auto.iter().filter(|(_, f)| f[area].is_some()).map(|(m, f)| (m.clone(), f[area])).collect();
                for ext in &self.extensions {
                    if !masks.iter().any(|(m, _)| m.eq_ignore_ascii_case(ext)) {
                        masks.push((ext.clone(), None));
                    }
                }
                masks.sort_by_key(|(m, _)| m.to_lowercase());
                let mut hot = Some('1');
                for (mask, mark) in masks {
                    entries.push(MenuEntry::new(menu_string("Panel file type", hot, true, &Filter::of_mask(&mask))));
                    hot = match hot {
                        Some('9') => Some('A'),
                        Some('Z') | None => None,
                        Some(c) => Some((c as u8 + 1) as char),
                    };
                    rows.push(Row::Auto(mask));
                    marks.push(mark);
                }
            }
            (rows, entries, marks)
        });
        self.rows = rows;
        self.marks = marks;
        let entries = entries.into_iter().zip(&self.marks).map(|(e, m)| e.checked(*m)).collect();
        self.menu.set_entries(entries, select);
    }

    fn set_mark(&mut self, row: usize, mark: Option<char>) {
        self.marks[row] = mark;
        if let Some(entry) = self.menu.entries.get_mut(row) {
            entry.check = mark;
        }
        self.changed = true;
    }

    pub fn key(&mut self, key: KeyEvent) -> MenuOutcome {
        let row = self.menu.selected();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        // the marks first: the items' hot keys do nothing here (DN_LISTHOTKEY)
        let mark = match key.code {
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => match c.to_ascii_uppercase() {
                '+' | '=' => Some(Some('+')),
                '-' => Some(Some('-')),
                'I' => Some(Some('I')),
                'X' => Some(Some('X')),
                ' ' => Some(if self.marks.get(row).copied().flatten().is_some() { None } else { Some('+') }),
                _ => return MenuOutcome::Open,
            },
            KeyCode::Backspace if shift => {
                for r in 0..self.marks.len() {
                    if self.rows[r] != Row::Separator {
                        self.set_mark(r, None);
                    }
                }
                return MenuOutcome::Open;
            }
            KeyCode::Backspace => Some(None),
            _ => None,
        };
        if let Some(new) = mark {
            if self.rows.get(row).is_some_and(|r| *r != Row::Separator) {
                let current = self.marks[row];
                let new = if key.code != KeyCode::Backspace && key.code != KeyCode::Char(' ') && current == new { None } else { new };
                self.set_mark(row, new);
                self.menu.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
            }
            return MenuOutcome::Open;
        }
        match self.menu.key(key) {
            Outcome::Open => MenuOutcome::Open,
            Outcome::Closed | Outcome::Chosen(_) => MenuOutcome::Closed(self.changed),
            Outcome::Key(key) => match (key.code, self.rows.get(row).cloned()) {
                (KeyCode::F(4), Some(Row::Custom(i))) => MenuOutcome::Edit(i),
                (KeyCode::F(4), _) => MenuOutcome::CustomOnly("Only custom filter can be edited"),
                (KeyCode::Delete, Some(Row::Custom(i))) => MenuOutcome::AskDelete(i),
                (KeyCode::Delete, _) => MenuOutcome::CustomOnly("Only custom filter can be deleted"),
                (KeyCode::Insert, _) => MenuOutcome::New(self.custom_pos(row), Box::default()),
                (KeyCode::F(5), Some(which)) => {
                    let template = match which {
                        Row::Custom(i) => with(|f| f.custom[i].clone()),
                        Row::Folders => folders_filter(),
                        Row::Auto(mask) => Filter::of_mask(&mask),
                        Row::Separator => return MenuOutcome::Open,
                    };
                    MenuOutcome::New(self.custom_pos(row), Box::new(Filter { title: String::new(), flags: [None; AREAS], ..template }))
                }
                (KeyCode::Up | KeyCode::Down, Some(Row::Custom(i))) if ctrl => {
                    let j = if key.code == KeyCode::Up { i.checked_sub(1) } else { Some(i + 1) };
                    let moved = with(|f| match j {
                        Some(j) if j < f.custom.len() => {
                            f.custom.swap(i, j);
                            f.save();
                            Some(j)
                        }
                        _ => None,
                    });
                    if let Some(j) = moved {
                        self.fill(j);
                    }
                    MenuOutcome::Open
                }
                _ => MenuOutcome::Open,
            },
        }
    }

    fn custom_pos(&self, row: usize) -> usize {
        let count = self.rows.iter().filter(|r| matches!(r, Row::Custom(_))).count();
        row.min(count)
    }

    /// ProcessSelection: the marks into the filters (a file type item marked
    /// becomes a mask filter, unmarked everywhere it goes).
    pub fn apply(&self) {
        let area = self.area;
        with(|filters| {
            for (row, mark) in self.rows.iter().zip(&self.marks) {
                match row {
                    Row::Custom(i) => filters.custom[*i].flags[area] = *mark,
                    Row::Folders => filters.folders[area] = *mark,
                    Row::Auto(mask) => match filters.auto.iter().position(|(m, _)| m.eq_ignore_ascii_case(mask)) {
                        Some(at) => {
                            filters.auto[at].1[area] = *mark;
                            if filters.auto[at].1.iter().all(Option::is_none) {
                                filters.auto.remove(at);
                            }
                        }
                        None if mark.is_some() => {
                            let mut flags = [None; AREAS];
                            flags[area] = *mark;
                            filters.auto.push((mask.clone(), flags));
                        }
                        None => {}
                    },
                    Row::Separator => {}
                }
            }
            filters.save();
        });
    }
}

// ---------------------------------------------------------------- the dialog

const ID_MASK: usize = 4;
const ID_SIZE: std::ops::RangeInclusive<usize> = 7..=10;
const ID_DATE: std::ops::RangeInclusive<usize> = 12..=23;
const ID_BEFORE_DATE: usize = 15;
const ID_BEFORE_DAYS: usize = 16;
const ID_AFTER_DATE: usize = 19;
const ID_AFTER_DAYS: usize = 20;
const ID_CURRENT: usize = 22;
const ID_ATTR_BUTTON: usize = 27;

/// FileFilterConfig without the highlighting part: 80 by 15.
pub fn dialog(filter: &Filter, index: usize, new: bool) -> Dialog {
    let format = Format::system();
    let (date_mask, _) = format.date_mask();
    let (time_mask, _) = format.time_mask();
    let edit = |id: &'static str, value: &str| {
        let text: Vec<char> = value.chars().collect();
        let cursor = text.len();
        Item::Edit { id, label: None, x: 0, text, cursor, unchanged: !value.is_empty(), enabled: true, history: false, secret: false }
    };
    let date_or_days = |(date, _): &(String, String), relative: bool| if relative { (String::new(), date.clone()) } else { (date.clone(), String::new()) };
    let (before_date, before_days) = date_or_days(&filter.before, filter.relative);
    let (after_date, after_days) = date_or_days(&filter.after, filter.relative);
    let check = |id: &'static str, label: &'static str, checked: bool| Item::Check { id, label, checked, enabled: true };
    let items_places: Vec<(Item, Place)> = vec![
        (Item::Label("&Name:".into()), dialog::at(5, 2)),
        (edit("name", &filter.title), dialog::span(11, 2, 74)),
        (Item::Separator(None), dialog::at(-1, 3)),
        (check("mask_on", "&Mask:", filter.mask_on), dialog::at(5, 4)),
        (edit("mask", &filter.mask), dialog::span(15, 4, 74)),
        (Item::Separator(None), dialog::at(-1, 5)),
        (check("size_on", "Si&ze:", filter.size_on), dialog::at(5, 6)),
        (Item::Label(">&=".into()), dialog::at(7, 7)),
        (edit("size_from", &filter.size_from), dialog::span(10, 7, 20)),
        (Item::Label("&<=".into()), dialog::at(7, 8)),
        (edit("size_to", &filter.size_to), dialog::span(10, 8, 20)),
        (check("date_on", "Date&/Time:", filter.date_on), dialog::at(24, 6)),
        (Item::Combo { id: "date_type", label: "", x: 24, options: vec!["write", "creation", "access", "change"], selected: filter.date_type, unchanged: true }, dialog::span(24, 7, 38)),
        (check("relative", "Relat&ive", filter.relative), dialog::at(24, 8)),
        (dialog::text("<="), dialog::at(43, 7)),
        (dialog::masked("before_date", &date_mask, &before_date), dialog::span(47, 7, 57)),
        (dialog::masked("before_days", "9999", &before_days), dialog::span(47, 7, 57)),
        (dialog::masked("before_time", &time_mask, &filter.before.1), dialog::span(59, 7, 74)),
        (dialog::text(">="), dialog::at(43, 8)),
        (dialog::masked("after_date", &date_mask, &after_date), dialog::span(47, 8, 57)),
        (dialog::masked("after_days", "9999", &after_days), dialog::span(47, 8, 57)),
        (dialog::masked("after_time", &time_mask, &filter.after.1), dialog::span(59, 8, 74)),
        (Item::Buttons(vec![dialog::stay_button("C&urrent", "current", true)]), dialog::span(54, 6, 64)),
        (Item::Buttons(vec![dialog::stay_button("B&lank", "blank", true)]), dialog::span(66, 6, 74)),
        (Item::Separator(None), dialog::at(-1, 9)),
        (Item::Line(['\u{252C}', '\u{2502}', '\u{2534}']), Place { x: 22, y: 5, x2: 0, y2: 9 }),
        (check("attr_on", "Attri&butes", filter.attr_on), dialog::at(5, 10)),
        (Item::Buttons(vec![dialog::stay_button("&...", "attributes", true)]), dialog::span(20, 10, 26)),
        (check("hardlinks", "Has more than one hardlin&k", filter.hardlinks), dialog::at(41, 10)),
        (Item::Separator(None), dialog::at(-1, 11)),
        (Item::Buttons(vec![dialog::button("OK", "ok", true), dialog::stay_button("&Reset", "reset", true), dialog::button("Cancel", "cancel", false)]), dialog::at(-1, 12)),
    ];
    let (items, places): (Vec<Item>, Vec<Place>) = items_places.into_iter().unzip();
    let action = Action::Filter { index, new, attributes: filter.attributes.clone() };
    let mut dialog = Dialog::placed("Filter".into(), (80, 15), items, places, 1, Some(hook), action);
    dialog.enable(ID_MASK, filter.mask_on);
    for i in ID_SIZE {
        dialog.enable(i, filter.size_on);
    }
    for i in ID_DATE {
        dialog.enable(i, filter.date_on);
    }
    dialog.enable(ID_ATTR_BUTTON, filter.attr_on);
    relative_items(&mut dialog, filter.relative);
    dialog
}

/// FilterDlgRelativeDateItemsUpdate: days instead of dates, no Current.
fn relative_items(dialog: &mut Dialog, relative: bool) {
    for (index, on) in [(ID_BEFORE_DATE, !relative), (ID_AFTER_DATE, !relative), (ID_CURRENT, !relative), (ID_BEFORE_DAYS, relative), (ID_AFTER_DAYS, relative)] {
        dialog.show(index, on);
    }
}

fn hook(dialog: &mut Dialog, event: Event) {
    let state = |dialog: &Dialog, id: &str| dialog.index_of(id).is_some_and(|i| dialog.check_state(i) == 1);
    match event {
        Event::Toggled("mask_on") => dialog.enable(ID_MASK, state(dialog, "mask_on")),
        Event::Toggled("size_on") => {
            let on = state(dialog, "size_on");
            for i in ID_SIZE {
                dialog.enable(i, on);
            }
        }
        Event::Toggled("date_on") => {
            let on = state(dialog, "date_on");
            for i in ID_DATE {
                dialog.enable(i, on);
            }
        }
        Event::Toggled("attr_on") => dialog.enable(ID_ATTR_BUTTON, state(dialog, "attr_on")),
        Event::Toggled("relative") => {
            relative_items(dialog, state(dialog, "relative"));
            for id in ["before_date", "before_days", "before_time", "after_date", "after_days", "after_time"] {
                dialog.set_text(id, "", false);
            }
        }
        Event::Button(which @ ("current" | "blank")) => {
            let (date, time) = if which == "current" { Format::system().to_strings(SystemTime::now()) } else { Default::default() };
            let relative = state(dialog, "relative");
            let (before, after) = if relative { ("before_days", "after_days") } else { ("before_date", "after_date") };
            dialog.set_text(after, &date, true);
            dialog.set_text("after_time", &time, true);
            dialog.set_text(before, &date, true);
            dialog.set_text("before_time", &time, true);
            dialog.focus_on(before);
        }
        Event::Button("reset") => {
            if let Some(i) = dialog.index_of("mask") {
                dialog.set_text_at(i, "*");
            }
            for id in ["size_from", "size_to"] {
                dialog.set_text(id, "", false);
            }
            if let Some(i) = dialog.index_of("date_type") {
                dialog.set_list_pos(i, 0);
            }
            for (id, on) in [("mask_on", true), ("size_on", false), ("hardlinks", false), ("date_on", false), ("relative", false), ("attr_on", true)] {
                if let Some(i) = dialog.index_of(id) {
                    dialog.set_check(i, on);
                }
            }
            for id in ["before_date", "before_days", "before_time", "after_date", "after_days", "after_time"] {
                dialog.set_text(id, "", false);
            }
            if let Action::Filter { attributes, .. } = dialog.action_mut() {
                *attributes = Filter::default().attributes;
            }
            relative_items(dialog, false);
            dialog.enable(ID_MASK, true);
            for i in ID_SIZE {
                dialog.enable(i, false);
            }
            for i in ID_DATE {
                dialog.enable(i, false);
            }
            dialog.enable(ID_ATTR_BUTTON, true);
        }
        Event::Button("attributes") => {
            let states = match dialog.action_mut() {
                Action::Filter { attributes, .. } => attributes.clone(),
                _ => return,
            };
            let mut items: Vec<Item> =
                ATTRIBUTES.iter().enumerate().map(|(i, (_, label))| Item::Check3 { id: ATTRIBUTE_IDS[i], label, state: states.get(i).copied().unwrap_or(2), three: true, enabled: true }).collect();
            items.push(Item::Separator(None));
            items.push(Item::Buttons(vec![dialog::button("OK", "ok", true), dialog::button("Cancel", "cancel", false)]));
            dialog.open_sub(Dialog::builder("Attributes", items, 0, None, Action::Info));
        }
        Event::SubClosed(Some(("ok", values))) if values.has_choice(ATTRIBUTE_IDS[0]) => {
            if let Action::Filter { attributes, .. } = dialog.action_mut() {
                *attributes = ATTRIBUTE_IDS.iter().map(|id| values.choice(id) as u8).collect();
            }
        }
        Event::Closing("ok") => {
            if state(dialog, "mask_on") {
                let mask = dialog.index_of("mask").map(|i| dialog.text_of(i)).unwrap_or_default();
                if Masks::new(&mask).is_none() {
                    dialog.message_over("Warning", &["File-mask string contains errors"], &[("OK", "ok")]);
                }
            }
        }
        _ => {}
    }
}

const ATTRIBUTE_IDS: [&str; 21] = [
    "a_ro", "a_arch", "a_hid", "a_sys", "a_comp", "a_enc", "a_noidx", "a_dir", "a_sparse", "a_temp", "a_offl", "a_repar", "a_virt", "a_integ", "a_noscrub", "a_pin",
    "a_unpin", "a_recopen", "a_recdata", "a_seq", "a_dev",
];

/// The filter the dialog's OK made.
pub fn from_values(values: &dialog::Values, attributes: Vec<u8>, flags: [Option<char>; AREAS]) -> Filter {
    let relative = values.checked("relative");
    let date = |date_id: &str, days_id: &str| if relative { values.text(days_id) } else { values.text(date_id) };
    Filter {
        title: values.text("name"),
        mask_on: values.checked("mask_on"),
        mask: values.text("mask"),
        size_on: values.checked("size_on"),
        size_from: values.text("size_from"),
        size_to: values.text("size_to"),
        date_on: values.checked("date_on"),
        date_type: values.choice("date_type"),
        relative,
        after: (date("after_date", "after_days"), values.text("after_time")),
        before: (date("before_date", "before_days"), values.text("before_time")),
        attr_on: values.checked("attr_on"),
        attributes,
        hardlinks: values.checked("hardlinks"),
        flags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, is_dir: bool) -> Entry {
        Entry { name: name.into(), is_dir, attributes: if is_dir { DIRECTORY } else { 0x20 }, ..Entry::default() }
    }

    #[test]
    fn far_include_and_exclude_rules() {
        let mut filters = Filters { custom: vec![], folders: [None; AREAS], auto: vec![], file: None };
        let now = SystemTime::now();
        assert!(filters.shows(&file("a.txt", false), 0, now));
        // `+` on *.txt: only txt files (and the folders) are shown
        filters.auto.push(("*.txt".into(), [Some('+'), None, None]));
        assert!(filters.shows(&file("a.txt", false), 0, now));
        assert!(!filters.shows(&file("a.rs", false), 0, now));
        assert!(filters.shows(&file("src", true), 0, now));
        assert!(filters.shows(&file("a.rs", false), 1, now), "the other panel");
        // `-` on Folders hides them
        filters.folders = [Some('-'), None, None];
        assert!(!filters.shows(&file("src", true), 0, now));
        // a size filter with `X` wins over what follows
        let big = Filter { size_on: true, size_from: "1K".into(), flags: [Some('X'), None, None], ..Filter::default() };
        filters.custom.push(big);
        let mut large = file("b.txt", false);
        large.size = 4096;
        assert!(!filters.shows(&large, 0, now));
        assert_eq!(parse_size("2M"), Some(2 << 20));
        let folders = crate::vmenu::strip_hot(&menu_string("Folders", Some('0'), false, &folders_filter()));
        assert_eq!(folders.find('\u{2502}'), Some(22), "{folders}");
        assert!(folders.starts_with("0. Folders ") && folders.contains("\u{2502} +D          ... \u{2502}"), "{folders}");
    }
}
