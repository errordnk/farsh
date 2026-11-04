//! farsh: the user's own shell (bash / zsh / PowerShell) in a PTY, with Far
//! Manager style panels that Ctrl+O shows and hides.

mod arcdialog;
mod archive;
mod attributes;
mod cwd;
mod dialog;
mod drives;
mod editor;
mod fileops;
mod grid;
mod keys;
mod listing;
mod menu;
mod mouse;
mod palette;
mod panels;
mod shell;
mod sorting;
mod sortmenu;
mod viewer;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::terminal;

use dialog::{Action, Dialog};
use fileops::{Access, Answer, Existing, Job, Kind, Options};
use grid::Grid;
use menu::Menu;
use panels::{Key, Panels, Request};
use shell::Shell;
use viewer::Viewer;

fn main() -> Result<()> {
    system_dlls_only();
    let (cols, rows) = terminal::size()?;
    let mut app = App::new(cols, rows)?;

    terminal::enable_raw_mode()?;
    let mut out = std::io::stdout();
    crossterm::execute!(out, terminal::EnterAlternateScreen, EnableMouseCapture, crossterm::cursor::Hide)?;
    let result = app.run(cols, rows);
    crossterm::execute!(out, crossterm::cursor::Show, DisableMouseCapture, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    result
}

/// DLLs load from Windows itself (and farsh's own folder), never from PATH or
/// the current folder. portable-pty loads `conpty.dll` by bare name: a copy
/// some other program put on PATH would replace Windows' own ConPTY.
#[cfg(windows)]
fn system_dlls_only() {
    use windows_sys::Win32::System::LibraryLoader::{SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS};
    unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) };
}

#[cfg(not(windows))]
fn system_dlls_only() {}

/// What is on top of the shell and the panels.
enum Overlay {
    None,
    Dialog(Dialog),
    Viewer(Viewer),
    /// F9: the menu bar with a drop-down.
    Menu(Menu),
    /// Alt+F1 / Alt+F2: the drives or mounted file systems.
    Drives(drives::DriveMenu),
    Sort(sortmenu::SortMenu),
    /// A copy, move or delete running, and the question it is waiting on, if any.
    Job(Job, Option<Dialog>),
}

struct App {
    shell: Shell,
    panels: Panels,
    panels_visible: bool,
    overlay: Overlay,
    quit: bool,
    /// The last left click: when and where, to tell a double click.
    last_click: Option<(std::time::Instant, u16, u16)>,
    /// The menu title F9 opened last, to open there again.
    last_menu: Option<usize>,
    size: (u16, u16),
    /// F4 ran an editor over the panels: they come back when it ends.
    editing: Option<Editing>,
}

/// A file being edited in the shell (F4), started at the shell's prompt
/// number `prompts`.
struct Editing {
    prompts: u64,
    name: String,
}

impl App {
    fn new(cols: u16, rows: u16) -> Result<Self> {
        Ok(Self {
            shell: Shell::spawn(cols, rows)?,
            panels: Panels::new(std::env::current_dir()?),
            panels_visible: false,
            overlay: Overlay::None,
            quit: false,
            last_click: None,
            last_menu: None,
            size: (cols, rows),
            editing: None,
        })
    }

