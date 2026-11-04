//! Far's F9 menu: a bar over the panels (Left, Files, Commands, Options,
//! Right) with a drop-down under the chosen title. As in Far, F9 shows the
//! bar alone; Enter or Down drops the title's list, which opens on its
//! first item. Items farsh does not do yet are shown grey: the highlight
//! can rest on them (Far's moves over every item), but they do nothing.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::grid::Grid;
use crate::palette;

// Far's COL_MENU*: white on cyan, the bar black on cyan, yellow hotkeys,
// the selection white on black, what farsh does not do yet dark gray
const BG: Color = palette::bg(palette::CYAN);
const TEXT: Color = palette::fg(palette::WHITE);
const HOT: Color = palette::fg(palette::YELLOW);
const DISABLED: Color = palette::fg(palette::DARK_GRAY);
const BAR_TEXT: Color = palette::fg(palette::BLACK);
const SELECTED_BG: Color = palette::bg(palette::BLACK);
const SELECTED_TEXT: Color = palette::fg(palette::WHITE);

/// What a menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// The same as pressing that function key over the panels.
    FKey(u8),
    Reread,
    SwapPanels,
    TogglePanels,
    InvertSelection,
    ChangeDrive { right: bool },
    /// Ctrl+1..Ctrl+4 for the left or right panel.
    View { right: bool, mode: u8 },
    /// Ctrl+F12 for the left or right panel.
    SortModes { right: bool },
    /// Ctrl+A over the active panel.
    Attributes,
    /// Alt+Del over the active panel.
    Wipe,
    /// The Left/Right menu's Info panel, Quick view (no Tree panel: Far's
    /// Tree.TurnOffCompletely hides it by default).
    Info { right: bool },
    QuickView { right: bool },
    /// Show long names (Ctrl+N).
    LongNames { right: bool },
    /// Panel On/Off (Ctrl+F1, Ctrl+F2).
    PanelOnOff { right: bool },
    /// Select group (true) / Unselect group: Far's Gray + and Gray -, Alt++ and Alt+- here.
    SelectGroup(bool),
    /// Ctrl+M.
    RestoreSelection,
    /// What that key asks of the active panel.
    Ask(Asked),
    /// A Commands menu item.
    Do(Cmd),
}

/// The Commands menu's items (each also has its key, if it shows one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    FindFile,
    History,
    VideoMode,
    ViewHistory,
    FoldersHistory,
    CompareFolders,
    EditUserMenu,
    FileAssociations,
    FolderShortcuts,
    Filter,
    PluginCommands,
    ScreensList,
    TaskList,
    Hotplug,
    /// F2 (not in the menu): the user menu itself.
    UserMenu,
}

/// The Files menu items that ask the active panel as their key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// Alt+F6.
    Link,
    /// Shift+F1.
    AddToArchive,
    /// Shift+F2.
    Extract,
    /// Shift+F3.
    ArchiveCommands,
    /// Ctrl+G.
    ApplyCommand,
    /// Ctrl+Z.
    Describe,
}

struct Item {
    /// The label with `&` before the hot letter; `-` is a separator.
    label: &'static str,
    shortcut: &'static str,
    command: Option<Command>,
    checked: bool,
}

const fn item(label: &'static str, shortcut: &'static str, command: Option<Command>) -> Item {
    Item { label, shortcut, command, checked: false }
}

/// A view mode item, checked when the panel shows it.
fn view(label: &'static str, shortcut: &'static str, right: bool, mode: u8, current: u8) -> Item {
    Item { label, shortcut, command: Some(Command::View { right, mode }), checked: mode == current }
}

const SEPARATOR: Item = item("-", "", None);

use crate::panels::Kind;

#[cfg(test)]
const FILES: [(Kind, bool, bool); 2] = [(Kind::Files, true, true), (Kind::Files, true, true)];

struct Title {
    label: &'static str,
    items: Vec<Item>,
}

