//! Far's user menu (usermenu.cpp): F2, and Commands > Edit user menu. The
//! menu is a `FarMenu.ini` in Far's own format: `key:  label` lines, the
//! commands indented under them, a submenu's items between `{` and `}`,
//! `--:` a separator. F2 shows the active folder's file (Local menu), else
//! the main one in farsh's profile (Main menu (User): farsh has no global
//! one in its program folder); BS goes to the parent folder's, Shift+F2
//! between local and main. Edit user menu asks Main or Local first. Ins
//! asks "Insert command" or "Insert menu", F4 edits (EditMenu: 76 by 22, a
//! submenu 76 by 10), Del deletes, Ctrl+Up/Down move, Right/Left go in and
//! out of a submenu, an F-key hot key picks its item. Enter runs the
//! commands (metasymbols substituted, `REM` and `::` lines skipped) in the
//! shell, one after another.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::vmenu::{Entry, Outcome, VMenu};

const LOCAL_NAME: &str = "FarMenu.ini";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub hotkey: String,
    pub label: String,
    pub commands: Vec<String>,
    /// A submenu's items (None: a command).
    pub submenu: Option<Vec<Item>>,
}

/// ParseMenu: from `lines[*at]` to the closing `}` (or the end).
fn parse_items(lines: &[&str], at: &mut usize) -> Vec<Item> {
    let mut menu: Vec<Item> = Vec::new();
    let mut last_open = false;
    while *at < lines.len() {
        let line = lines[*at].trim_end();
        *at += 1;
        if line.is_empty() {
            continue;
        }
        if line.starts_with('{') && last_open {
            let items = parse_items(lines, at);
            if let Some(item) = menu.last_mut() {
                item.submenu = Some(items);
            }
            last_open = false;
            continue;
        }
        // '}' can be a hot key as well
        if line.starts_with('}') && !line[1..].starts_with(':') {
            break;
        }
        if !line.starts_with([' ', '\t']) {
            let Some(mut colon) = line.find(':') else { continue };
            // the hot key ':'
            if line[colon + 1..].starts_with(':') {
                colon += 1;
            }
            menu.push(Item { hotkey: line[..colon].to_string(), label: line[colon + 1..].trim_start().to_string(), commands: vec![], submenu: None });
            last_open = true;
        } else if let Some(item) = menu.last_mut() {
            item.commands.push(line.trim_start().to_string());
        }
    }
    menu
}

pub fn parse(text: &str) -> Vec<Item> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let lines: Vec<&str> = text.lines().collect();
    parse_items(&lines, &mut 0)
}

