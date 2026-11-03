//! Far's F9 menu: a bar over the panels (Left, Files, Commands, Options,
//! Right) with a drop-down under the chosen title. Items farsh does not do
//! yet are shown grey and cannot be chosen.

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

const fn checked(label: &'static str, shortcut: &'static str) -> Item {
    Item { label, shortcut, command: None, checked: true }
}

/// A view mode item, checked when the panel shows it.
fn view(label: &'static str, shortcut: &'static str, right: bool, mode: u8, current: u8) -> Item {
    Item { label, shortcut, command: Some(Command::View { right, mode }), checked: mode == current }
}

const SEPARATOR: Item = item("-", "", None);

struct Title {
    label: &'static str,
    items: Vec<Item>,
}

/// `mode`: the panel's view, 1 Brief .. 4 Wide.
fn panel_menu(title: &'static str, f: u8, mode: u8) -> Title {
    let (on_off, drive) = if f == 1 { ("Ctrl+F1", "Alt+F1") } else { ("Ctrl+F2", "Alt+F2") };
    let right = f == 2;
    Title {
        label: title,
        items: vec![
            view("&Brief", "Ctrl+1", right, 1, mode),
            view("&Medium", "Ctrl+2", right, 2, mode),
            view("&Full", "Ctrl+3", right, 3, mode),
            view("&Wide", "Ctrl+4", right, 4, mode),
            item("Detai&led", "Ctrl+5", None),
            item("&Descriptions", "Ctrl+6", None),
            item("Lon&g descriptions", "Ctrl+7", None),
            item("File own&ers", "Ctrl+8", None),
            item("File lin&ks", "Ctrl+9", None),
            item("&Alternative full", "Ctrl+0", None),
            SEPARATOR,
            item("&Info panel", "Ctrl+L", None),
            item("Quick &view", "Ctrl+Q", None),
            SEPARATOR,
            item("&Sort modes", "Ctrl+F12", Some(Command::SortModes { right })),
            checked("Show long &names", "Ctrl+N"),
            item("Panel &On/Off", on_off, None),
            item("&Re-read", "Ctrl+R", Some(Command::Reread)),
            item("&Change drive", drive, Some(Command::ChangeDrive { right: f == 2 })),
        ],
    }
}