/// `mode`: the panel's view as Far numbers them, 1 Brief .. 9 File links, 0 Alternative full.
fn panel_menu(title: &'static str, f: u8, mode: u8, state: (Kind, bool, bool)) -> Title {
    // SetLeftRightMenuChecks: the view mode is checked on a file panel only
    let mode = if state.0 == Kind::Files { mode } else { u8::MAX };
    let (on_off, drive) = if f == 1 { ("Ctrl+F1", "Alt+F1") } else { ("Ctrl+F2", "Alt+F2") };
    let right = f == 2;
    Title {
        label: title,
        items: vec![
            view("&Brief", "Ctrl+1", right, 1, mode),
            view("&Medium", "Ctrl+2", right, 2, mode),
            view("&Full", "Ctrl+3", right, 3, mode),
            view("&Wide", "Ctrl+4", right, 4, mode),
            view("Detai&led", "Ctrl+5", right, 5, mode),
            view("&Descriptions", "Ctrl+6", right, 6, mode),
            view("Lon&g descriptions", "Ctrl+7", right, 7, mode),
            view("File own&ers", "Ctrl+8", right, 8, mode),
            view("File lin&ks", "Ctrl+9", right, 9, mode),
            view("&Alternative full", "Ctrl+0", right, 0, mode),
            SEPARATOR,
            Item { label: "&Info panel", shortcut: "Ctrl+L", command: Some(Command::Info { right }), checked: state.0 == Kind::Info },
            Item { label: "Quick &view", shortcut: "Ctrl+Q", command: Some(Command::QuickView { right }), checked: state.0 == Kind::QuickView },
            SEPARATOR,
            item("&Sort modes", "Ctrl+F12", Some(Command::SortModes { right })),
            Item { label: "Show long &names", shortcut: "Ctrl+N", command: Some(Command::LongNames { right }), checked: state.2 },
            item("Panel &On/Off", on_off, Some(Command::PanelOnOff { right })),
            item("&Re-read", "Ctrl+R", Some(Command::Reread)),
            item("&Change drive", drive, Some(Command::ChangeDrive { right: f == 2 })),
        ],
    }
}

fn titles(modes: [u8; 2], states: [(Kind, bool, bool); 2]) -> Vec<Title> {
    vec![
        panel_menu("&Left", 1, modes[0], states[0]),
        Title {
            label: "&Files",
            items: vec![
                item("&View", "F3", Some(Command::FKey(3))),
                item("&Edit", "F4", Some(Command::FKey(4))),
                item("&Copy", "F5", Some(Command::FKey(5))),
                item("&Rename or move", "F6", Some(Command::FKey(6))),
                item("Lin&k", "Alt+F6", Some(Command::Ask(Asked::Link))),
                item("&Make folder", "F7", Some(Command::FKey(7))),
                item("&Delete", "F8", Some(Command::FKey(8))),
                item("&Wipe", "Alt+Del", Some(Command::Wipe)),
                SEPARATOR,
                item("Add &to archive", "Shift+F1", Some(Command::Ask(Asked::AddToArchive))),
                item("E&xtract files", "Shift+F2", Some(Command::Ask(Asked::Extract))),
                item("Arc&hive commands", "Shift+F3", Some(Command::Ask(Asked::ArchiveCommands))),
                SEPARATOR,
                item("File &attributes", "Ctrl+A", Some(Command::Attributes)),
                item("A&pply command", "Ctrl+G", Some(Command::Ask(Asked::ApplyCommand))),
                item("Descri&be files", "Ctrl+Z", Some(Command::Ask(Asked::Describe))),
                SEPARATOR,
                // Far's Gray +, - and * are Alt with the same character here (keyboards without a keypad)
                item("Select &group", "Alt++", Some(Command::SelectGroup(true))),
                item("U&nselect group", "Alt+-", Some(Command::SelectGroup(false))),
                item("&Invert selection", "Alt+*", Some(Command::InvertSelection)),
                item("Re&store selection", "Ctrl+M", Some(Command::RestoreSelection)),
            ],
        },
        Title {
            label: "&Commands",
            items: vec![
                item("&Find file", "Alt+F7", Some(Command::Do(Cmd::FindFile))),
                item("&History", "Alt+F8", Some(Command::Do(Cmd::History))),
                item("&Video mode", "Alt+F9", Some(Command::Do(Cmd::VideoMode))),
                item("File vie&w history", "Alt+F11", Some(Command::Do(Cmd::ViewHistory))),
                item("F&olders history", "Alt+F12", Some(Command::Do(Cmd::FoldersHistory))),
                SEPARATOR,
                item("&Swap panels", "Ctrl+U", Some(Command::SwapPanels)),
                item("&Panels On/Off", "Ctrl+O", Some(Command::TogglePanels)),
                item("&Compare folders", "", Some(Command::Do(Cmd::CompareFolders))),
                SEPARATOR,
                item("Edit user &menu", "", Some(Command::Do(Cmd::EditUserMenu))),
                item("File &associations", "", Some(Command::Do(Cmd::FileAssociations))),
                item("Fol&der shortcuts", "", Some(Command::Do(Cmd::FolderShortcuts))),
                item("File panel f&ilter", "Ctrl+I", Some(Command::Do(Cmd::Filter))),
                SEPARATOR,
                item("Pl&ugin commands", "F11", Some(Command::Do(Cmd::PluginCommands))),
                item("Sc&reens list", "F12", Some(Command::Do(Cmd::ScreensList))),
                item("Task &list", "Ctrl+W", Some(Command::Do(Cmd::TaskList))),
                item("Ho&tplug devices list", "", Some(Command::Do(Cmd::Hotplug))),
            ],
        },
        Title {
            label: "&Options",
            items: vec![
                item("S&ystem settings", "", None),
                item("&Panel settings", "", None),
                item("&Interface settings", "", None),
                item("Lan&guages", "", None),
                item("Pl&ugins configuration", "", None),
                item("Plugins manage&r settings", "", None),
                item("Di&alog settings", "", None),
                item("Menu settings", "", None),
                item("&Command line settings", "", None),
                item("Au&toComplete settings", "", None),
                item("Inf&oPanel settings", "", None),
                item("Groups of file mas&ks", "", None),
                SEPARATOR,
                item("Co&nfirmations", "", None),
                item("File panel &modes", "", None),
                item("File &descriptions", "", None),
                item("&Folder description files", "", None),
                SEPARATOR,
                item("&Viewer settings", "", None),
                item("&Editor settings", "", None),
                item("Code pages", "", None),
                SEPARATOR,
                item("Co&lors", "", None),
                item("Files &highlighting and sort groups", "", None),
                SEPARATOR,
                item("&Save setup", "Shift+F9", None),
            ],
        },
        panel_menu("&Right", 2, modes[1], states[1]),
    ]
}

