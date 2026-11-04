//! The Commands menu (and its keys) over the panels: the three histories,
//! Video mode, Compare folders, the user menu, file associations, folder
//! shortcuts, the filters, Find file, the plugin commands, the screens,
//! the task list and the hotplug devices. Each list is a Far menu
//! (`crate::vmenu`); a dialog it opens (an edit, a question) is drawn over
//! it, and the list comes back when the dialog closes (`App::under`).

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use crate::dialog::{Action, Dialog, Values};
use crate::history::{self, History, Which};
use crate::menu::Cmd;
use crate::panels::{Key, Request};
use crate::vmenu::{self, Entry, Outcome, VMenu};
use crate::{assoc, filter, findfile, hotplug, profile, shortcuts, tasklist, usermenu};

use super::{App, Editing, Overlay};

/// A list on the screen, with what it lists.
pub enum List {
    History(history::HistoryMenu),
    Shortcuts(VMenu),
    Assocs(VMenu),
    /// Several associations for a file: the commands to choose from.
    SelectAssoc(VMenu, Vec<String>),
    UserMenu(Box<usermenu::UserMenu>),
    Filters(filter::FiltersMenu),
    Plugins(VMenu),
    ArcLite(VMenu),
    /// The screens, and the viewer F12 was pressed in (to go back to on Esc).
    Screens(VMenu, Option<usize>),
    Tasks(VMenu, Vec<tasklist::Task>, bool),
    Hotplug(VMenu, Vec<hotplug::Device>),
    /// Options > Languages: the main language's menu, then the help's.
    Languages(VMenu, bool),
    /// Options > Groups of file masks: the groups listed, whether F7 filtered them.
    MaskGroups(VMenu, Vec<String>, bool),
    CodePages(crate::codepages::CodePagesMenu),
    /// Options > File panel modes.
    PanelModes(VMenu),
    /// Options > Colors: the groups, a group's items (where its menu is), Themes.
    ColorGroups(VMenu),
    ColorItems(VMenu, &'static [crate::colordata::Item], (u16, u16)),
    Themes(VMenu),
    /// Options > Files highlighting and sort groups: what each row is.
    Highlight(VMenu, Vec<crate::optmenu::HiRow>),
}

impl List {
    pub fn paint(&mut self, grid: &mut crate::grid::Grid) {
        self.menu_mut().paint(grid);
    }

    fn menu_mut(&mut self) -> &mut VMenu {
        match self {
            List::History(h) => &mut h.menu,
            List::UserMenu(u) => &mut u.menu,
            List::Filters(f) => &mut f.menu,
            List::CodePages(c) => &mut c.menu,
            List::Shortcuts(m)
            | List::Assocs(m)
            | List::SelectAssoc(m, _)
            | List::Plugins(m)
            | List::ArcLite(m)
            | List::Screens(m, _)
            | List::Tasks(m, _, _)
            | List::Hotplug(m, _)
            | List::Languages(m, _)
            | List::PanelModes(m)
            | List::ColorGroups(m)
            | List::ColorItems(m, _, _)
            | List::Themes(m)
            | List::Highlight(m, _)
            | List::MaskGroups(m, _, _) => m,
        }
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn quote(text: &str) -> String {
    format!("\"{text}\"")
}

impl App {
    /// The menu's commands and their keys (F2, F11 and F12 come as requests).
    pub(crate) fn run_command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::FindFile => self.open_find(),
            Cmd::History => self.open_history(Which::Commands),
            Cmd::ViewHistory => self.open_history(Which::Views),
            Cmd::FoldersHistory => self.open_history(Which::Folders),
            Cmd::VideoMode => {
                // Far maximizes the console window or brings it back (Alt+F9): xterm's window operation
                self.maximized = !self.maximized;
                let code = if self.maximized { "\x1b[9;1t" } else { "\x1b[9;0t" };
                let mut out = std::io::stdout();
                let _ = std::io::Write::write_all(&mut out, code.as_bytes());
                let _ = std::io::Write::flush(&mut out);
            }
            Cmd::CompareFolders => match self.panels.compare_folders() {
                None => {
                    let lines = ["Two file panels are required to perform".into(), "the Compare folders command".into()];
                    self.overlay = Overlay::Dialog(Dialog::message("Compare", &lines, &[("OK", "ok")], true, Action::Info));
                }
                Some(true) => {
                    let lines = ["The folders contents seems".into(), "to be identical".into()];
                    self.overlay = Overlay::Dialog(Dialog::message("Compare", &lines, &[("OK", "ok")], false, Action::Info));
                }
                Some(false) => {}
            },
            Cmd::EditUserMenu => {
                let buttons = [("&Main", "main"), ("&Local", "local"), ("Cancel", "cancel")];
                self.overlay = Overlay::Dialog(Dialog::message("User menu", &["Choose user menu type to edit".into()], &buttons, false, Action::UserMenuChoose));
            }
            Cmd::UserMenu => {
                let name = self.panels.current_name().unwrap_or_default().to_string();
                let menu = usermenu::UserMenu::open(self.panels.active_dir(), &profile::dir(), &name);
                self.overlay = Overlay::List(Box::new(List::UserMenu(Box::new(menu))));
            }
            Cmd::FileAssociations => self.overlay = Overlay::List(Box::new(List::Assocs(assoc::Assocs::load().menu(0)))),
            Cmd::FolderShortcuts => self.overlay = Overlay::List(Box::new(List::Shortcuts(shortcuts::Shortcuts::load().menu(0)))),
            Cmd::Filter => {
                let menu = filter::FiltersMenu::new(self.panels.filter_area(), &self.panels.names());
                self.overlay = Overlay::List(Box::new(List::Filters(menu)));
            }
            Cmd::PluginCommands => {
                let mut menu = VMenu::with("Plugin commands", vec![Entry::new("ArcLite")]);
                menu.max_height_margin = Some(4);
                self.overlay = Overlay::List(Box::new(List::Plugins(menu)));
            }
            Cmd::ScreensList => self.open_screens(None),
            Cmd::TaskList => {
                let tasks = tasklist::list();
                let mut menu = VMenu::with("Task list", tasklist::entries(&tasks, false));
                menu.max_height_margin = Some(4);
                menu.show_ampersand = true;
                menu.auto_highlight = true;
                menu.bottom = "Del, F2, Ctrl+R".into();
                self.overlay = Overlay::List(Box::new(List::Tasks(menu, tasks, false)));
            }
            Cmd::Hotplug => {
                let devices = hotplug::list();
                self.overlay = Overlay::List(Box::new(List::Hotplug(hotplug_menu(&devices, 0), devices)));
            }
        }
    }