    fn run(&mut self, cols: u16, rows: u16) -> Result<()> {
        let mut out = std::io::stdout();
        let mut screen = Grid::new(cols, rows);
        let mut frame = Grid::new(cols, rows);
        let mut dirty = true;

        while !self.quit {
            if self.shell.pump() {
                dirty = true;
            }
            if self.shell.exited() {
                return Ok(());
            }
            // The shell changed directory (a typed `cd`, a script): the panel follows.
            if let Some(dir) = self.shell.take_cwd() {
                self.panels.go_to(dir);
                dirty = true;
            }
            // The editor F4 started has ended (the shell is at its next prompt):
            // the panels are back as they were, on the file edited, as in Far.
            if self.editing.as_ref().is_some_and(|editing| self.shell.prompts() > editing.prompts)
                && let Some(editing) = self.editing.take()
            {
                self.panels_visible = true;
                self.panels.refresh(Some(&editing.name));
                dirty = true;
            }
            if self.poll_job() {
                dirty = true;
            }
            if dirty {
                frame.clear();
                let cursor = self.paint(&mut frame);
                frame.flush(&mut screen, cursor, &mut out)?;
                out.flush()?;
                dirty = false;
            }
            // A copy in progress redraws as it goes.
            let wait = if matches!(self.overlay, Overlay::Job(..)) { 50 } else { 10 };
            if !event::poll(Duration::from_millis(wait))? {
                dirty |= matches!(self.overlay, Overlay::Job(..));
                continue;
            }
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    dirty = true;
                    log_key(&key);
                    self.on_key(key);
                }
                Event::Mouse(mouse) => {
                    dirty = true;
                    self.on_mouse(mouse);
                }
                Event::Resize(cols, rows) => {
                    // while a window is dragged the events queue up: the last size is the real one
                    let (cols, rows) = terminal::size().unwrap_or((cols, rows));
                    self.size = (cols, rows);
                    self.shell.resize(cols, rows);
                    screen = Grid::unknown(cols, rows);
                    frame = Grid::new(cols, rows);
                    dirty = true;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Draws everything into `frame`; returns where the cursor goes.
    fn paint(&mut self, frame: &mut Grid) -> Option<(u16, u16)> {
        self.shell.paint(frame);
        let mut cursor = if self.panels_visible {
            let (_, col) = self.shell.cursor_line();
            let line = frame.row(self.shell.cursor_row());
            self.panels.paint(frame, &line, col)
        } else {
            self.shell.cursor()
        };
        match &mut self.overlay {
            Overlay::None => {}
            Overlay::Dialog(dialog) => cursor = dialog.paint(frame),
            Overlay::Viewer(viewer) => {
                viewer.paint(frame);
                cursor = None;
            }
            Overlay::Menu(menu) => {
                menu.paint(frame);
                cursor = None;
            }
            Overlay::Drives(list) => {
                list.paint(frame);
                cursor = None;
            }
            Overlay::Sort(menu) => {
                menu.paint(frame);
                cursor = None;
            }
            Overlay::Job(job, question) => {
                dialog::paint_progress(job, frame);
                cursor = question.as_ref().and_then(|dialog| dialog.paint(frame));
            }
        }
        cursor
    }

    /// Takes the worker's news; when it is done, closes the box and reads the panels again.
    fn poll_job(&mut self) -> bool {
        let Overlay::Job(job, question) = &mut self.overlay else { return false };
        job.poll();
        if question.is_none() {
            if let Some(asked) = &job.question {
                let wipe = job.kind == Kind::Wipe;
                *question = Some(match asked {
                    fileops::Question::Conflict(conflict) => Dialog::conflict(conflict),
                    fileops::Question::FolderNotEmpty(path) => Dialog::folder_not_empty(path, wipe),
                    fileops::Question::ReadOnly(path) => Dialog::read_only(path, wipe),
                    fileops::Question::HardLinks(path) => Dialog::hard_links(path),
                    fileops::Question::Exists { path, new, old } => arcdialog::overwrite(path, *new, *old),
                });
                return true;
            }
        }
        let Some(result) = job.result.clone() else { return false };
        self.overlay = match result {
            Err(error) if error != "Cancelled" => Overlay::Dialog(Dialog::error(&error)),
            _ => Overlay::None,
        };
        self.panels.refresh(None);
        true
    }

    fn on_key(&mut self, key: KeyEvent) {
        match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::Dialog(mut dialog) => match dialog.key(key) {
                dialog::Outcome::Open => self.overlay = Overlay::Dialog(dialog),
                dialog::Outcome::Closed => {}
                dialog::Outcome::Pressed(action, id, values) => self.accept(action, id, &values),
            },
            Overlay::Viewer(mut viewer) => {
                if matches!(viewer.key(key), viewer::Outcome::Open) {
                    self.overlay = Overlay::Viewer(viewer);
                }
            }
            Overlay::Menu(mut menu) => {
                let outcome = menu.key(key);
                self.after_menu(menu, outcome);
            }
            Overlay::Drives(mut list) => {
                let outcome = list.key(key);
                self.after_drives(list, outcome);
            }
            Overlay::Sort(mut menu) => {
                let outcome = menu.key(key);
                self.after_sort(menu, outcome);
            }
            Overlay::Job(mut job, question) => {
                match question {
                    Some(mut dialog) => {
                        let outcome = dialog.key(key);
                        let question = answer(&mut job, dialog, outcome);
                        self.overlay = Overlay::Job(job, question);
                    }
                    None => {
                        if key.code == KeyCode::Esc {
                            job.cancel();
                        }
                        self.overlay = Overlay::Job(job, None);
                    }
                }
            }
            Overlay::None => self.on_panel_or_shell_key(key),
        }
    }

    fn on_mouse(&mut self, event: MouseEvent) {
        let (col, row) = (event.column, event.row);
        match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::Dialog(mut dialog) => {
                let outcome = if event.kind == MouseEventKind::Down(MouseButton::Left) {
                    dialog.click(col, row, self.size)
                } else {
                    dialog::Outcome::Open
                };
                match outcome {
                    dialog::Outcome::Open => self.overlay = Overlay::Dialog(dialog),
                    dialog::Outcome::Closed => {}
                    dialog::Outcome::Pressed(action, id, values) => self.accept(action, id, &values),
                }
            }
            Overlay::Viewer(mut viewer) => {
                viewer.mouse(event);
                self.overlay = Overlay::Viewer(viewer);
            }
            Overlay::Menu(mut menu) => {
                let outcome = menu.click(event, self.size.0);
                self.after_menu(menu, outcome);
            }
            Overlay::Drives(mut list) => {
                let outcome = list.click(event, self.size.0, self.size.1);
                self.after_drives(list, outcome);
            }
            Overlay::Sort(mut menu) => {
                let outcome = menu.click(event);
                self.after_sort(menu, outcome);
            }
            Overlay::Job(mut job, question) => {
                let question = match question {
                    Some(mut dialog) if event.kind == MouseEventKind::Down(MouseButton::Left) => {
                        let outcome = dialog.click(col, row, self.size);
                        answer(&mut job, dialog, outcome)
                    }
                    other => other,
                };
                self.overlay = Overlay::Job(job, question);
            }
            Overlay::None if self.panels_visible => {
                let double = event.kind == MouseEventKind::Down(MouseButton::Left) && self.is_double_click(col, row);
                let dir_before = self.panels.active_dir().to_path_buf();
                match self.panels.mouse(event, double) {
                    Key::Taken => {
                        if self.panels.active_dir() != dir_before && self.shell.command_line_is_empty() {
                            self.shell.cd(self.panels.active_dir());
                        }
                    }
                    Key::Ask(request) => self.request(request),
                    Key::Pass => {}
                }
            }
            Overlay::None => {
                // A program in the shell that asked for the mouse gets it.
                let (mode, encoding) = self.shell.mouse_mode();
                if let Some(bytes) = mouse::encode(event, mode, encoding) {
                    self.shell.write(&bytes, false);
                }
            }
        }
    }