fn plain(label: &str) -> String {
    label.replace('&', "")
}

fn hot_letter(label: &str) -> Option<char> {
    let at = label.find('&')?;
    label[at + 1..].chars().next().and_then(|c| c.to_lowercase().next())
}

/// Draws `label`, the letter after `&` in `hot`.
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

/// What the menu made of a key or a click.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Closed,
    Chosen(Command),
}

pub struct Menu {
    titles: Vec<Title>,
    /// The open title and the highlighted item in it.
    current: usize,
    item: usize,
    /// Whether the drop-down is shown, or only the bar.
    dropped: bool,
}

impl Menu {
    /// Opens on title `current` (0 is Left, 4 is Right); `modes` are the
    /// left and right panels' views (1 Brief .. 4 Wide).
    /// `states`: each panel's kind, whether it is shown, whether it shows long names.
    pub fn new(current: usize, modes: [u8; 2], states: [(Kind, bool, bool); 2]) -> Self {
        let mut menu = Self { titles: titles(modes, states), current: 0, item: 0, dropped: false };
        menu.open(current);
        menu
    }

    /// The open title, to open on it next time.
    pub fn current(&self) -> usize {
        self.current
    }

    fn open(&mut self, index: usize) {
        self.current = index % self.titles.len();
        self.item = 0;
    }

    fn items(&self) -> &[Item] {
        &self.titles[self.current].items
    }

    /// Moves the highlight by `step`, over separators, round the ends.
    fn step(&mut self, step: isize) {
        let len = self.items().len() as isize;
        let mut at = self.item as isize;
        for _ in 0..len {
            at = (at + step).rem_euclid(len);
            if self.items()[at as usize].label != "-" {
                self.item = at as usize;
                return;
            }
        }
    }

    /// The first (or last) item.
    fn edge(&mut self, last: bool) {
        self.item = if last { 0 } else { self.items().len() - 1 };
        self.step(if last { -1 } else { 1 });
    }