    /// Keys over the panels that are the Commands menu's (and Far's own
    /// keys for what it lists); `true` when one was used.
    pub(crate) fn command_key(&mut self, key: KeyEvent) -> bool {
        let m = key.modifiers;
        let alt = m == KeyModifiers::ALT;
        let cmd = match key.code {
            KeyCode::F(7) if alt => Some(Cmd::FindFile),
            KeyCode::F(8) if alt => Some(Cmd::History),
            KeyCode::F(9) if alt => Some(Cmd::VideoMode),
            KeyCode::F(11) if alt => Some(Cmd::ViewHistory),
            KeyCode::F(12) if alt => Some(Cmd::FoldersHistory),
            KeyCode::F(9) if m == KeyModifiers::SHIFT => {
                self.run_option(crate::menu::Opt::SaveSetup);
                return true;
            }
            // Ctrl+W is readline's: only while nothing is typed
            KeyCode::Char('w') if m == KeyModifiers::CONTROL && self.shell.command_line_is_empty() => Some(Cmd::TaskList),
            KeyCode::Char('i') if m == KeyModifiers::CONTROL => Some(Cmd::Filter),
            _ => None,
        };
        if let Some(cmd) = cmd {
            self.run_command(cmd);
            return true;
        }
        // the file's associations (ProcessLocalFileTypes)
        let mode = match key.code {
            KeyCode::Enter if m.is_empty() && self.shell.command_line_is_empty() => Some(assoc::Mode::Exec),
            KeyCode::PageDown if m == KeyModifiers::CONTROL => Some(assoc::Mode::AltExec),
            KeyCode::F(3) if m.is_empty() => Some(assoc::Mode::View),
            KeyCode::F(3) if alt => Some(assoc::Mode::AltView),
            KeyCode::F(4) if m.is_empty() => Some(assoc::Mode::Edit),
            KeyCode::F(4) if alt => Some(assoc::Mode::AltEdit),
            _ => None,
        };
        if let (Some(mode), Some(file)) = (mode, self.panels.current_file())
            && self.run_association(&file, mode)
        {
            return true;
        }
        // Alt+F3 without an association: the other viewer
        if key.code == KeyCode::F(3)
            && alt
            && let Some(file) = self.panels.current_file()
        {
            self.alt_view(&file);
            return true;
        }
        false
    }

    fn jump_to_shortcut(&mut self, index: usize) {
        if let Some(dir) = shortcuts::Shortcuts::load().folder(index) {
            self.panels.open_at(dir, None);
            if self.shell.command_line_is_empty() {
                self.shell.cd(self.panels.active_dir());
            }
        }
    }