    /// Whether a left click at (`col`, `row`) now is the second of a double click.
    fn is_double_click(&mut self, col: u16, row: u16) -> bool {
        let now = std::time::Instant::now();
        let double = self
            .last_click
            .is_some_and(|(at, c, r)| (c, r) == (col, row) && now.duration_since(at) < Duration::from_millis(500));
        // a double click is used up: the next one starts again
        self.last_click = if double { None } else { Some((now, col, row)) };
        double
    }

    fn on_panel_or_shell_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.panels_visible = !self.panels_visible;
            return;
        }
        if self.panels_visible && panels::is_insert_name(key) {
            if let Some(name) = self.panels.current_name() {
                let text = editor::name_for_command_line(self.shell.kind(), name);
                self.shell.write(text.as_bytes(), false);
            }
            return;
        }
        if self.panels_visible {
            let dir_before = self.panels.active_dir().to_path_buf();
            match self.panels.key(key, self.shell.command_line_is_empty()) {
                Key::Taken => {
                    // If that took the panel to another folder, the shell goes there too
                    // (unless something is typed on its line: that must stay as it is).
                    if self.panels.active_dir() != dir_before && self.shell.command_line_is_empty() {
                        self.shell.cd(self.panels.active_dir());
                    }
                    return;
                }
                Key::Ask(request) => return self.request(request),
                Key::Pass => {}
            }
        }
        if let Some(bytes) = keys::encode(panels::to_shell(key)) {
            let runs = key.code == KeyCode::Enter;
            self.shell.write(&bytes, runs);
            if runs {
                // the command's output is the shell's to show
                self.panels_visible = false;
            }
        }
    }

    fn request(&mut self, request: Request) {
        match request {
            Request::View(path) => {
                self.overlay = match Viewer::open(&path) {
                    Ok(viewer) => Overlay::Viewer(viewer),
                    Err(error) => Overlay::Dialog(Dialog::error(&format!("{}: {error}", path.display()))),
                };
            }
            Request::Edit(path) => {
                // Whatever was typed gives way to the editor, as with any F-key.
                self.shell.clear_line();
                let line = editor::command_line(&editor::editor(), self.shell.kind(), &path);
                // The panels come back when the editor ends, if the shell
                // reports its prompts (see `Shell::prompts`); else Ctrl+O does.
                if self.panels_visible && self.shell.kind() != cwd::Kind::Other {
                    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    self.editing = Some(Editing { prompts: self.shell.prompts(), name });
                }
                self.shell.run(&line);
                self.panels_visible = false;
            }
            Request::Copy(sources, other) => {
                let label = copy_label("Copy", &sources);
                let dialog = Dialog::copy_or_move(Kind::Copy, label, &default_dest(&sources, &other), Action::Copy(sources));
                self.overlay = Overlay::Dialog(dialog);
            }
            Request::Move(sources, other) => {
                let label = copy_label("Rename or move", &sources);
                let dialog = Dialog::copy_or_move(Kind::Move, label, &default_dest(&sources, &other), Action::Move(sources));
                self.overlay = Overlay::Dialog(dialog);
            }
            Request::MkDir => self.overlay = Overlay::Dialog(Dialog::mkdir()),
            Request::Delete(sources) => {
                let (question, names) = delete_question(&sources, "delete");
                let single = sources.len() == 1;
                self.overlay = Overlay::Dialog(Dialog::delete(question, names, single, Action::Delete(sources)));
            }
            Request::Wipe(sources) => {
                let (question, names) = delete_question(&sources, "wipe");
                let single = sources.len() == 1;
                self.overlay = Overlay::Dialog(Dialog::delete(question, names, single, Action::Wipe(sources)));
            }
            Request::Quit => {
                self.overlay = Overlay::Dialog(Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit));
            }
            Request::ChangeDrive { right } => {
                let list = drives::DriveMenu::new(right, self.panels.dir(right));
                self.overlay = Overlay::Drives(list);
            }
            Request::SortMenu => self.overlay = Overlay::Sort(self.panels.sort_menu()),
            Request::Error(text) => self.overlay = Overlay::Dialog(Dialog::error(&text)),
            Request::Attributes(sources) => self.open_attributes(sources),
            Request::Extract(pick, dest) => self.overlay = Overlay::Dialog(arcdialog::extract(pick, &dest)),
            Request::ViewInArchive(pick) => {
                let dir = std::env::temp_dir().join(format!("farsh-view-{}", std::process::id()));
                let mut quiet = Unattended;
                let file = pick.items[0].clone();
                let name = file.rsplit('/').next().unwrap_or(&file).to_string();
                let view = &pick.view;
                let done = archive::extract(&view.file, &view.listing, &[file], &view.dir, &dir, view.password.as_deref(), archive::Overwrite::Overwrite, &mut quiet);
                self.overlay = match done.map(|()| Viewer::open(&dir.join(&name))) {
                    Ok(Ok(viewer)) => Overlay::Viewer(viewer),
                    Ok(Err(error)) | Err(error) => Overlay::Dialog(Dialog::error(&format!("{name}: {error}"))),
                };
            }
            Request::CreateArchive(sources) => {
                let base = self.panels.active_dir().to_path_buf();
                self.overlay = Overlay::Dialog(arcdialog::create_archive(sources, &base));
            }
            Request::Menu => {
                let side = if self.panels.right_active() { 4 } else { 0 };
                self.overlay = Overlay::Menu(Menu::new(self.last_menu.unwrap_or(side), self.panels.view_modes()));
            }
        }
    }

    /// The menu was used: keeps it open, closes it, or does the chosen item.
    fn after_menu(&mut self, menu: Menu, outcome: menu::Outcome) {
        self.last_menu = Some(menu.current());
        let command = match outcome {
            menu::Outcome::Open => return self.overlay = Overlay::Menu(menu),
            menu::Outcome::Closed => return,
            menu::Outcome::Chosen(command) => command,
        };
        match command {
            menu::Command::FKey(n) => {
                if let Key::Ask(request) = self.panels.function_key(n) {
                    self.request(request);
                }
            }
            menu::Command::Wipe => {
                if let Key::Ask(request) = self.panels.wipe_request() {
                    self.request(request);
                }
            }
            menu::Command::Attributes => {
                if let Key::Ask(request) = self.panels.attributes_request() {
                    self.request(request);
                }
            }
            menu::Command::Reread => self.panels.refresh(None),
            menu::Command::SwapPanels => self.panels.swap(),
            menu::Command::TogglePanels => self.panels_visible = !self.panels_visible,
            menu::Command::InvertSelection => self.panels.invert_marks(),
            menu::Command::ChangeDrive { right } => self.request(Request::ChangeDrive { right }),
            menu::Command::View { right, mode } => self.panels.set_view(right, mode),
            menu::Command::SortModes { right } => {
                self.panels.activate(right);
                self.request(Request::SortMenu);
            }
        }
    }

    /// The drive list was used: keeps it open, closes it, or takes its panel there.
    fn after_sort(&mut self, menu: sortmenu::SortMenu, outcome: sortmenu::Outcome) {
        match outcome {
            sortmenu::Outcome::Open => self.overlay = Overlay::Sort(menu),
            sortmenu::Outcome::Closed => {}
            sortmenu::Outcome::Picked(picked) => self.panels.apply_sort(picked),
        }
    }

    fn after_drives(&mut self, list: drives::DriveMenu, outcome: drives::Outcome) {
        match outcome {
            drives::Outcome::Open => self.overlay = Overlay::Drives(list),
            drives::Outcome::Closed => {}
            drives::Outcome::Chosen(dir) => {
                self.panels.open_dir(list.right, dir);
                if self.shell.command_line_is_empty() {
                    self.shell.cd(self.panels.active_dir());
                }
            }
        }
    }

    /// A button was pressed in a dialog.
    fn accept(&mut self, action: Action, id: &str, values: &dialog::Values) {
        match action {
            Action::Extract(setup) => {
                let pick = setup.pick.clone();
                let values = values.clone();
                let base = self.panels.active_dir().to_path_buf();
                let work: fileops::Work = Box::new(move |reporter| unpack(&pick, &values, &base, reporter));
                self.overlay = Overlay::Job(fileops::spawn(Kind::Unpack, work), None);
            }
            Action::CreateArchive(setup) => {
                let Some(options) = setup.options.clone() else { return };
                let sources = setup.sources.clone();
                let work: fileops::Work = Box::new(move |reporter| pack(&options, &sources, reporter));
                self.overlay = Overlay::Job(fileops::spawn(Kind::Pack, work), None);
            }
            Action::Attributes(setup) if id == "system" => {
                if let Err(error) = attributes::system_properties(&setup.paths[0]) {
                    self.overlay = Overlay::Dialog(Dialog::error(&error));
                }
            }
            Action::Attributes(setup) => {
                let change = attribute_change(&setup, values);
                let subfolders = values.choice("subfolders") == 1;
                for path in &setup.paths {
                    if let Err(error) = attributes::apply_tree(path, &change, subfolders) {
                        self.overlay = Overlay::Dialog(Dialog::error(&error));
                        break;
                    }
                }
                self.panels.refresh(None);
            }
            Action::Copy(sources) => self.start_job(Kind::Copy, &sources, values),
            Action::Move(sources) => self.start_job(Kind::Move, &sources, values),
            Action::Delete(sources) => {
                let items = sources.into_iter().map(|path| (path.clone(), path)).collect();
                self.overlay = Overlay::Job(fileops::start(Kind::Delete, items, Options::default()), None);
            }
            Action::Wipe(sources) => {
                let items = sources.into_iter().map(|path| (path.clone(), path)).collect();
                self.overlay = Overlay::Job(fileops::start(Kind::Wipe, items, Options::default()), None);
            }
            Action::MkDir => {
                let typed = values.text("name");
                let names: Vec<&str> = if values.checked("multiple") {
                    typed.split([';', ',']).map(str::trim).filter(|n| !n.is_empty()).collect()
                } else {
                    vec![typed.trim()].into_iter().filter(|n| !n.is_empty()).collect()
                };
                let mut first = None;
                for name in names {
                    if let Err(error) = std::fs::create_dir_all(self.panels.active_dir().join(name)) {
                        self.overlay = Overlay::Dialog(Dialog::error(&format!("{name}: {error}")));
                        break;
                    }
                    first.get_or_insert_with(|| name.split(['/', '\\']).find(|part| !part.is_empty()).unwrap_or(name).to_string());
                }
                if let Some(first) = first {
                    self.panels.refresh(Some(&first));
                }
            }
            Action::Quit => self.quit = true,
            Action::Conflict | Action::Info => {}
        }
    }

    /// Ctrl+A over the panels: the attributes of what is selected (or under the cursor).
    fn open_attributes(&mut self, sources: Vec<PathBuf>) {
        self.overlay = match attributes::Setup::new(sources) {
            Ok(setup) => Overlay::Dialog(Dialog::attributes(setup)),
            Err(error) => Overlay::Dialog(Dialog::error(&error)),
        };
    }

    fn start_job(&mut self, kind: Kind, sources: &[PathBuf], values: &dialog::Values) {
        let options = Options {
            existing: match values.choice("existing") {
                1 => Existing::Overwrite,
                2 => Existing::Skip,
                _ => Existing::Ask,
            },
            timestamps: values.checked("timestamps"),
            follow_links: values.checked("links"),
            access: match values.choice("access") {
                1 => Access::Copy,
                2 => Access::Inherit,
                _ => Access::Default,
            },
        };
        let base = self.panels.active_dir().to_path_buf();
        match fileops::targets(sources, &values.text("dest"), &base, values.checked("multiple")) {
            Ok(items) => self.overlay = Overlay::Job(fileops::start(kind, items, options), None),
            Err(error) => self.overlay = Overlay::Dialog(Dialog::error(&error)),
        }
    }
}

