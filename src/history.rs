//! Far's three histories (history.cpp): the commands run (Alt+F8), the
//! files viewed and edited (Alt+F11), the folders visited (Alt+F12). Each is
//! kept in the profile, the newest last, a repeated name moved to the end
//! (Far's RemoveDups), at most a thousand records; a locked record (Ins)
//! survives clearing. The menu is History::Select's: centered, at most
//! ScrY - 8 rows, a separator with the date before each day, the last
//! record selected.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::profile;
use crate::vmenu::{Entry, Outcome, VMenu};

const LIMIT: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Commands,
    Views,
    Folders,
}

impl Which {
    fn file(self) -> &'static str {
        match self {
            Which::Commands => "history.commands",
            Which::Views => "history.views",
            Which::Folders => "history.folders",
        }
    }

    /// The menu's title (MHistoryTitle, MViewHistoryTitle, MFolderHistoryTitle).
    pub fn title(self) -> &'static str {
        match self {
            Which::Commands => "History",
            Which::Views => "File view and edit history",
            Which::Folders => "Folders history",
        }
    }
}

/// What a record of the view history is (HR_VIEWER, HR_EDITOR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plain,
    View,
    Edit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Seconds since 1970.
    pub time: u64,
    pub locked: bool,
    pub kind: Kind,
    pub name: String,
}

pub struct History {
    which: Which,
    pub records: Vec<Record>,
    file: Option<std::path::PathBuf>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl History {
    pub fn load(which: Which) -> Self {
        Self::load_from(which, profile::path(which.file()))
    }

    pub fn load_from(which: Which, file: std::path::PathBuf) -> Self {
        let records = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let f = profile::fields(line);
                let [time, locked, kind, name] = &f[..] else { return None };
                let kind = match kind.as_str() {
                    "view" => Kind::View,
                    "edit" => Kind::Edit,
                    _ => Kind::Plain,
                };
                Some(Record { time: time.parse().ok()?, locked: locked == "1", kind, name: name.clone() })
            })
            .collect();
        Self { which, records, file: Some(file) }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let key = match self.which {
            Which::Commands => "System.SaveHistory",
            Which::Views => "System.SaveViewHistory",
            Which::Folders => "System.SaveFoldersHistory",
        };
        if !cfg!(test) && !crate::settings::bool(key) {
            return;
        }
        let lines: Vec<String> = self
            .records
            .iter()
            .map(|r| {
                let kind = match r.kind {
                    Kind::View => "view",
                    Kind::Edit => "edit",
                    Kind::Plain => "",
                };
                profile::join(&[&r.time.to_string(), if r.locked { "1" } else { "0" }, kind, &r.name])
            })
            .collect();
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, lines.join("\n") + "\n");
    }

    /// AddToHistory: the name goes to the end (an equal one, the same kind,
    /// is taken out first; a locked one keeps its lock).
    pub fn add(&mut self, name: &str, kind: Kind) {
        if name.trim().is_empty() {
            return;
        }
        let same = |r: &Record| r.kind == kind && if self.which == Which::Commands { r.name == name } else { r.name.eq_ignore_ascii_case(name) };
        let locked = self.records.iter().any(|r| same(r) && r.locked);
        self.records.retain(|r| !same(r));
        self.records.push(Record { time: now(), locked, kind, name: name.to_string() });
        while self.records.len() > LIMIT {
            match self.records.iter().position(|r| !r.locked) {
                Some(oldest) => {
                    self.records.remove(oldest);
                }
                None => break,
            }
        }
        self.save();
    }

    /// The menu of these records.
    pub fn menu(&self) -> HistoryMenu {
        let mut menu = VMenu::with(self.which.title(), vec![]);
        menu.max_height_margin = Some(8);
        let mut list = HistoryMenu { which: self.which, menu, rows: vec![] };
        list.fill(self, None);
        list
    }

    fn remove(&mut self, index: usize) {
        self.records.remove(index);
        self.save();
    }

    fn toggle_lock(&mut self, index: usize) {
        self.records[index].locked = !self.records[index].locked;
        self.save();
    }

    /// Del: everything that is not locked goes (DeleteAllUnlocked).
    pub fn clear(&mut self) {
        self.records.retain(|r| r.locked);
        self.save();
    }

    /// Ctrl+R: the folders and files that are not there any more go.
    fn forget_missing(&mut self) -> bool {
        let before = self.records.len();
        self.records.retain(|r| r.locked || Path::new(&r.name).exists());
        let changed = self.records.len() != before;
        if changed {
            self.save();
        }
        changed
    }
}

/// What the user picked in a history menu (Far's history_return_type).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picked {
    /// Enter (and Shift+Enter, which does it without adding to the history).
    Enter(Record, bool),
    /// Ctrl+Enter: to the command line.
    ToCommandLine(String),
    /// F3 / F4 on a viewed or edited file.
    View(String),
    Edit(String),
}