    /// The commands of the file's associations for this key: none (the key
    /// does what it does), one (run it), several ("Select association").
    fn run_association(&mut self, file: &Path, mode: assoc::Mode) -> bool {
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let found = assoc::Assocs::load().commands_for(&name, mode);
        if found.is_empty() {
            return false;
        }
        let names = vec![name.clone()];
        let mut labels = Vec::new();
        let mut commands = Vec::new();
        for (description, command) in found {
            let command = self.substitute(&command, &name, &names);
            if command.trim().is_empty() {
                continue;
            }
            labels.push(if description.is_empty() { command.clone() } else { self.substitute(&description, &name, &names) });
            commands.push(command);
        }
        match commands.len() {
            0 => {}
            1 => self.run_lines(&commands, true),
            _ => self.overlay = Overlay::List(Box::new(List::SelectAssoc(assoc::select_menu(labels), commands))),
        }
        true
    }

    /// Runs these lines in the shell one after another; the panels come
    /// back when the last is done (as after F4).
    pub(crate) fn run_lines(&mut self, lines: &[String], remember: bool) {
        if lines.is_empty() {
            return;
        }
        self.shell.clear_line();
        if self.panels_visible && self.shell.kind() != crate::cwd::Kind::Other {
            self.editing = Some(Editing { until: self.shell.prompts() + lines.len() as u64, name: None });
        }
        for line in lines {
            if remember {
                self.commands.add(line, history::Kind::Plain);
            }
            self.shell.run(line);
        }
        self.panels_visible = false;
        self.under.clear();
    }

    fn history_of(&mut self, which: Which) -> &mut History {
        match which {
            Which::Commands => &mut self.commands,
            Which::Views => &mut self.views,
            Which::Folders => &mut self.folders,
        }
    }

    fn open_history(&mut self, which: Which) {
        let menu = self.history_of(which).menu();
        self.overlay = Overlay::List(Box::new(List::History(menu)));
    }

    /// F12: the panels, then the viewers in the background (`from`: the
    /// viewer F12 was pressed in, now among them).
    fn open_screens(&mut self, from: Option<usize>) {
        let name = self.panels.current_name().unwrap_or_default().to_string();
        let mut rows: Vec<(String, String)> = vec![("Panels".into(), name)];
        rows.extend(self.screens.iter().map(|v| ("View".into(), v.name().to_string())));
        let width = rows.iter().map(|(t, _)| t.len()).max().unwrap_or(0);
        let entries = rows
            .iter()
            .enumerate()
            .map(|(i, (kind, name))| {
                let hot = match i {
                    0..=9 => format!("&{i}"),
                    10..=35 => format!("&{}", (b'A' + (i - 10) as u8) as char),
                    _ => " ".into(),
                };
                Entry::new(format!("{hot}  {kind:<width$}   {}", vmenu_escape(name)))
            })
            .collect();
        let mut menu = VMenu::with("Screens", entries);
        menu.max_height_margin = Some(4);
        menu.select(from.map_or(0, |i| i + 1));
        self.overlay = Overlay::List(Box::new(List::Screens(menu, from)));
    }

    /// F12 in a viewer: it goes among the screens.
    pub(crate) fn screens_from_viewer(&mut self, viewer: crate::viewer::Viewer) {
        self.screens.push(viewer);
        self.open_screens(Some(self.screens.len() - 1));
    }

    fn open_find(&mut self) {
        let dialog = findfile::dialog(&self.find_options, self.panels.active_dir());
        self.overlay = Overlay::Dialog(dialog);
    }