fn titles(modes: [u8; 2]) -> Vec<Title> {
    vec![
        panel_menu("&Left", 1, modes[0]),
        Title {
            label: "&Files",
            items: vec![
                item("&View", "F3", Some(Command::FKey(3))),
                item("&Edit", "F4", Some(Command::FKey(4))),
                item("&Copy", "F5", Some(Command::FKey(5))),
                item("&Rename or move", "F6", Some(Command::FKey(6))),
                item("Lin&k", "Alt+F6", None),
                item("&Make folder", "F7", Some(Command::FKey(7))),
                item("&Delete", "F8", Some(Command::FKey(8))),
                item("Wipe", "Alt+Del", None),
                SEPARATOR,
                item("Add &to archive", "Shift+F1", None),
                item("E&xtract files", "Shift+F2", None),
                item("Arc&hive commands", "Shift+F3", None),
                SEPARATOR,
                item("File &attributes", "Ctrl+A", None),
                item("A&pply command", "Ctrl+G", None),
                item("Descri&be files", "Ctrl+Z", None),
                SEPARATOR,
                item("Select &group", "Gray +", None),
                item("U&nselect group", "Gray -", None),
                item("&Invert selection", "Gray *", Some(Command::InvertSelection)),
                item("Re&store selection", "Ctrl+M", None),
            ],
        },
        Title {
            label: "&Commands",
            items: vec![
                item("&Find file", "Alt+F7", None),
                item("&History", "Alt+F8", None),
                item("&Video mode", "Alt+F9", None),
                item("File vie&w history", "Alt+F11", None),
                item("F&olders history", "Alt+F12", None),
                SEPARATOR,
                item("&Swap panels", "Ctrl+U", Some(Command::SwapPanels)),
                item("&Panels On/Off", "Ctrl+O", Some(Command::TogglePanels)),
                item("&Compare folders", "", None),
                SEPARATOR,
                item("Edit user &menu", "", None),
                item("File &associations", "", None),
                item("Fol&der shortcuts", "", None),
                item("File panel f&ilter", "Ctrl+I", None),
                SEPARATOR,
                item("Pl&ugin commands", "F11", None),
                item("Sc&reens list", "F12", None),
                item("Task &list", "Ctrl+W", None),
                item("Ho&tplug devices list", "", None),
            ],
        },
        Title {
            label: "&Options",
            items: vec![
                item("&System settings", "", None),
                item("&Panel settings", "", None),
                item("&Interface settings", "", None),
                item("Lan&guages", "", None),
                item("Pl&ugins configuration", "", None),
                item("Plugi&ns manager settings", "", None),
                item("Dial&og settings", "", None),
                item("Menu se&ttings", "", None),
                item("Co&mmand line settings", "", None),
                item("&AutoComplete settings", "", None),
                item("Info&Panel settings", "", None),
                item("Groups of file mas&ks", "", None),
                SEPARATOR,
                item("Con&firmations", "", None),
                item("File panel &modes", "", None),
                item("File &descriptions", "", None),
                item("Folder description &files", "", None),
                SEPARATOR,
                item("&Viewer settings", "", None),
                item("&Editor settings", "", None),
                item("Code pages", "", None),
                SEPARATOR,
                item("&Colors", "", None),
                item("Files &highlighting and sort groups", "", None),
                SEPARATOR,
                item("&Save setup", "Shift+F9", None),
            ],
        },
        panel_menu("&Right", 2, modes[1]),
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
}

impl Menu {
    /// Opens on title `current` (0 is Left, 4 is Right); `modes` are the
    /// left and right panels' views (1 Brief .. 4 Wide).
    pub fn new(current: usize, modes: [u8; 2]) -> Self {
        let mut menu = Self { titles: titles(modes), current: 0, item: 0 };
        menu.open(current);
        menu
    }

    /// The open title, to open on it next time.
    pub fn current(&self) -> usize {
        self.current
    }

    fn open(&mut self, index: usize) {
        self.current = index % self.titles.len();
        let items = &self.titles[self.current].items;
        self.item = items.iter().position(|i| i.command.is_some()).unwrap_or(0);
    }

    fn items(&self) -> &[Item] {
        &self.titles[self.current].items
    }

    /// Moves the highlight by `step`, over separators and grey items, round the ends.
    fn step(&mut self, step: isize) {
        let len = self.items().len() as isize;
        let mut at = self.item as isize;
        for _ in 0..len {
            at = (at + step).rem_euclid(len);
            if self.items()[at as usize].command.is_some() {
                self.item = at as usize;
                return;
            }
        }
    }

    /// The first (or last) item that can be chosen.
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

        let (x, y, w, h) = self.drop_down(cols);
        // the shadow, then the box with one more row under the frame
        grid.fill_with(x, y, w, h + 1, TEXT, BG);
        palette::drop_shadow(grid, x, y, w, h + 1);
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
            }
            return Outcome::Open;
        }
        let (x, y, w, h) = self.drop_down(cols);
        if col > x && col + 1 < x + w && row > y && row + 1 < y + h {
            let index = (row - y - 1) as usize;
            if self.items()[index].command.is_some() {
                self.item = index;
            }
            return self.choose(index);
        }
        if col >= x && col < x + w && row >= y && row <= y + h {
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
    fn opens_on_the_first_item_that_can_be_chosen_and_skips_grey_ones() {
        let mut menu = Menu::new(1, [2, 2]);
        assert_eq!(menu.key(key(KeyCode::Enter)), Outcome::Chosen(Command::FKey(3)));
        menu.key(key(KeyCode::Down));
        menu.key(key(KeyCode::Down));
        menu.key(key(KeyCode::Down));
        // Link is grey: Down goes from Rename or move to Make folder
        menu.key(key(KeyCode::Down));
        assert_eq!(menu.key(key(KeyCode::Enter)), Outcome::Chosen(Command::FKey(7)));
        // Up from the top goes round to the last item that can be chosen
        menu.key(key(KeyCode::Home));
        menu.key(key(KeyCode::Up));
        assert_eq!(menu.key(key(KeyCode::Enter)), Outcome::Chosen(Command::InvertSelection));
    }

    #[test]
    fn left_and_right_go_round_the_bar_and_letters_pick_items() {
        let mut menu = Menu::new(0, [2, 2]);
        menu.key(key(KeyCode::Left));
        assert_eq!(menu.current(), 4, "Left from Left is Right");
        menu.key(key(KeyCode::Right));
        menu.key(key(KeyCode::Right));
        menu.key(key(KeyCode::Right));
        assert_eq!(menu.current(), 2);
        assert_eq!(menu.key(key(KeyCode::Char('s'))), Outcome::Chosen(Command::SwapPanels));
        assert_eq!(menu.key(key(KeyCode::Char('f'))), Outcome::Open, "Find file is grey");
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
        let menu = Menu::new(1, [2, 2]);
        menu.paint(&mut grid);
        let top: String = (0..40).map(|c| grid.row(0)[c].text.clone()).collect();
        assert!(top.contains("Left") && top.contains("Files") && top.contains("Commands"), "{top}");
        let first: String = (10..50).map(|c| grid.row(2)[c].text.clone()).collect();
        assert!(first.contains("View") && first.contains("F3"), "{first}");
    }

    #[test]
    fn clicks_open_titles_choose_items_and_close_outside() {
        let mut menu = Menu::new(0, [2, 2]);
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