/// What the user did with the question a job is waiting on: gives the answer to
/// the job, and hands the dialog back while it is still open.
fn answer(job: &mut Job, dialog: Dialog, outcome: dialog::Outcome) -> Option<Dialog> {
    match outcome {
        dialog::Outcome::Open => Some(dialog),
        dialog::Outcome::Closed => {
            job.answer(Answer::Cancel);
            None
        }
        dialog::Outcome::Pressed(_, id, values) => {
            let remember = values.checked("remember");
            job.answer(match id {
                "overwrite" => Answer::Overwrite { remember },
                "skip" => Answer::Skip { remember },
                "proceed" => Answer::Proceed,
                "rename" => Answer::Rename { remember },
                "append" => Answer::Append { remember },
                "all" => Answer::All,
                "skipall" => Answer::SkipAll,
                _ => Answer::Cancel,
            });
            None
        }
    }
}

/// With `FARSH_KEYLOG=<file>` set, every key event is appended to that file
/// (to see what a terminal really sends for a key).
fn log_key(key: &KeyEvent) {
    let Some(path) = std::env::var_os("FARSH_KEYLOG") else { return };
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{:?} {:?} {:?}", key.code, key.modifiers, key.kind);
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// `Copy README t&o:` or `Copy 3 items t&o:` (Far's MCopyFile / MCopyFiles).
fn copy_label(verb: &str, sources: &[PathBuf]) -> String {
    match sources {
        [one] => format!("{verb} {} t&o:", dialog::escape_ampersands(&dialog::quote_outer_space(&file_name(one)))),
        many => format!("{verb} {} item{} t&o:", many.len(), if many.len() == 1 { "" } else { "s" }),
    }
}

/// Where a copy goes at first: the other panel's folder with a separator
/// after it, or the file's own name when that folder is its own (copy.cpp).
fn default_dest(sources: &[PathBuf], other: &Path) -> String {
    if let [one] = sources {
        let same = |a: &Path, b: &Path| a == b || std::fs::canonicalize(a).ok().zip(std::fs::canonicalize(b).ok()).is_some_and(|(a, b)| a == b);
        if one.parent().is_some_and(|parent| same(parent, other)) {
            return file_name(one);
        }
    }
    let mut dest = other.display().to_string();
    if !dest.ends_with(std::path::MAIN_SEPARATOR) && !dest.ends_with('/') {
        dest.push(std::path::MAIN_SEPARATOR);
    }
    dest
}

/// Delete's question and the names under it (delete.cpp): "the file" or
/// "the folder" and its name, and where a link points; for several items
/// the first ten names and "…and N more".
fn delete_question(sources: &[PathBuf], verb: &str) -> (String, Vec<String>) {
    const SHOWN: usize = 10;
    match sources {
        [one] => {
            let link = std::fs::symlink_metadata(one).is_ok_and(|m| m.file_type().is_symlink());
            let what = if one.is_dir() { "the folder" } else { "the file" };
            let mut names = vec![dialog::quote_outer_space(&file_name(one))];
            if link {
                if let Ok(target) = std::fs::read_link(one) {
                    let target = target.display().to_string();
                    let target = target.strip_prefix(r"\\?\").or_else(|| target.strip_prefix(r"\??\")).unwrap_or(&target).to_string();
                    names.push("a link to".into());
                    names.push(target);
                }
            }
            (format!("Do you wish to {verb} {what}?"), names)
        }
        many => {
            let shown = if many.len() > SHOWN + 1 { SHOWN } else { many.len() };
            let mut names: Vec<String> = many[..shown].iter().map(|p| dialog::quote_outer_space(&file_name(p))).collect();
            if shown < many.len() {
                names.push(format!("\u{2026}and {} more", many.len() - shown));
            }
            (format!("Do you wish to {verb} the objects?"), names)
        }
    }
}

/// Extracts for F3 in an archive: nothing to ask, nothing to show.
struct Unattended;

impl archive::Progress for Unattended {
    fn file(&mut self, _: &archive::Entry) -> std::io::Result<()> {
        Ok(())
    }
    fn bytes(&mut self, _: u64) -> std::io::Result<()> {
        Ok(())
    }
}

impl archive::Extracting for Unattended {
    fn exists(&mut self, _: &Path, _: &archive::Item) -> std::io::Result<archive::Overwrite> {
        Ok(archive::Overwrite::Overwrite)
    }
}

/// What Extract files asked (arclite's ExtractOptions and plugin.cpp's
/// extract): where to, into a folder of the archive's name if asked, what
/// to do with a file that is there, then deleting the archive if asked.
fn unpack(pick: &panels::ArchivePick, values: &dialog::Values, base: &Path, reporter: &mut fileops::Reporter) -> std::io::Result<()> {
    struct Report<'a, 'b> {
        reporter: &'a mut fileops::Reporter<'b>,
        remembered: Option<archive::Overwrite>,
    }
    impl archive::Progress for Report<'_, '_> {
        fn file(&mut self, entry: &archive::Entry) -> std::io::Result<()> {
            let progress = self.reporter.progress();
            progress.from = entry.name.clone();
            progress.file_size = entry.size;
            progress.file_done = 0;
            progress.files_done += 1;
            self.reporter.tick()
        }
        fn bytes(&mut self, count: u64) -> std::io::Result<()> {
            let progress = self.reporter.progress();
            progress.file_done += count;
            progress.bytes_done += count;
            self.reporter.tick()
        }
    }
    impl archive::Extracting for Report<'_, '_> {
        fn exists(&mut self, target: &Path, item: &archive::Item) -> std::io::Result<archive::Overwrite> {
            if let Some(choice) = self.remembered {
                return Ok(choice);
            }
            let old = std::fs::metadata(target).ok();
            let question = fileops::Question::Exists {
                path: target.to_path_buf(),
                new: (item.size, item.modified),
                old: (old.as_ref().map_or(0, |m| m.len()), old.and_then(|m| m.modified().ok())),
            };
            let (choice, remember) = match self.reporter.ask(question)? {
                Answer::Overwrite { remember } => (archive::Overwrite::Overwrite, remember),
                Answer::Skip { remember } => (archive::Overwrite::Skip, remember),
                Answer::Rename { remember } => (archive::Overwrite::Rename, remember),
                Answer::Append { remember } => (archive::Overwrite::Append, remember),
                _ => return Err(std::io::Error::other("Cancelled")),
            };
            if remember {
                self.remembered = Some(choice);
            }
            Ok(choice)
        }
    }
    let typed = values.text("dest").trim().replace('"', "");
    let mut dest = base.join(if typed.is_empty() { "." } else { &typed });
    // Ex&tract archive into separate directory: on (F5 is not OPM_TOPLEVEL, so [?] is off)
    if values.choice("separatedir") == 1 {
        let stem = pick.view.file.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        dest = archive::auto_rename(&dest.join(stem));
    }
    let policy = match values.choice("overwrite") {
        1 => archive::Overwrite::Overwrite,
        2 => archive::Overwrite::Skip,
        3 => archive::Overwrite::Rename,
        4 => archive::Overwrite::Append,
        _ => archive::Overwrite::Ask,
    };
    let typed_password = values.text("password");
    let password = if typed_password.is_empty() { pick.view.password.clone() } else { Some(typed_password) };
    let view = &pick.view;
    let wanted: Vec<&archive::Item> = view.listing.items.iter().filter(|i| pick.items.iter().any(|w| i.path == *w || i.path.starts_with(&format!("{w}/")))).collect();
    let progress = reporter.progress();
    progress.to = view.file.display().to_string();
    progress.bytes_total = wanted.iter().map(|i| i.size).sum();
    progress.files_total = wanted.iter().filter(|i| !i.is_dir).count();
    let mut report = Report { reporter, remembered: None };
    archive::extract(&view.file, &view.listing, &pick.items, &view.dir, &dest, password.as_deref(), policy, &mut report)?;
    if values.checked("deletearchive") || values.choice("deletearchive") == 1 {
        std::fs::remove_file(&view.file)?;
    }
    Ok(())
}

/// Makes the archive, telling the progress box (arclite's ArchiveUpdateProgress) as it goes.
fn pack(options: &archive::Options, sources: &[PathBuf], reporter: &mut fileops::Reporter) -> std::io::Result<()> {
    struct Report<'a, 'b>(&'a mut fileops::Reporter<'b>);
    impl archive::Progress for Report<'_, '_> {
        fn file(&mut self, entry: &archive::Entry) -> std::io::Result<()> {
            let progress = self.0.progress();
            progress.from = entry.path.display().to_string();
            progress.file_size = entry.size;
            progress.file_done = 0;
            if !entry.is_dir {
                progress.files_done += 1;
            }
            self.0.tick()
        }
        fn bytes(&mut self, count: u64) -> std::io::Result<()> {
            let progress = self.0.progress();
            progress.file_done += count;
            progress.bytes_done += count;
            self.0.tick()
        }
    }
    let entries = archive::entries(sources)?;
    let progress = reporter.progress();
    progress.to = options.path.display().to_string();
    progress.bytes_total = entries.iter().map(|e| e.size).sum();
    progress.files_total = entries.iter().filter(|e| !e.is_dir).count();
    archive::create(options, sources, &mut Report(reporter))
}

/// What Set in the Attributes dialog asks for (setattr.cpp): a check box on
/// sets, off clears, "?" leaves; a date or time field the user changed is
/// set; an owner typed in is given.
fn attribute_change(setup: &attributes::Setup, values: &dialog::Values) -> attributes::Change {
    let mut change = attributes::Change { format: setup.format, ..attributes::Change::default() };
    for i in 0..20 {
        let state = if i < 10 { values.choice(attributes::MAIN[i].1) as u8 } else { setup.current[i] };
        let attribute = attributes::Setup::attribute(i);
        if !attributes::settable(attribute) {
            continue;
        }
        match state {
            1 => change.set |= attribute,
            0 => change.clear |= attribute,
            _ => {}
        }
    }
    let (date_mask, _) = setup.format.date_mask();
    let (time_mask, _) = setup.format.time_mask();
    for (k, (date_id, time_id)) in dialog::TIME_IDS.iter().enumerate() {
        let typed = (values.text(date_id), values.text(time_id));
        let opened = (dialog::masked_text(&date_mask, &setup.dates[k].0), dialog::masked_text(&time_mask, &setup.dates[k].1));
        if typed != opened {
            change.times[k] = Some(typed);
        }
    }
    let owner = values.text("owner");
    if !owner.is_empty() && owner != setup.owner {
        change.owner = Some(owner);
    }
    change
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_in_an_untouched_attributes_dialog_changes_nothing_and_a_toggle_is_applied() {
        let dir = std::env::temp_dir().join(format!("farsh-attrset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let before = attributes::read(&file).unwrap();
        let mut dialog = Dialog::attributes(attributes::Setup::new(vec![file.clone()]).unwrap());
        let setup = attributes::Setup::new(vec![file.clone()]).unwrap();
        let change = attribute_change(&setup, &dialog.values());
        assert!(change.times.iter().all(Option::is_none), "{change:?}");
        assert_eq!(change.owner, None);
        attributes::apply(&file, &change).unwrap();
        assert_eq!(attributes::read(&file).unwrap(), before, "nothing moved");

        dialog.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT));
        let dialog::Outcome::Pressed(_, "ok", values) = dialog.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) else { panic!("Set") };
        attributes::apply(&file, &attribute_change(&setup, &values)).unwrap();
        assert!(attributes::read(&file).unwrap().attributes & attributes::READONLY != 0);
        attributes::apply(&file, &attributes::Change { clear: attributes::READONLY, ..attributes::Change::default() }).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn extracting_asks_overwrite_file_and_rename_keeps_both() {
        let dir = std::env::temp_dir().join(format!("farsh-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("out")).unwrap();
        std::fs::write(dir.join("a.txt"), "new").unwrap();
        std::fs::write(dir.join("out").join("a.txt"), "old").unwrap();
        let options = archive::Options {
            path: dir.join("x.zip"),
            format: archive::Format::Zip,
            level: 5,
            method: "Deflate".into(),
            solid: false,
            password: None,
            encrypt_header: false,
            volume_size: None,
            move_files: false,
            ignore_errors: false,
        };
        archive::create(&options, &[dir.join("a.txt")], &mut Unattended).unwrap();
        let listing = archive::list(&options.path, None).unwrap();
        let view = panels::ArchiveView { file: options.path.clone(), listing, dir: String::new(), password: None };
        let pick = panels::ArchivePick { view, items: vec!["a.txt".into()] };
        let mut dialog = arcdialog::extract(pick.clone(), &dir.join("out"));
        let dialog::Outcome::Pressed(_, "ok", values) = dialog.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) else { panic!("OK") };
        let base = dir.clone();
        let mut job = fileops::spawn(Kind::Unpack, Box::new(move |reporter| unpack(&pick, &values, &base, reporter)));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut asked = 0;
        while job.result.is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            job.poll();
            if let Some(fileops::Question::Exists { path, new, old }) = job.question.clone() {
                assert_eq!(path, dir.join("out").join("a.txt"));
                assert_eq!((new.0, old.0), (3, 3));
                asked += 1;
                job.answer(Answer::Rename { remember: false });
            }
        }
        assert_eq!(job.result, Some(Ok(())));
        assert_eq!(asked, 1);
        assert_eq!(std::fs::read_to_string(dir.join("out").join("a.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(dir.join("out").join("a.txt.0")).unwrap(), "new");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