    /// A key over a list.
    pub(crate) fn list_key(&mut self, list: List, key: KeyEvent) {
        match list {
            List::Languages(menu, help) => self.languages_key(menu, help, key),
            List::MaskGroups(menu, names, filtered) => self.mask_groups_key(menu, names, filtered, key),
            List::CodePages(menu) => self.code_pages_key(menu, key),
            List::PanelModes(menu) => self.panel_modes_key(menu, key),
            List::ColorGroups(menu) => self.color_groups_key(menu, key),
            List::Highlight(menu, rows) => self.highlight_key(menu, rows, key),
            List::ColorItems(menu, items, at) => self.color_items_key(menu, items, at, key),
            List::Themes(mut menu) => match menu.key(key) {
                Outcome::Chosen(_) => {
                    crate::colors::reset_all();
                }
                Outcome::Closed => {}
                _ => self.overlay = Overlay::List(Box::new(List::Themes(menu))),
            },
            List::History(mut menu) => {
                let which = menu.which;
                let outcome = menu.key(key, self.history_of(which));
                match outcome {
                    history::MenuOutcome::Open => self.overlay = Overlay::List(Box::new(List::History(menu))),
                    history::MenuOutcome::Closed => {}
                    history::MenuOutcome::AskClear if !crate::settings::bool("Confirmations.HistoryClear") => {
                        self.history_of(which).clear();
                        let history = match which {
                            Which::Commands => &self.commands,
                            Which::Views => &self.views,
                            Which::Folders => &self.folders,
                        };
                        menu.refill(history);
                        self.overlay = Overlay::List(Box::new(List::History(menu)));
                    }
                    history::MenuOutcome::AskClear => {
                        let title = which.title();
                        let buttons = [("&Clear history", "ok"), ("Cancel", "cancel")];
                        let lines = ["All records in the history will be deleted. Continue?".to_string()];
                        self.under.push(Overlay::List(Box::new(List::History(menu))));
                        self.overlay = Overlay::Dialog(Dialog::message(title, &lines, &buttons, true, Action::ClearHistory));
                    }
                    history::MenuOutcome::Picked(picked) => self.history_picked(which, picked),
                }
            }
            List::Shortcuts(mut menu) => match menu.key(key) {
                Outcome::Open => self.overlay = Overlay::List(Box::new(List::Shortcuts(menu))),
                Outcome::Closed => {}
                Outcome::Chosen(index) => self.jump_to_shortcut(index),
                Outcome::Key(key) => {
                    let index = menu.selected();
                    let mut all = shortcuts::Shortcuts::load();
                    match key.code {
                        KeyCode::Insert => all.set(index, &self.panels.active_dir().to_path_buf()),
                        KeyCode::Delete => {
                            all.items[index] = None;
                            let _ = all.save();
                        }
                        KeyCode::F(4) => {
                            if let Some(shortcut) = all.items[index].clone() {
                                self.under.push(Overlay::List(Box::new(List::Shortcuts(menu))));
                                self.overlay = Overlay::Dialog(shortcuts::edit_dialog(&shortcut, index));
                                return;
                            }
                        }
                        _ => {}
                    }
                    self.overlay = Overlay::List(Box::new(List::Shortcuts(all.menu(index))));
                }
            },
            List::Assocs(mut menu) => {
                let outcome = menu.key(key);
                let index = menu.selected();
                let all = assoc::Assocs::load();
                let edit = |app: &mut App, menu: VMenu, dialog: Dialog| {
                    app.under.push(Overlay::List(Box::new(List::Assocs(menu))));
                    app.overlay = Overlay::Dialog(dialog);
                };
                match outcome {
                    Outcome::Open => self.overlay = Overlay::List(Box::new(List::Assocs(menu))),
                    Outcome::Closed => {}
                    Outcome::Chosen(_) => {
                        if let Some(a) = all.items.get(index) {
                            let dialog = assoc::edit_dialog(a, index, false);
                            edit(self, menu, dialog);
                        }
                    }
                    Outcome::Key(key) => match key.code {
                        KeyCode::F(4) if index < all.items.len() => {
                            let dialog = assoc::edit_dialog(&all.items[index], index, false);
                            edit(self, menu, dialog);
                        }
                        KeyCode::Insert => {
                            let at = index.min(all.items.len());
                            let dialog = assoc::edit_dialog(&assoc::Assoc::default(), at, true);
                            edit(self, menu, dialog);
                        }
                        KeyCode::Delete if index < all.items.len() => {
                            let lines = ["Do you wish to delete association for".to_string(), quote(&all.items[index].masks)];
                            let dialog = Dialog::message("File associations", &lines, &[("Delete", "ok"), ("Cancel", "cancel")], true, Action::DeleteAssoc(index));
                            edit(self, menu, dialog);
                        }
                        KeyCode::Up | KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            let mut all = all;
                            let other = if key.code == KeyCode::Up { index.checked_sub(1) } else { Some(index + 1).filter(|&j| j < all.items.len()) };
                            match other {
                                Some(j) if index < all.items.len() => {
                                    all.items.swap(index, j);
                                    let _ = all.save();
                                    self.overlay = Overlay::List(Box::new(List::Assocs(all.menu(j))));
                                }
                                _ => self.overlay = Overlay::List(Box::new(List::Assocs(menu))),
                            }
                        }
                        _ => self.overlay = Overlay::List(Box::new(List::Assocs(menu))),
                    },
                }
            }
            List::SelectAssoc(mut menu, commands) => match menu.key(key) {
                Outcome::Chosen(index) => {
                    let line = commands[index].clone();
                    self.run_lines(&[line], true);
                }
                Outcome::Closed => {}
                _ => self.overlay = Overlay::List(Box::new(List::SelectAssoc(menu, commands))),
            },
            List::UserMenu(mut menu) => {
                let outcome = menu.key(key, &profile::dir());
                self.user_menu_outcome(menu, outcome);
            }
            List::Filters(mut menu) => {
                let outcome = menu.key(key);
                self.filters_outcome(menu, outcome);
            }
            List::Plugins(mut menu) => match menu.key(key) {
                Outcome::Chosen(_) => {
                    let items = ["&Open archive", "Open (&format menu)", "&Create archive", "&Extract archive(s)", "&Test archive(s)", "C&onvert archive into SFX"];
                    let mut arc = VMenu::new("ArcLite", items.iter().map(|s| s.to_string()).collect());
                    arc.max_height_margin = Some(4);
                    self.overlay = Overlay::List(Box::new(List::ArcLite(arc)));
                }
                Outcome::Closed => {}
                _ => self.overlay = Overlay::List(Box::new(List::Plugins(menu))),
            },
            List::ArcLite(mut menu) => match menu.key(key) {
                Outcome::Chosen(index) => self.arclite_command(index),
                Outcome::Closed => {}
                _ => self.overlay = Overlay::List(Box::new(List::ArcLite(menu))),
            },
            List::Screens(mut menu, from) => match menu.key(key) {
                Outcome::Chosen(0) => {}
                Outcome::Chosen(index) => {
                    let viewer = self.screens.remove(index - 1);
                    self.overlay = Overlay::Viewer(viewer);
                }
                Outcome::Closed => {
                    if let Some(from) = from.filter(|&i| i < self.screens.len()) {
                        let viewer = self.screens.remove(from);
                        self.overlay = Overlay::Viewer(viewer);
                    }
                }
                _ => self.overlay = Overlay::List(Box::new(List::Screens(menu, from))),
            },
            List::Tasks(mut menu, tasks, show_image) => match menu.key(key) {
                Outcome::Chosen(index) => {
                    if let Some(task) = tasks.get(index) {
                        tasklist::switch_to(task);
                    }
                }
                Outcome::Closed => {}
                Outcome::Key(k) if k.code == KeyCode::Delete && !tasks.is_empty() => {
                    let task = &tasks[menu.selected()];
                    let lines = ["Do you wish to kill selected task?".to_string(), task.title.clone(), "You will lose any unsaved information in this program".into()];
                    let dialog = Dialog::message("Kill task", &lines, &[("Kill", "ok"), ("Cancel", "cancel")], true, Action::KillTask(task.pid));
                    self.under.push(Overlay::List(Box::new(List::Tasks(menu, tasks, show_image))));
                    self.overlay = Overlay::Dialog(dialog);
                }
                Outcome::Key(k) if k.code == KeyCode::F(2) => self.overlay = Overlay::List(Box::new(task_list(menu.selected(), !show_image))),
                Outcome::Key(k) if k.code == KeyCode::Char('r') && k.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.overlay = Overlay::List(Box::new(task_list(menu.selected(), show_image)));
                }
                _ => self.overlay = Overlay::List(Box::new(List::Tasks(menu, tasks, show_image))),
            },
            List::Hotplug(mut menu, devices) => match menu.key(key) {
                Outcome::Closed => {}
                Outcome::Key(k) if k.code == KeyCode::Delete && !devices.is_empty() && !crate::settings::bool("Confirmations.RemoveHotPlug") => {
                    let index = menu.selected();
                    self.under.push(Overlay::List(Box::new(List::Hotplug(menu, devices))));
                    self.accept_command(Action::Eject(index), "ok", &Values::default());
                }
                Outcome::Key(k) if k.code == KeyCode::Delete && !devices.is_empty() => {
                    let index = menu.selected();
                    let device = &devices[index];
                    let mut lines = vec!["Do you want to remove the device".to_string(), device.description.clone()];
                    if !device.friendly.is_empty() && !device.friendly.eq_ignore_ascii_case(&device.description) {
                        lines.push(device.friendly.clone());
                    }
                    let dialog = Dialog::message("Device Removal", &lines, &[("R&emove", "ok"), ("&Cancel", "cancel")], true, Action::Eject(index));
                    self.under.push(Overlay::List(Box::new(List::Hotplug(menu, devices))));
                    self.overlay = Overlay::Dialog(dialog);
                }
                Outcome::Key(k) if k.code == KeyCode::Char('r') && k.modifiers.contains(KeyModifiers::CONTROL) => {
                    let devices = hotplug::list();
                    self.overlay = Overlay::List(Box::new(List::Hotplug(hotplug_menu(&devices, menu.selected()), devices)));
                }
                _ => self.overlay = Overlay::List(Box::new(List::Hotplug(menu, devices))),
            },
        }
    }

    /// A click on a list: the item is chosen as Enter would.
    pub(crate) fn list_click(&mut self, mut list: List, event: MouseEvent) {
        match list.menu_mut().click(event) {
            vmenu::Outcome::Chosen(_) => self.list_key(list, key(KeyCode::Enter)),
            vmenu::Outcome::Closed => self.list_key(list, key(KeyCode::Esc)),
            _ => self.overlay = Overlay::List(Box::new(list)),
        }
    }

    fn history_picked(&mut self, which: Which, picked: history::Picked) {
        use history::Picked;
        match (which, picked) {
            (_, Picked::ToCommandLine(text)) => {
                self.shell.clear_line();
                self.shell.write(text.as_bytes(), false);
            }
            (Which::Commands, Picked::Enter(record, _)) => self.run_lines(&[record.name], true),
            (Which::Views, Picked::Enter(record, _)) => {
                let path = PathBuf::from(&record.name);
                let request = if record.kind == history::Kind::Edit { Request::Edit(path) } else { Request::View(path) };
                self.request(request);
            }
            (Which::Views, Picked::View(name)) => self.request(Request::View(PathBuf::from(name))),
            (Which::Views, Picked::Edit(name)) => self.request(Request::Edit(PathBuf::from(name))),
            (Which::Folders, Picked::Enter(record, quietly)) => {
                let dir = PathBuf::from(&record.name);
                if quietly {
                    self.last_folder = dir.clone();
                }
                self.panels.open_at(dir, None);
                if self.shell.command_line_is_empty() {
                    self.shell.cd(self.panels.active_dir());
                }
            }
            _ => {}
        }
    }

    fn user_menu_outcome(&mut self, menu: Box<usermenu::UserMenu>, outcome: usermenu::MenuOutcome) {
        use usermenu::MenuOutcome;
        match outcome {
            MenuOutcome::Open => self.overlay = Overlay::List(Box::new(List::UserMenu(menu))),
            MenuOutcome::Closed => self.panels.refresh(None),
            MenuOutcome::Run { commands } => {
                let name = menu.name.clone();
                let names = if name.is_empty() { vec![] } else { vec![name.clone()] };
                let lines: Vec<String> = commands.iter().map(|c| self.substitute(c, &name, &names)).filter(|l| !l.trim().is_empty()).collect();
                self.run_lines(&lines, false);
            }
            MenuOutcome::AskInsert => {
                let lines = ["Do you wish to insert a new command or a new menu?".to_string()];
                let dialog = Dialog::message("User menu", &lines, &[("Insert command", "command"), ("Insert menu", "menu")], false, Action::UserMenuInsert);
                self.under.push(Overlay::List(Box::new(List::UserMenu(menu))));
                self.overlay = Overlay::Dialog(dialog);
            }
            MenuOutcome::AskDelete { label, submenu } => {
                let question = if submenu { "Do you wish to delete the submenu" } else { "Do you wish to delete the menu item" };
                let lines = [question.to_string(), quote(&label)];
                let dialog = Dialog::message("User menu", &lines, &[("Delete", "ok"), ("Cancel", "cancel")], true, Action::UserMenuDelete);
                self.under.push(Overlay::List(Box::new(List::UserMenu(menu))));
                self.overlay = Overlay::Dialog(dialog);
            }
            MenuOutcome::Edit { item } => {
                let dialog = usermenu::edit_dialog(&item, item.submenu.is_some(), false);
                self.under.push(Overlay::List(Box::new(List::UserMenu(menu))));
                self.overlay = Overlay::Dialog(dialog);
            }
            MenuOutcome::EditFile(path) => {
                self.under.clear();
                self.request(Request::Edit(path));
            }
        }
    }

    fn filters_outcome(&mut self, menu: filter::FiltersMenu, outcome: filter::MenuOutcome) {
        use filter::MenuOutcome;
        let over = |app: &mut App, menu: filter::FiltersMenu, dialog: Dialog| {
            app.under.push(Overlay::List(Box::new(List::Filters(menu))));
            app.overlay = Overlay::Dialog(dialog);
        };
        match outcome {
            MenuOutcome::Open => self.overlay = Overlay::List(Box::new(List::Filters(menu))),
            MenuOutcome::Closed(changed) => {
                if changed {
                    menu.apply();
                    self.panels.refilter();
                }
            }
            MenuOutcome::Edit(index) => {
                // the marks so far are kept: the menu is filled again afterwards
                menu.apply();
                let filter = filter::with(|f| f.custom[index].clone());
                over(self, menu, filter::dialog(&filter, index, false));
            }
            MenuOutcome::New(at, template) => {
                menu.apply();
                over(self, menu, filter::dialog(&template, at, true));
            }
            MenuOutcome::AskDelete(index) => {
                menu.apply();
                let title = filter::with(|f| f.custom[index].title.clone());
                let lines = ["Do you wish to delete the filter".to_string(), quote(&title)];
                over(self, menu, Dialog::message("Filters menu", &lines, &[("Delete", "ok"), ("Cancel", "cancel")], false, Action::DeleteFilter(index)));
            }
            MenuOutcome::CustomOnly(text) => over(self, menu, Dialog::message("Filters menu", &[text.to_string()], &[("OK", "ok")], true, Action::Info)),
        }
    }

    /// arclite's plugin menu (OPEN_PLUGINSMENU).
    fn arclite_command(&mut self, index: usize) {
        let asked = match index {
            0 | 1 => {
                let is_archive = self.panels.current_file().is_some_and(|f| crate::archive::detect(&f).is_some());
                if is_archive {
                    self.panels.key(key(KeyCode::Enter), true);
                }
                return;
            }
            2 => self.panels.add_to_archive_request(),
            3 => self.panels.extract_archive_request(),
            4 => match self.panels.archive_commands_request() {
                Key::Ask(Request::ArchiveCommands(picks)) => return self.test_archives(picks),
                other => other,
            },
            _ => Key::Ask(Request::Error("Archive cannot be converted into SFX archive".into())),
        };
        if let Key::Ask(request) = asked {
            self.request(request);
        }
    }

    /// A button pressed in one of these dialogs; the list under it (if any) is updated.
    pub(crate) fn accept_command(&mut self, action: Action, id: &str, values: &Values) {
        match action {
            Action::ClearHistory => {
                if let Some(Overlay::List(list)) = self.under.last_mut()
                    && let List::History(menu) = list.as_mut()
                {
                    let which = menu.which;
                    let history = match which {
                        Which::Commands => &mut self.commands,
                        Which::Views => &mut self.views,
                        Which::Folders => &mut self.folders,
                    };
                    history.clear();
                    menu.refill(history);
                }
            }
            Action::Shortcut(index) => {
                let mut all = shortcuts::Shortcuts::load();
                let (title, folder) = (values.text("title"), values.text("path"));
                all.items[index] = if title.is_empty() && folder.is_empty() { None } else { Some(shortcuts::Shortcut { title, folder }) };
                let _ = all.save();
                self.replace_under(List::Shortcuts(all.menu(index)));
            }
            Action::Assoc { index, new } => {
                let mut all = assoc::Assocs::load();
                let edited = assoc::from_values(values);
                if new {
                    all.items.insert(index.min(all.items.len()), edited);
                } else if index < all.items.len() {
                    all.items[index] = edited;
                }
                let _ = all.save();
                self.replace_under(List::Assocs(all.menu(index)));
            }
            Action::DeleteAssoc(index) => {
                let mut all = assoc::Assocs::load();
                if index < all.items.len() {
                    all.items.remove(index);
                    let _ = all.save();
                }
                self.replace_under(List::Assocs(all.menu(index)));
            }
            Action::KillTask(pid) => {
                let task = tasklist::list().into_iter().find(|t| t.pid == pid);
                if let Some(task) = task
                    && let Err(error) = tasklist::kill(&task)
                {
                    let lines = ["Cannot kill the specified task".to_string(), error.to_string()];
                    self.overlay = Overlay::Dialog(Dialog::message("Kill task", &lines, &[("OK", "ok")], true, Action::Info));
                }
                let selected = self.under_menu_selected();
                self.replace_under(task_list(selected, false));
            }
            Action::Eject(index) => {
                let devices = hotplug::list();
                let parked: Vec<hotplug::Device> = match self.under.last() {
                    Some(Overlay::List(list)) => match list.as_ref() {
                        List::Hotplug(_, d) => d.clone(),
                        _ => devices.clone(),
                    },
                    _ => devices.clone(),
                };
                if let Some(device) = parked.get(index) {
                    let (lines, warning, title) = if hotplug::eject(device) {
                        let mut lines = vec!["The device".to_string(), device.description.clone()];
                        if !device.friendly.is_empty() {
                            lines.push(device.friendly.clone());
                        }
                        lines.push("can now be safely removed".into());
                        (lines, false, "Device Removal")
                    } else {
                        (vec!["Cannot remove a device:".to_string(), device.label()], true, "Error")
                    };
                    self.overlay = Overlay::Dialog(Dialog::message(title, &lines, &[("OK", "ok")], warning, Action::Info));
                }
                let devices = hotplug::list();
                self.replace_under(List::Hotplug(hotplug_menu(&devices, index), devices));
            }
            Action::UserMenuChoose => {
                let dir = self.panels.active_dir().to_path_buf();
                let name = self.panels.current_name().unwrap_or_default().to_string();
                let (mode, file) = if id == "main" { (usermenu::Mode::Main, profile::path("FarMenu.ini")) } else { (usermenu::Mode::Local, dir.join("FarMenu.ini")) };
                let menu = usermenu::UserMenu::load(mode, dir, file, &name);
                self.overlay = Overlay::List(Box::new(List::UserMenu(Box::new(menu))));
            }
            Action::UserMenuInsert => {
                let submenu = id == "menu";
                self.overlay = Overlay::Dialog(usermenu::edit_dialog(&usermenu::Item::default(), submenu, true));
            }
            Action::UserMenuEdit { new, submenu } => {
                if let Some(Overlay::List(list)) = self.under.last_mut()
                    && let List::UserMenu(menu) = list.as_mut()
                {
                    if new {
                        menu.insert(usermenu::from_values(values, submenu.then(Vec::new)));
                    } else {
                        let children = menu.items().get(menu.menu.selected()).and_then(|item| item.submenu.clone());
                        menu.replace(usermenu::from_values(values, if submenu { children.or_else(|| Some(vec![])) } else { None }));
                    }
                }
            }
            Action::UserMenuDelete => {
                if let Some(Overlay::List(list)) = self.under.last_mut()
                    && let List::UserMenu(menu) = list.as_mut()
                {
                    menu.delete();
                }
            }
            Action::Filter { index, new, attributes } => {
                filter::with(|filters| {
                    if new {
                        let edited = filter::from_values(values, attributes, [None; filter::AREAS]);
                        filters.custom.insert(index.min(filters.custom.len()), edited);
                    } else if index < filters.custom.len() {
                        let flags = filters.custom[index].flags;
                        filters.custom[index] = filter::from_values(values, attributes, flags);
                    }
                    filters.save();
                });
                self.refill_filters(index);
            }
            Action::DeleteFilter(index) => {
                filter::with(|filters| {
                    if index < filters.custom.len() {
                        filters.custom.remove(index);
                        filters.save();
                    }
                });
                self.refill_filters(index);
            }
            Action::FindFile => {
                self.find_options = findfile::Options::from_values(values);
                match id {
                    "drive" => {
                        self.find_after_drive = true;
                        self.request(Request::ChangeDrive { right: self.panels.right_active() });
                    }
                    "filterbtn" => {
                        self.under.push(Overlay::Dialog(findfile::dialog(&self.find_options, self.panels.active_dir())));
                        let menu = filter::FiltersMenu::new(filter::FIND_AREA, &self.panels.names());
                        self.overlay = Overlay::List(Box::new(List::Filters(menu)));
                    }
                    _ => {
                        let results = findfile::Results::start(self.find_options.clone(), self.panels.active_dir(), self.panels.selected_dirs());
                        self.overlay = Overlay::Find(Box::new(results));
                    }
                }
            }
            other => self.accept_option(other, id, values),
        }
    }

    /// The filters menu under the dialog, filled again; the panels read
    /// again, as the marks may have changed.
    fn refill_filters(&mut self, select: usize) {
        if let Some(Overlay::List(list)) = self.under.last_mut()
            && let List::Filters(menu) = list.as_mut()
        {
            menu.fill(select);
        }
        self.panels.refilter();
    }

    fn under_menu_selected(&mut self) -> usize {
        match self.under.last_mut() {
            Some(Overlay::List(list)) => list.menu_mut().selected(),
            _ => 0,
        }
    }

    /// The list under the dialog, replaced by a new one of the same kind.
    fn replace_under(&mut self, list: List) {
        if let Some(Overlay::List(parked)) = self.under.last_mut() {
            **parked = list;
        }
    }

    /// A key in the Find file results.
    pub(crate) fn find_outcome(&mut self, mut results: Box<findfile::Results>, outcome: findfile::Outcome) {
        match outcome {
            findfile::Outcome::Open => self.overlay = Overlay::Find(results),
            findfile::Outcome::Closed => {}
            findfile::Outcome::GoTo(dir, name) => {
                results.stop();
                self.panels_visible = true;
                self.panels.open_at(dir, name.as_deref());
                if self.shell.command_line_is_empty() {
                    self.shell.cd(self.panels.active_dir());
                }
            }
            findfile::Outcome::View(path) => {
                self.under.push(Overlay::Find(results));
                self.request(Request::View(path));
            }
            findfile::Outcome::Edit(path) => {
                self.under.push(Overlay::Find(results));
                self.request(Request::Edit(path));
            }
            findfile::Outcome::NewSearch => self.open_find(),
        }
    }
}

fn vmenu_escape(text: &str) -> String {
    crate::dialog::escape_ampersands(text)
}

fn task_list(select: usize, show_image: bool) -> List {
    let tasks = tasklist::list();
    let mut menu = VMenu::with("Task list", tasklist::entries(&tasks, show_image));
    menu.max_height_margin = Some(4);
    menu.show_ampersand = true;
    menu.auto_highlight = true;
    menu.bottom = "Del, F2, Ctrl+R".into();
    menu.select(select);
    List::Tasks(menu, tasks, show_image)
}

fn hotplug_menu(devices: &[hotplug::Device], select: usize) -> VMenu {
    let mut menu = VMenu::with("Hotplug devices list", devices.iter().map(|d| Entry::new(d.label())).collect());
    menu.show_ampersand = true;
    menu.auto_highlight = true;
    menu.bottom = "Del, Ctrl+R".into();
    menu.select(select);
    menu
}