pub enum MenuOutcome {
    Open,
    Closed,
    Picked(Picked),
    /// Del: ask "All records in the history will be deleted. Continue?".
    AskClear,
}

pub struct HistoryMenu {
    pub which: Which,
    pub menu: VMenu,
    /// The record each menu row shows (None for a date separator).
    rows: Vec<Option<usize>>,
}

impl HistoryMenu {
    /// Fills the menu from the history, selecting the record `select` (else the last).
    fn fill(&mut self, history: &History, select: Option<usize>) {
        let mut entries = Vec::new();
        self.rows.clear();
        let mut last_day = String::new();
        for (index, record) in history.records.iter().enumerate() {
            let day = crate::listing::date_time(Some(UNIX_EPOCH + Duration::from_secs(record.time))).0;
            if day != last_day {
                entries.push(Entry::separator(day.clone()));
                self.rows.push(None);
                last_day = day;
            }
            let prefix = match record.kind {
                Kind::View => "View: ",
                Kind::Edit => "Edit: ",
                Kind::Plain => "",
            };
            let text = crate::dialog::escape_ampersands(&format!("{prefix}{}", record.name));
            entries.push(Entry::new(text).checked(record.locked.then_some('\u{221A}')));
            self.rows.push(Some(index));
        }
        let row = select.and_then(|r| self.rows.iter().position(|row| *row == Some(r))).unwrap_or(entries.len().saturating_sub(1));
        self.menu.set_entries(entries, row);
    }

    fn record(&self) -> Option<usize> {
        self.rows.get(self.menu.selected()).copied().flatten()
    }

    pub fn key(&mut self, key: KeyEvent, history: &mut History) -> MenuOutcome {
        let picked = |history: &History, index: usize| history.records[index].clone();
        match self.menu.key(key) {
            Outcome::Open => MenuOutcome::Open,
            Outcome::Closed => MenuOutcome::Closed,
            Outcome::Chosen(_) => match self.record() {
                Some(index) => MenuOutcome::Picked(Picked::Enter(picked(history, index), false)),
                None => MenuOutcome::Open,
            },
            Outcome::Key(key) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                let current = self.record();
                match (key.code, current) {
                    (KeyCode::Enter, Some(index)) => {
                        let record = picked(history, index);
                        // no keys with two modifiers in farsh (Far's Ctrl+Shift+Enter is left out)
                        MenuOutcome::Picked(if ctrl { Picked::ToCommandLine(record.name) } else { Picked::Enter(record, true) })
                    }
                    (KeyCode::F(3), Some(index)) if self.which == Which::Views => MenuOutcome::Picked(Picked::View(picked(history, index).name)),
                    (KeyCode::F(4), Some(index)) if self.which == Which::Views => MenuOutcome::Picked(Picked::Edit(picked(history, index).name)),
                    (KeyCode::Insert, Some(index)) => {
                        history.toggle_lock(index);
                        self.fill(history, Some(index));
                        MenuOutcome::Open
                    }
                    (KeyCode::Delete, Some(index)) if shift => {
                        if !history.records[index].locked {
                            history.remove(index);
                            self.fill(history, Some(index.min(history.records.len().saturating_sub(1))));
                        }
                        MenuOutcome::Open
                    }
                    (KeyCode::Delete, _) if !history.records.is_empty() => MenuOutcome::AskClear,
                    (KeyCode::Char('r'), _) if ctrl && self.which != Which::Commands => {
                        if history.forget_missing() {
                            self.fill(history, None);
                        }
                        MenuOutcome::Open
                    }
                    _ => MenuOutcome::Open,
                }
            }
        }
    }

    /// After the Del question was answered Clear.
    pub fn refill(&mut self, history: &History) {
        self.fill(history, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_move_to_the_end_locks_survive_clearing() {
        let file = std::env::temp_dir().join(format!("farsh-history-{}", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let mut history = History::load_from(Which::Commands, file.clone());
        history.add("dir", Kind::Plain);
        history.add("ls", Kind::Plain);
        history.add("dir", Kind::Plain);
        assert_eq!(history.records.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["ls", "dir"]);
        let mut menu = history.menu();
        // the date separator, then the two; the last is selected
        assert_eq!(menu.menu.len(), 3);
        assert_eq!(menu.menu.selected(), 2);
        menu.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &mut history);
        menu.key(KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE), &mut history);
        assert!(history.records[0].locked);
        history.clear();
        let reloaded = History::load_from(Which::Commands, file.clone());
        assert_eq!(reloaded.records.len(), 1);
        assert_eq!(reloaded.records[0].name, "ls");
        let MenuOutcome::Picked(Picked::ToCommandLine(name)) = menu.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), &mut history) else { panic!() };
        assert_eq!(name, "ls");
        std::fs::remove_file(file).unwrap();
    }
}