    fn choose(&self, index: usize) -> Outcome {
        match self.items().get(index).and_then(|i| i.command) {
            Some(command) => Outcome::Chosen(command),
            None => Outcome::Open,
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if !self.dropped {
            return self.bar_key(key);
        }
        match key.code {
            KeyCode::Esc | KeyCode::F(9) | KeyCode::F(10) => return Outcome::Closed,
            KeyCode::Left => self.open(self.current + self.titles.len() - 1),
            KeyCode::Right => self.open(self.current + 1),
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            KeyCode::Home | KeyCode::PageUp => self.edge(false),
            KeyCode::End | KeyCode::PageDown => self.edge(true),
            KeyCode::Enter => return self.choose(self.item),
            KeyCode::Char(c) => {
                let c = c.to_lowercase().next().unwrap_or(c);
                // Alt+letter opens a title, a letter picks an item
                if key.modifiers.contains(KeyModifiers::ALT) {
                    if let Some(index) = self.titles.iter().position(|t| hot_letter(t.label) == Some(c)) {
                        self.open(index);
                    }
                } else if let Some(index) =
                    self.items().iter().position(|i| i.command.is_some() && hot_letter(i.label) == Some(c))
                {
                    self.item = index;
                    return self.choose(index);
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    /// A key while only the bar shows: Left and Right move along it, Enter
    /// and Down drop the list, a title's letter drops its list.
    fn bar_key(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Esc | KeyCode::F(9) | KeyCode::F(10) => return Outcome::Closed,
            KeyCode::Left => self.open(self.current + self.titles.len() - 1),
            KeyCode::Right => self.open(self.current + 1),
            KeyCode::Home => self.open(0),
            KeyCode::End => self.open(self.titles.len() - 1),
            KeyCode::Enter | KeyCode::Down | KeyCode::Up => self.dropped = true,
            KeyCode::Char(c) => {
                let c = c.to_lowercase().next().unwrap_or(c);
                if let Some(index) = self.titles.iter().position(|t| hot_letter(t.label) == Some(c)) {
                    self.open(index);
                    self.dropped = true;
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    /// The bar titles: (column, width) of each, as drawn.
    fn title_spans(&self) -> Vec<(u16, u16)> {
        let mut col = 2;
        self.titles
            .iter()
            .map(|t| {
                let width = plain(t.label).chars().count() as u16 + 4;
                let span = (col, width);
                col += width;
                span
            })
            .collect()
    }

    /// The drop-down's frame: column, row, width, height.
    fn drop_down(&self, cols: u16) -> (u16, u16, u16, u16) {
        let items = self.items();
        let label = items.iter().map(|i| plain(i.label).chars().count()).max().unwrap_or(0) as u16;
        let shortcut = items.iter().map(|i| i.shortcut.chars().count()).max().unwrap_or(0) as u16;
        // ║ √ label  shortcut ║
        let width = 1 + 2 + label + if shortcut > 0 { 2 + shortcut } else { 0 } + 2;
        let height = items.len() as u16 + 2;
        let col = self.title_spans()[self.current].0.min(cols.saturating_sub(width + 2));
        (col, 1, width, height)
    }

    pub fn paint(&self, grid: &mut Grid) {
        let cols = grid.cols;
        grid.fill_with(0, 0, cols, 1, BAR_TEXT, BG);
        for (index, (title, (col, width))) in self.titles.iter().zip(self.title_spans()).enumerate() {
            if index == self.current {
                grid.fill_with(col, 0, width, 1, SELECTED_TEXT, SELECTED_BG);
                put_label(grid, col + 2, 0, title.label, SELECTED_TEXT, SELECTED_BG, HOT);
            } else {
                put_label(grid, col + 2, 0, title.label, BAR_TEXT, BG, HOT);
            }
        }

        if !self.dropped {
            return;
        }
        let (x, y, w, h) = self.drop_down(cols);
        // the shadow, then the frame (no margin: it hangs from the bar)
        grid.fill_with(x, y, w, h, TEXT, BG);
        palette::drop_shadow(grid, x, y, w, h);
        let line = "\u{2550}".repeat(w.saturating_sub(2) as usize);
        grid.put(x, y, &format!("\u{2554}{line}\u{2557}"), TEXT, BG);
        grid.put(x, y + h - 1, &format!("\u{255A}{line}\u{255D}"), TEXT, BG);
        let shortcut_col = x + w - 2 - self.items().iter().map(|i| i.shortcut.chars().count()).max().unwrap_or(0) as u16;
        for (index, item) in self.items().iter().enumerate() {
            let row = y + 1 + index as u16;
            if item.label == "-" {
                let thin = "\u{2500}".repeat(w.saturating_sub(2) as usize);
                grid.put(x, row, &format!("\u{255F}{thin}\u{2562}"), TEXT, BG);
                continue;
            }
            grid.put(x, row, "\u{2551}", TEXT, BG);
            grid.put(x + w - 1, row, "\u{2551}", TEXT, BG);
            let selected = index == self.item;
            let (fg, bg, hot) = match (selected, item.command.is_some()) {
                (true, _) => (SELECTED_TEXT, SELECTED_BG, HOT),
                (false, true) => (TEXT, BG, HOT),
                (false, false) => (DISABLED, BG, DISABLED),
            };
            if selected {
                grid.fill_with(x + 1, row, w - 2, 1, fg, bg);
            }
            if item.checked {
                grid.put(x + 1, row, "\u{221A}", fg, bg);
            }
            put_label(grid, x + 3, row, item.label, fg, bg, hot);
            grid.put(shortcut_col, row, item.shortcut, fg, bg);
        }
    }

    /// A left click: a title opens, an item is chosen, outside closes.
    pub fn click(&mut self, event: MouseEvent, cols: u16) -> Outcome {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Outcome::Open;
        }
        let (col, row) = (event.column, event.row);
        if row == 0 {
            if let Some(index) = self.title_spans().iter().position(|&(x, w)| col >= x && col < x + w) {
                self.open(index);
                self.dropped = true;
            }
            return Outcome::Open;
        }
        if !self.dropped {
            return Outcome::Closed;
        }
        let (x, y, w, h) = self.drop_down(cols);
        if col > x && col + 1 < x + w && row > y && row + 1 < y + h {
            let index = (row - y - 1) as usize;
            if self.items()[index].label != "-" {
                self.item = index;
            }
            return self.choose(index);
        }
        if col >= x && col < x + w && row >= y && row < y + h {
            return Outcome::Open;
        }
        Outcome::Closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn f9_shows_the_bar_and_enter_drops_the_list_on_its_first_item() {
        let mut menu = Menu::new(1, [2, 2], FILES);
        assert!(!menu.dropped, "the bar alone");
        menu.key(key(KeyCode::Right));
        assert_eq!(menu.current(), 2);
        assert!(!menu.dropped, "Right moves along the bar");
        menu.key(key(KeyCode::Left));
        assert_eq!(menu.key(key(KeyCode::Enter)), Outcome::Open);
        assert!(menu.dropped);
        assert_eq!(menu.key(key(KeyCode::Enter)), Outcome::Chosen(Command::FKey(3)));
        for _ in 0..4 {
            menu.key(key(KeyCode::Down));
        }
        assert_eq!(menu.key(key(KeyCode::Enter)), Outcome::Chosen(Command::Ask(Asked::Link)));
        menu.key(key(KeyCode::Down));
        assert_eq!(menu.key(key(KeyCode::Enter)), Outcome::Chosen(Command::FKey(7)));
        // a panel menu opens on Brief, not on the checked mode
        let mut menu = Menu::new(0, [2, 2], FILES);
        menu.key(key(KeyCode::Down));
        assert_eq!(menu.key(key(KeyCode::Enter)), Outcome::Chosen(Command::View { right: false, mode: 1 }));
    }

    #[test]
    fn left_and_right_go_round_the_bar_and_letters_pick_items() {
        let mut menu = Menu::new(0, [2, 2], FILES);
        menu.dropped = true;
        menu.key(key(KeyCode::Left));
        assert_eq!(menu.current(), 4, "Left from Left is Right");
        menu.key(key(KeyCode::Right));
        menu.key(key(KeyCode::Right));
        menu.key(key(KeyCode::Right));
        assert_eq!(menu.current(), 2);
        assert_eq!(menu.key(key(KeyCode::Char('s'))), Outcome::Chosen(Command::SwapPanels));
        assert_eq!(menu.key(key(KeyCode::Char('f'))), Outcome::Chosen(Command::Do(Cmd::FindFile)));
        assert_eq!(
            menu.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT)),
            Outcome::Open,
            "Alt+F opens Files"
        );
        assert_eq!(menu.current(), 1);
        assert_eq!(menu.key(key(KeyCode::Esc)), Outcome::Closed);
    }

    #[test]
    fn painting_puts_the_bar_on_top_and_the_drop_down_under_its_title() {
        let mut grid = Grid::new(80, 30);
        let mut menu = Menu::new(1, [2, 2], FILES);
        menu.dropped = true;
        menu.paint(&mut grid);
        let top: String = (0..40).map(|c| grid.row(0)[c].text.clone()).collect();
        assert!(top.contains("Left") && top.contains("Files") && top.contains("Commands"), "{top}");
        let first: String = (10..50).map(|c| grid.row(2)[c].text.clone()).collect();
        assert!(first.contains("View") && first.contains("F3"), "{first}");
    }

    #[test]
    fn clicks_open_titles_choose_items_and_close_outside() {
        let mut menu = Menu::new(0, [2, 2], FILES);
        let click = |col, row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let files = menu.title_spans()[1].0;
        assert_eq!(menu.click(click(files + 1, 0), 80), Outcome::Open);
        assert_eq!(menu.current(), 1);
        // the third row of the drop-down is Copy
        assert_eq!(menu.click(click(files + 4, 4), 80), Outcome::Chosen(Command::FKey(5)));
        assert_eq!(menu.click(click(79, 29), 80), Outcome::Closed);
    }
}