/// SerializeMenu (CRLF on Windows, as Far's eol::system).
pub fn serialize(menu: &[Item]) -> String {
    let eol = if cfg!(windows) { "\r\n" } else { "\n" };
    let mut out = String::new();
    for item in menu {
        let key = format!("{:<5}", format!("{}:", item.hotkey));
        out.push_str(&format!("{key}{}{eol}", item.label));
        match &item.submenu {
            Some(items) => out.push_str(&format!("{{{eol}{}}}{eol}", serialize(items))),
            None => {
                for command in &item.commands {
                    out.push_str(&format!("{}{command}{eol}", " ".repeat(key.chars().count())));
                }
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Local,
    Main,
}

/// What the menu wants done.
pub enum MenuOutcome {
    Open,
    /// The menu is gone (Esc at the top, Shift+F10, a command run).
    Closed,
    /// Run these command lines (still to be substituted) in the shell.
    Run { commands: Vec<String> },
    /// Ask before: Ins, Del, F4.
    AskInsert,
    AskDelete { label: String, submenu: bool },
    Edit { item: Item },
    /// Alt+F4: edit the file itself.
    EditFile(PathBuf),
}

pub struct UserMenu {
    pub mode: Mode,
    /// The folder of the local menu (where BS goes up from).
    pub dir: PathBuf,
    pub file: PathBuf,
    pub root: Vec<Item>,
    /// The submenus opened, each with the item it was opened from.
    path: Vec<usize>,
    pub menu: VMenu,
    /// The substitution of `!.!` and the like in labels: the current file.
    pub name: String,
    pub modified: bool,
}

impl UserMenu {
    /// F2 (UserMenu(false)): the local menu if the folder has one, else the main one.
    pub fn open(dir: &Path, main_dir: &Path, name: &str) -> Self {
        let local = dir.join(LOCAL_NAME);
        if local.is_file() { Self::load(Mode::Local, dir.to_path_buf(), local, name) } else { Self::load(Mode::Main, dir.to_path_buf(), main_dir.join(LOCAL_NAME), name) }
    }

    pub fn load(mode: Mode, dir: PathBuf, file: PathBuf, name: &str) -> Self {
        let root = std::fs::read(&file).map(|bytes| parse(&String::from_utf8_lossy(&bytes))).unwrap_or_default();
        let mut menu = Self { mode, dir, file, root, path: vec![], menu: VMenu::with("", vec![]), name: name.to_string(), modified: false };
        menu.fill(0);
        menu
    }

    fn title(&self) -> String {
        let mut title = match self.mode {
            Mode::Local => "Local menu".to_string(),
            Mode::Main => "Main menu (User)".to_string(),
        };
        let mut items = &self.root;
        for &i in &self.path {
            title.push_str(&format!(" \u{00BB} {}", crate::vmenu::strip_hot(&items[i].label)));
            items = match &items[i].submenu {
                Some(sub) => sub,
                None => break,
            };
        }
        title
    }

    pub fn items(&self) -> &Vec<Item> {
        let mut items = &self.root;
        for &i in &self.path {
            items = items[i].submenu.as_ref().expect("a submenu");
        }
        items
    }

    pub fn items_mut(&mut self) -> &mut Vec<Item> {
        let mut items = &mut self.root;
        for &i in &self.path {
            items = items[i].submenu.as_mut().expect("a submenu");
        }
        items
    }

    /// FillUserMenu: the hot key padded to 4 (5 with `&`), `&` before it, the label.
    pub fn fill(&mut self, select: usize) {
        let entries = self
            .items()
            .iter()
            .map(|item| {
                if item.hotkey == "--" {
                    return Entry::separator(item.label.clone());
                }
                // PrepareHotKey: `F` and a bad number is no hot key; a lone `&` is doubled
                let mut key = if item.hotkey == "&" { "&&".to_string() } else { item.hotkey.clone() };
                let function = fkey(&key).is_some();
                if !function && key.chars().count() > 1 && key.starts_with(['F', 'f']) {
                    key.clear();
                }
                let width = if key.starts_with('&') { 5 } else { 4 };
                let has_key = !key.is_empty();
                let padded: String = format!("{key:<width$}").chars().take(width).collect();
                let label = crate::shortcuts::expand(&item.label);
                Entry { text: format!("{}{padded}{label}", if has_key && !function { "&" } else { "" }), submenu: item.submenu.is_some(), ..Entry::default() }
            })
            .collect();
        self.menu = VMenu::with(&self.title(), entries);
        self.menu.max_height_margin = Some(4);
        self.menu.bottom = "Ins, Del, F4, Alt+F4, Ctrl+Up, Ctrl+Down".into();
        self.menu.select(select);
    }

    fn selected(&self) -> Option<usize> {
        (self.menu.selected() < self.items().len()).then(|| self.menu.selected())
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        if !self.modified {
            return Ok(());
        }
        self.modified = false;
        let text = serialize(&self.root);
        if text.is_empty() {
            return match std::fs::remove_file(&self.file) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.file, format!("\u{FEFF}{text}"))
    }

    pub fn key(&mut self, key: KeyEvent, main_dir: &Path) -> MenuOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        // an F-key hot key picks its item
        if let KeyCode::F(n) = key.code
            && !ctrl
            && !alt
            && !shift
            && let Some(index) = self.items().iter().position(|item| fkey(&item.hotkey) == Some(n))
        {
            self.menu.select(index);
            return self.choose(index);
        }
        if key.code == KeyCode::Char(' ') {
            return MenuOutcome::Open;
        }
        match self.menu.key(key) {
            Outcome::Open => MenuOutcome::Open,
            Outcome::Closed => self.up(),
            Outcome::Chosen(index) => self.choose(index),
            Outcome::Key(key) => match key.code {
                KeyCode::Right if self.selected().is_some_and(|i| self.items()[i].submenu.is_some()) => self.choose(self.menu.selected()),
                KeyCode::Left if !self.path.is_empty() => self.up(),
                KeyCode::Delete => match self.selected() {
                    Some(i) => {
                        let item = &self.items()[i];
                        MenuOutcome::AskDelete { label: item.label.clone(), submenu: item.submenu.is_some() }
                    }
                    None => MenuOutcome::Open,
                },
                KeyCode::Insert => MenuOutcome::AskInsert,
                KeyCode::F(4) if alt => {
                    let _ = self.save();
                    MenuOutcome::EditFile(self.file.clone())
                }
                KeyCode::F(4) => match self.selected() {
                    Some(i) => MenuOutcome::Edit { item: self.items()[i].clone() },
                    None => MenuOutcome::Open,
                },
                KeyCode::Up | KeyCode::Down if ctrl => {
                    let up = key.code == KeyCode::Up;
                    if let Some(i) = self.selected() {
                        let j = if up { i.checked_sub(1) } else { (i + 1 < self.items().len()).then_some(i + 1) };
                        if let Some(j) = j {
                            self.items_mut().swap(i, j);
                            self.modified = true;
                            self.fill(j);
                        }
                    }
                    MenuOutcome::Open
                }
                KeyCode::F(10) if shift => {
                    let _ = self.save();
                    MenuOutcome::Closed
                }
                KeyCode::F(2) if shift => {
                    let _ = self.save();
                    *self = match self.mode {
                        Mode::Local => Self::load(Mode::Main, self.dir.clone(), main_dir.join(LOCAL_NAME), &self.name),
                        Mode::Main => Self::load(Mode::Local, self.dir.clone(), self.dir.join(LOCAL_NAME), &self.name),
                    };
                    MenuOutcome::Open
                }
                KeyCode::Backspace if self.mode == Mode::Local => {
                    let _ = self.save();
                    *self = match self.dir.parent() {
                        Some(parent) => Self::load(Mode::Local, parent.to_path_buf(), parent.join(LOCAL_NAME), &self.name),
                        None => Self::load(Mode::Main, self.dir.clone(), main_dir.join(LOCAL_NAME), &self.name),
                    };
                    MenuOutcome::Open
                }
                _ => MenuOutcome::Open,
            },
        }
    }

    /// Esc / Left: out of the submenu, or the menu closes.
    fn up(&mut self) -> MenuOutcome {
        match self.path.pop() {
            Some(from) => {
                self.fill(from);
                MenuOutcome::Open
            }
            None => {
                let _ = self.save();
                MenuOutcome::Closed
            }
        }
    }

    fn choose(&mut self, index: usize) -> MenuOutcome {
        let Some(item) = self.items().get(index).cloned() else { return MenuOutcome::Open };
        if item.submenu.is_some() {
            self.path.push(index);
            self.fill(0);
            return MenuOutcome::Open;
        }
        let _ = self.save();
        let commands = item.commands.into_iter().filter(|c| !is_remark(c)).collect();
        MenuOutcome::Run { commands }
    }

    /// Ins: the new item goes before the selected one.
    pub fn insert(&mut self, item: Item) {
        let at = self.menu.selected().min(self.items().len());
        self.items_mut().insert(at, item);
        self.modified = true;
        self.fill(at);
    }

    /// F4: the selected item replaced.
    pub fn replace(&mut self, item: Item) {
        if let Some(i) = self.selected() {
            self.items_mut()[i] = item;
            self.modified = true;
            self.fill(i);
        }
    }

    pub fn delete(&mut self) {
        if let Some(i) = self.selected() {
            self.items_mut().remove(i);
            self.modified = true;
            self.fill(i);
        }
    }
}

const COMMAND_IDS: [&str; 10] = ["cmd0", "cmd1", "cmd2", "cmd3", "cmd4", "cmd5", "cmd6", "cmd7", "cmd8", "cmd9"];

/// EditMenu: 76 by 22 for a command (ten command lines), 76 by 10 for a
/// submenu (its label only).
pub fn edit_dialog(item: &Item, submenu: bool, new: bool) -> crate::dialog::Dialog {
    use crate::dialog::{self, at, span, Item as DItem, Place};
    let height: u16 = if submenu { 10 } else { 22 };
    let mut items_places: Vec<(DItem, Place)> = vec![
        (DItem::Label("&Hot key:".into()), at(5, 2)),
        (dialog::line("hotkey", &item.hotkey, false), span(5, 3, 7)),
        (DItem::Label("&Label:".into()), at(5, 4)),
        (dialog::line("label", &item.label, false), span(5, 5, 70)),
    ];
    if !submenu {
        items_places.push((DItem::Separator(None), at(-1, 6)));
        items_places.push((DItem::Label("&Commands:".into()), at(5, 7)));
        for (i, id) in COMMAND_IDS.iter().enumerate() {
            let value = item.commands.get(i).map(String::as_str).unwrap_or("");
            items_places.push((dialog::line(id, value, false), span(5, 8 + i as u16, 70)));
        }
    }
    items_places.push((DItem::Separator(None), at(-1, height - 4)));
    items_places.push((DItem::Buttons(vec![dialog::button("OK", "ok", true), dialog::button("Cancel", "cancel", false)]), at(-1, height - 3)));
    let (items, places): (Vec<DItem>, Vec<Place>) = items_places.into_iter().unzip();
    let title = if submenu { "Edit submenu label" } else { "Edit user menu" };
    crate::dialog::Dialog::placed(title.into(), (76, height), items, places, 1, Some(edit_hook), crate::dialog::Action::UserMenuEdit { new, submenu })
}

/// EditMenuDlgProc's DN_CLOSE: the hot key and the label must do.
fn edit_hook(dialog: &mut crate::dialog::Dialog, event: crate::dialog::Event) {
    if event != crate::dialog::Event::Closing("ok") {
        return;
    }
    let text = |id: &str| dialog.index_of(id).map(|i| dialog.text_of(i)).unwrap_or_default();
    if let Some(message) = invalid(&text("hotkey"), &text("label")) {
        let field = if message.contains("Label") { "label" } else { "hotkey" };
        dialog.message_over("User menu", &[message], &[("OK", "ok")]);
        dialog.focus_on(field);
    }
}

/// The item the dialog's OK made (commands up to the last one typed).
pub fn from_values(values: &crate::dialog::Values, submenu: Option<Vec<Item>>) -> Item {
    let mut commands: Vec<String> = COMMAND_IDS.iter().map(|id| values.text(id)).collect();
    while commands.last().is_some_and(String::is_empty) {
        commands.pop();
    }
    Item { hotkey: values.text("hotkey"), label: values.text("label"), commands: if submenu.is_some() { vec![] } else { commands }, submenu }
}

/// `F1`..`F24` as a hot key.
fn fkey(key: &str) -> Option<u8> {
    let rest = key.strip_prefix(['F', 'f'])?;
    rest.parse::<u8>().ok().filter(|n| (1..=24).contains(n))
}

/// `REM ...` and `::...` are not run.
fn is_remark(command: &str) -> bool {
    let upper = command.to_uppercase();
    (upper.starts_with("REM") && (upper.len() == 3 || upper[3..].starts_with([' ', '\t']))) || command.starts_with("::")
}

/// What EditMenuDlgProc says of the hot key and label typed, if they will not do.
pub fn invalid(hotkey: &str, label: &str) -> Option<&'static str> {
    if hotkey == "--" {
        return None;
    }
    if label.is_empty() {
        return Some("Invalid format for UserMenu Label");
    }
    (hotkey.chars().count() > 1 && fkey(hotkey).is_none()).then_some("Invalid format for Hot Key")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "A:   &Archive\n     7z a !.7z !.!\nM:   More\n{\nB:   Build\n     cargo build\n--:  tools\n}\nF5:  Copy\n     REM nothing\n     copy !.! x\n";

    #[test]
    fn parses_and_writes_far_format() {
        let menu = parse(SAMPLE);
        assert_eq!(menu.len(), 3);
        assert_eq!(menu[0].commands, ["7z a !.7z !.!"]);
        let sub = menu[1].submenu.as_ref().unwrap();
        assert_eq!(sub[0], Item { hotkey: "B".into(), label: "Build".into(), commands: vec!["cargo build".into()], submenu: None });
        assert_eq!(sub[1].hotkey, "--");
        assert_eq!(menu[2].hotkey, "F5");
        assert_eq!(parse(&serialize(&menu)), menu);
    }

    #[test]
    fn menu_keys() {
        let dir = std::env::temp_dir().join(format!("farsh-usermenu-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOCAL_NAME), SAMPLE).unwrap();
        let mut menu = UserMenu::open(&dir, &dir.join("main"), "x.txt");
        assert_eq!(menu.mode, Mode::Local);
        assert_eq!(menu.menu.entries[0].text, "&A   &Archive");
        assert_eq!(menu.menu.entries[2].text, "F5  Copy");
        let MenuOutcome::Run { commands, .. } = menu.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE), &dir) else { panic!() };
        assert_eq!(commands, ["copy !.! x"]);
        menu.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE), &dir);
        assert_eq!(menu.menu.title, "Local menu \u{00BB} More");
        menu.key(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL), &dir);
        menu.key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL), &dir);
        assert!(menu.modified);
        assert!(matches!(menu.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &dir), MenuOutcome::Open));
        assert!(matches!(menu.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &dir), MenuOutcome::Closed));
        assert_eq!(invalid("XY", "a"), Some("Invalid format for Hot Key"));
        assert_eq!(invalid("F12", "a"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
