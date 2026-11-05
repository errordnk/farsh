//! farsh: the user's own shell (bash / zsh / PowerShell) in a PTY, with Far
//! Manager style panels that Ctrl+O shows and hides.

mod arcdialog;
mod archive;
mod assoc;
mod attributes;
mod builder;
mod codepages;
mod colordata;
mod colorpick;
mod colors;
mod commands;
mod cwd;
mod dialog;
mod elevation;
mod diz;
mod drives;
mod editor;
mod fileops;
mod filter;
mod findfile;
mod grid;
mod help;
mod highlight;
mod history;
mod hotplug;
mod infopanel;
mod keys;
mod lang;
mod links;
mod listing;
mod masks;
mod farconfig;
mod farconfig_data;
mod menu;
mod modkeys;
mod optmenu;
mod options;
mod mouse;
mod login;
mod srp;
mod fastfind;
#[cfg(unix)]
mod rawinput;
#[cfg(windows)]
mod rawkeys;
mod palette;
mod panels;
mod profile;
mod qview;
mod shell;
mod shortcuts;
mod sorting;
mod settings;
mod sortmenu;
mod subst;
mod tasklist;
mod usermenu;
mod viewer;
mod vmenu;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
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
    // the copy of farsh UAC started to do what needs administrator rights
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--elevation") {
        return Ok(elevation::serve(&args[1..])?);
    }
    // As the user's shell (/etc/shells, passwd): only an interactive session
    // on a terminal is farsh's; `-c cmd`, a script, a pipe is the real shell's.
    #[cfg(unix)]
    {
        let argv0 = std::env::args().next().unwrap_or_default();
        let login = login::is_login(&argv0, &args);
        let tty = unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 };
        if login::wants_plain_shell(&args) || !tty {
            let error = login::exec_real_shell(login, &args);
            eprintln!("farsh: cannot start {}: {error}", shell::shell_program());
            std::process::exit(127);
        }
        login::set_login(login);
        if login {
            login::install_panic_fallback();
        }
    }
    let (cols, rows) = terminal::size()?;
    terminal::enable_raw_mode()?;
    let output_mode = vt_output();
    // The shell starts where the terminal's cursor is (its ConPTY asks).
    let (col, row) = crossterm::cursor::position().unwrap_or((0, 0));
    shell::set_start_cursor(col, row.min(rows.saturating_sub(1)));
    let mut app = match App::new(cols, rows) {
        Ok(app) => app,
        Err(error) => {
            restore_output(output_mode);
            let _ = terminal::disable_raw_mode();
            // a login shell that cannot start farsh's session is still a shell
            #[cfg(unix)]
            if login::login() {
                eprintln!("farsh: {error:#}");
                let error = login::exec_real_shell(true, &[]);
                eprintln!("farsh: cannot start {}: {error}", shell::shell_program());
            }
            return Err(error);
        }
    };
    let mut out = std::io::stdout();
    // Without panels the shell owns the terminal's main screen: its output
    // goes there as it wrote it. The panels are on the alternate screen
    // (`App::sync_screen`), the mouse is taken only while farsh needs it
    // (`App::sync_mouse`).
    let result = app.run(cols, rows);
    if app.mouse == Some(true) {
        let _ = mouse_capture(false);
    }
    let _ = app.finish(&mut out);
    crossterm::execute!(out, crossterm::cursor::Show)?;
    restore_output(output_mode);
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

/// Windows: the console's output understands the shell's escape sequences,
/// which are passed on as they are. Returns the mode it had, to put back.
#[cfg(windows)]
fn vt_output() -> Option<u32> {
    use windows_sys::Win32::System::Console::{
        ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_OUTPUT_HANDLE,
        SetConsoleMode,
    };
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return None;
        }
        SetConsoleMode(handle, mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        Some(mode)
    }
}

#[cfg(windows)]
fn restore_output(mode: Option<u32>) {
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_OUTPUT_HANDLE, SetConsoleMode};
    if let Some(mode) = mode {
        unsafe { SetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), mode) };
    }
}

#[cfg(not(windows))]
fn vt_output() -> Option<u32> {
    None
}

#[cfg(not(windows))]
fn restore_output(_: Option<u32>) {}

/// What the shell may write while the panels are up before the main screen is
/// redrawn from farsh's copy of it instead.
const PENDING_MAX: usize = 16 * 1024 * 1024;

/// Takes the mouse from the terminal (`on`) or gives it back, so that it
/// selects text itself. On Windows only the console's mouse flag changes:
/// crossterm's DisableMouseCapture would put back the mode it found, without
/// ENABLE_WINDOW_INPUT, and the resizes would be lost. Without the mouse a
/// plain console gets its QuickEdit selection back; ConPTY tells its terminal
/// to stop reporting the mouse.
#[cfg(windows)]
fn mouse_capture(on: bool) -> std::io::Result<()> {
    use windows_sys::Win32::System::Console::{
        ENABLE_EXTENDED_FLAGS, ENABLE_MOUSE_INPUT, ENABLE_QUICK_EDIT_MODE, ENABLE_WINDOW_INPUT, GetConsoleMode, GetStdHandle,
        STD_INPUT_HANDLE, SetConsoleMode,
    };
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(input, &mut mode) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        mode |= ENABLE_EXTENDED_FLAGS | ENABLE_WINDOW_INPUT;
        if on {
            mode = (mode | ENABLE_MOUSE_INPUT) & !ENABLE_QUICK_EDIT_MODE;
        } else {
            mode = (mode | ENABLE_QUICK_EDIT_MODE) & !ENABLE_MOUSE_INPUT;
        }
        if SetConsoleMode(input, mode) == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn mouse_capture(on: bool) -> std::io::Result<()> {
    if on {
        crossterm::execute!(std::io::stdout(), event::EnableMouseCapture)
    } else {
        crossterm::execute!(std::io::stdout(), event::DisableMouseCapture)
    }
}

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
    /// Shift+F3: arclite's menu for these archives.
    ArcMenu(vmenu::VMenu, Vec<panels::ArchivePick>),
    /// A list of the Commands menu (a history, the user menu, the filters...).
    List(Box<commands::List>),
    /// Alt+F7's results.
    Find(Box<findfile::Results>),
    /// Options > Colors: the color dialog.
    ColorPick(Box<colorpick::Picker>),
    /// far:config, typed on the command line.
    FarConfig(Box<farconfig::FarConfig>),
    /// F1: Far's help.
    Help(Box<help::Help>),
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
    /// Where the cursor goes when the running job is done (a folder made in a zip).
    select_after_job: Option<String>,
    /// What to say when the running job ends well (arclite's "Archive test complete").
    done_message: Option<(String, String)>,
    /// The last mask of Select group (Far's static strPrevMask) and the last applied command.
    last_mask: String,
    last_command: String,
    /// What a dialog or viewer on top was opened over (a list, Find's
    /// results): it comes back when that closes.
    under: Vec<Overlay>,
    /// Far's command, view/edit and folder histories.
    commands: history::History,
    views: history::History,
    folders: history::History,
    /// The active panel's folder when it was last put into the folder history.
    last_folder: PathBuf,
    /// Find file's last question, and whether the drive menu was opened from it.
    find_options: findfile::Options,
    find_after_drive: bool,
    /// Alt+F9.
    maximized: bool,
    /// The minute the clock showed last.
    clock: String,
    /// Whether the mouse is taken from the terminal (None: not set yet).
    mouse: Option<bool>,
    /// The console's input records, read for the shell (Windows).
    #[cfg(windows)]
    raw: rawkeys::Reader,
    /// What the terminal sent after a Ctrl+O that opened the panels, for the
    /// shell when they are gone (unix).
    #[cfg(unix)]
    held_input: Vec<u8>,
    /// Whether the terminal is on its alternate screen, where the panels are.
    on_alt: bool,
    /// What the shell wrote while the panels were up, for the main screen.
    pending: Vec<u8>,
    /// More came than `PENDING_MAX`: the screen is redrawn from farsh's copy.
    pending_lost: bool,
    /// A highlighting group whose color dialog is open, and the part a new one goes to.
    hl_pending: Option<(highlight::Group, usize, bool)>,
    hl_section: u8,
    /// Viewers sent to the background (F12, the screens list).
    screens: Vec<Viewer>,
}

/// A file being edited in the shell (F4), started at the shell's prompt
/// number `prompts`.
struct Editing {
    /// The prompt number the shell reaches when the commands are done.
    until: u64,
    /// The file to put the cursor on then.
    name: Option<String>,
}

impl App {
    fn new(cols: u16, rows: u16) -> Result<Self> {
        lang::load();
        sorting::load_options();
        Ok(Self {
            shell: Shell::spawn(cols, rows)?,
            panels: {
                let mut panels = Panels::new(std::env::current_dir()?);
                panels.restore_state();
                panels
            },
            panels_visible: false,
            overlay: Overlay::None,
            quit: false,
            last_click: None,
            last_menu: None,
            size: (cols, rows),
            editing: None,
            select_after_job: None,
            done_message: None,
            last_mask: "*.*".into(),
            last_command: String::new(),
            under: Vec::new(),
            commands: history::History::load(history::Which::Commands),
            views: history::History::load(history::Which::Views),
            folders: history::History::load(history::Which::Folders),
            last_folder: PathBuf::new(),
            find_options: findfile::Options::default(),
            find_after_drive: false,
            maximized: false,
            clock: String::new(),
            mouse: None,
            #[cfg(windows)]
            raw: rawkeys::Reader::default(),
            #[cfg(unix)]
            held_input: Vec::new(),
            on_alt: false,
            pending: Vec::new(),
            pending_lost: false,
            hl_pending: None,
            hl_section: 0,
            screens: Vec::new(),
        })
    }

    fn run(&mut self, cols: u16, rows: u16) -> Result<()> {
        let mut out = std::io::stdout();
        // the main screen shows what `main` scrolled away: not known
        let mut screen = Grid::unknown(cols, rows);
        let mut frame = Grid::new(cols, rows);
        let mut dirty = true;
        // a debug build can be made to fail, to see a login session survive it
        #[cfg(debug_assertions)]
        if std::env::var_os("FARSH_DEBUG_PANIC").is_some() {
            panic!("FARSH_DEBUG_PANIC");
        }

        while !self.quit {
            if self.shell.pump() {
                dirty = true;
            }
            if self.shell.exited() {
                self.shell.drain_after_exit();
                self.sync_screen(&mut screen, &mut out)?;
                return Ok(());
            }
            // The shell changed directory (a typed `cd`, a script): the panel follows.
            if let Some(dir) = self.shell.take_cwd() {
                self.panels.go_to(dir);
                dirty = true;
            }
            // The editor F4 started has ended (the shell is at its next prompt):
            // the panels are back as they were, on the file edited, as in Far.
            if self.editing.as_ref().is_some_and(|editing| self.shell.prompts() >= editing.until)
                && let Some(editing) = self.editing.take()
            {
                self.panels_visible = true;
                self.panels.refresh(editing.name.as_deref());
                dirty = true;
            }
            if self.poll_job() {
                dirty = true;
            }
            if let Overlay::Find(results) = &mut self.overlay
                && results.poll()
            {
                dirty = true;
            }
            self.after_event();
            self.sync_mouse();
            if self.sync_screen(&mut screen, &mut out)? {
                dirty = true;
            }
            // the clock moves on
            let minute = chrono::Local::now().format("%H:%M").to_string();
            if self.panels_visible && minute != self.clock {
                self.clock = minute;
                dirty = true;
            }
            if dirty {
                // the panels are drawn on the alternate screen; the main one is the shell's
                if self.on_alt {
                    frame.clear();
                    let cursor = self.paint(&mut frame);
                    frame.flush(&mut screen, cursor, &mut out)?;
                    out.flush()?;
                }
                dirty = false;
            }
            // A copy in progress, or a folder the quick view is counting, redraws as it goes.
            let busy = matches!(self.overlay, Overlay::Job(..))
                || matches!(&self.overlay, Overlay::Viewer(viewer) if viewer.busy())
                || matches!(&self.overlay, Overlay::Find(results) if results.busy())
                || (self.panels_visible && self.panels.busy());
            let wait = if busy { 50 } else { 10 };
            // Without panels what the terminal sends goes to the shell as it is.
            #[cfg(any(windows, unix))]
            if !self.on_alt {
                if let Some((cols, rows)) = self.read_for_shell()? {
                    self.size = (cols, rows);
                    self.shell.resize(cols, rows);
                    screen = Grid::unknown(cols, rows);
                    frame = Grid::new(cols, rows);
                    dirty = true;
                }
                continue;
            }
            // Som resizes the pseudo-console without always sending the resize
            // record (seen: the screen stayed at the old size until a key came),
            // so the size is also looked at here.
            if let Some((cols, rows)) = terminal::size().ok().filter(|size| *size != self.size) {
                self.size = (cols, rows);
                self.shell.resize(cols, rows);
                screen = Grid::unknown(cols, rows);
                frame = Grid::new(cols, rows);
                dirty = true;
            }
            // Shift/Ctrl/Alt on their own change the key bar
            let (input, held_changed) = modkeys::wait(Duration::from_millis(wait))?;
            if held_changed && self.panels_visible {
                dirty = true;
            }
            if !input || !event::poll(Duration::ZERO)? {
                dirty |= busy;
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
        // far:config stays in sight under a value's dialog, as in Far
        for under in &mut self.under {
            match under {
                Overlay::FarConfig(config) => config.paint(frame),
                // the help stays in sight under its search dialog and messages
                Overlay::Help(help) => help.paint(frame),
                _ => {}
            }
        }
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
            Overlay::ArcMenu(menu, _) => {
                menu.paint(frame);
                cursor = None;
            }
            Overlay::List(list) => {
                list.paint(frame);
                cursor = None;
            }
            Overlay::Find(results) => {
                results.paint(frame);
                cursor = None;
            }
            Overlay::ColorPick(picker) => {
                picker.paint(frame);
                cursor = None;
            }
            Overlay::FarConfig(config) => {
                config.paint(frame);
                cursor = None;
            }
            Overlay::Help(help) => {
                help.paint(frame);
                cursor = None;
            }
            Overlay::Job(job, question) => {
                dialog::paint_progress(job, frame);
                cursor = question.as_mut().and_then(|dialog| dialog.paint(frame));
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
                    fileops::Question::Exists { path, new, old, update } => arcdialog::overwrite(path, *new, *old, *update),
                    fileops::Question::Elevate { why, path } => Dialog::elevation(*why, path),
                });
                return true;
            }
        }
        let Some(result) = job.result.clone() else { return false };
        let message = self.done_message.take();
        self.overlay = match (result, message) {
            (Err(error), _) if error != "Cancelled" => Overlay::Dialog(Dialog::error(&error)),
            (Ok(()), Some((title, text))) => Overlay::Dialog(Dialog::info(&title, &text)),
            _ => Overlay::None,
        };
        let select = self.select_after_job.take();
        self.panels.refresh(select.as_deref());
        true
    }

    fn on_key(&mut self, key: KeyEvent) {
        let key = keys::normalize(key);
        // farsh's keys have one modifier at most: a chord is none of them.
        // Over the panels it is the shell's; in a window it does nothing.
        let key = match keys::chord(key) {
            keys::Chord::Text(key) | keys::Chord::Single(key) => key,
            keys::Chord::Many(key) => {
                if matches!(self.overlay, Overlay::None) {
                    self.send_to_shell(key);
                }
                return;
            }
        };
        match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::Dialog(mut dialog) => match dialog.key(key) {
                dialog::Outcome::Open => self.overlay = Overlay::Dialog(dialog),
                dialog::Outcome::Closed => {}
                dialog::Outcome::Pressed(action, id, values) => self.accept(action, id, &values),
            },
            // F12 in a viewer: the screens list (Far's manager key)
            Overlay::Viewer(viewer) if key.code == KeyCode::F(12) && key.modifiers.is_empty() => self.screens_from_viewer(viewer),
            Overlay::Viewer(mut viewer) => {
                if matches!(viewer.key(key), viewer::Outcome::Open) {
                    self.overlay = Overlay::Viewer(viewer);
                }
            }
            Overlay::List(list) => self.list_key(*list, key),
            Overlay::ColorPick(mut picker) => {
                let outcome = picker.key(key);
                self.color_pick_outcome(picker, outcome);
            }
            Overlay::Help(mut help) => {
                let outcome = help.key(key);
                self.help_outcome(help, outcome);
            }
            Overlay::FarConfig(mut config) => match config.key(key) {
                farconfig::Outcome::Open => self.overlay = Overlay::FarConfig(config),
                farconfig::Outcome::Closed => {}
                farconfig::Outcome::Changed => {
                    self.far_config_changed();
                    self.overlay = Overlay::FarConfig(config);
                }
                farconfig::Outcome::Edit(dialog) => {
                    self.under.push(Overlay::FarConfig(config));
                    self.overlay = Overlay::Dialog(dialog);
                }
                farconfig::Outcome::Help(topics) => {
                    let (cols, rows) = self.size;
                    let help = topics.iter().find_map(|topic| help::Help::found(topic, cols, rows));
                    self.overlay = Overlay::FarConfig(config);
                    if let Some(help) = help {
                        self.under.push(std::mem::replace(&mut self.overlay, Overlay::Help(Box::new(help))));
                    }
                }
            },
            Overlay::Find(mut results) => {
                let outcome = results.key(key);
                self.find_outcome(results, outcome);
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
            Overlay::ArcMenu(mut menu, picks) => match menu.key(key) {
                vmenu::Outcome::Open | vmenu::Outcome::Key(_) => self.overlay = Overlay::ArcMenu(menu, picks),
                vmenu::Outcome::Closed => {}
                vmenu::Outcome::Chosen(_) => self.test_archives(picks),
            },
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
            Overlay::List(list) => self.list_click(*list, event),
            Overlay::ColorPick(mut picker) => {
                let outcome = picker.click(event);
                self.color_pick_outcome(picker, outcome);
            }
            Overlay::Help(mut help) => {
                let outcome = help.mouse(event);
                self.help_outcome(help, outcome);
            }
            Overlay::FarConfig(mut config) => match config.click(event) {
                farconfig::Outcome::Closed => {}
                farconfig::Outcome::Edit(dialog) => {
                    self.under.push(Overlay::FarConfig(config));
                    self.overlay = Overlay::Dialog(dialog);
                }
                farconfig::Outcome::Changed => {
                    self.far_config_changed();
                    self.overlay = Overlay::FarConfig(config);
                }
                farconfig::Outcome::Open | farconfig::Outcome::Help(_) => self.overlay = Overlay::FarConfig(config),
            },
            Overlay::Find(mut results) => {
                let outcome = results.click(event);
                self.find_outcome(results, outcome);
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
            Overlay::ArcMenu(mut menu, picks) => match menu.click(event) {
                vmenu::Outcome::Open | vmenu::Outcome::Key(_) => self.overlay = Overlay::ArcMenu(menu, picks),
                vmenu::Outcome::Closed => {}
                vmenu::Outcome::Chosen(_) => self.test_archives(picks),
            },
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
            // Far: a click on the top line opens the menu bar, on the title under it
            Overlay::None if self.panels_visible && row == 0 && event.kind == MouseEventKind::Down(MouseButton::Left) => {
                let side = if self.panels.right_active() { 4 } else { 0 };
                let mut menu = Menu::new(self.last_menu.unwrap_or(side), self.panels.view_modes(), self.panels.states());
                let outcome = menu.click(event, self.size.0);
                self.after_menu(menu, outcome);
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

    /// The mouse is farsh's while the panels or something over them are shown,
    /// or a program in the shell asked for it (forwarded by `on_mouse`). Over
    /// the shell's own output it stays the terminal's: it selects text into
    /// the clipboard and pastes with a right click, as anywhere in Som. Over
    /// the panels the terminal selects with Shift held (Som, Windows
    /// Terminal). Far's Interface.Mouse off leaves it to the terminal always.
    fn sync_mouse(&mut self) {
        let wanted = settings::bool("Interface.Mouse")
            && (self.panels_visible
                || !matches!(self.overlay, Overlay::None)
                // Windows: the console's mouse records are turned into the reports a
                // program asked for; elsewhere its own request reaches the terminal
                || (cfg!(windows) && self.shell.mouse_mode().0 != vt100::MouseProtocolMode::None));
        if self.mouse != Some(wanted) && mouse_capture(wanted).is_ok() {
            self.mouse = Some(wanted);
        }
    }

    /// What the shell wrote goes to the terminal as it wrote it, on its main
    /// screen, where the terminal keeps its history, reflows and selects. The
    /// panels (and anything over them) are on the alternate screen; what the
    /// shell writes meanwhile waits, and goes to the main screen when it is
    /// back, so the terminal gets the same stream as without farsh. Returns
    /// whether the screen farsh draws changed.
    fn sync_screen(&mut self, screen: &mut Grid, out: &mut impl Write) -> std::io::Result<bool> {
        let want = self.panels_visible || !matches!(self.overlay, Overlay::None) || !self.under.is_empty();
        let bytes = self.shell.take_out();
        let mut changed = false;
        if want && !self.on_alt {
            out.write_all(&bytes)?;
            out.write_all(b"\x1b[?1049h")?;
            self.on_alt = true;
            *screen = Grid::unknown(screen.cols, screen.rows);
            changed = true;
        } else if !want && self.on_alt {
            self.leave_alt(out)?;
            out.write_all(&bytes)?;
            out.write_all(if self.shell.cursor_shown() { b"\x1b[?25h" } else { b"\x1b[?25l" })?;
            changed = true;
        } else if self.on_alt {
            if self.pending.len() + bytes.len() > PENDING_MAX {
                self.pending.clear();
                self.pending_lost = true;
            } else if !self.pending_lost {
                self.pending.extend_from_slice(&bytes);
            }
        } else {
            out.write_all(&bytes)?;
        }
        out.flush()?;
        Ok(changed)
    }

    /// Back to the main screen, with what the shell wrote meanwhile.
    fn leave_alt(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        out.write_all(b"\x1b[?1049l")?;
        self.on_alt = false;
        if std::mem::take(&mut self.pending_lost) {
            out.write_all(&self.shell.screen_bytes())?;
        } else {
            out.write_all(&self.pending)?;
        }
        self.pending.clear();
        Ok(())
    }

    /// farsh ends: the terminal is left on its main screen with the shell's
    /// last words, as if the shell had ended by itself.
    fn finish(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        let bytes = self.shell.take_out();
        if self.on_alt {
            self.leave_alt(out)?;
        }
        out.write_all(&bytes)?;
        out.flush()
    }

    /// A key sent to the shell, Windows' way: every console input record the
    /// terminal made goes to the shell's ConPTY as it is (see `rawkeys`),
    /// except Ctrl+O at a prompt. Returns the new size if the window changed.
    #[cfg(windows)]
    fn read_for_shell(&mut self) -> std::io::Result<Option<(u16, u16)>> {
        if !rawkeys::wait(Duration::from_millis(5))? {
            return Ok(None);
        }
        let mut resized = None;
        while let Some(record) = self.raw.read() {
            match &record {
                rawkeys::Record::Resize => {
                    if let Ok(size) = terminal::size() {
                        resized = Some(size);
                    }
                }
                rawkeys::Record::Mouse(event) => {
                    let (mode, encoding) = self.shell.mouse_mode();
                    if let Some(bytes) = mouse::encode(*event, mode, encoding) {
                        self.shell.send_raw(&bytes);
                    }
                }
                rawkeys::Record::Key { .. } => {
                    if record.is_ctrl_o_down() && self.shell.ctrl_o_is_ours() {
                        // its release, and what follows, are read as panel keys
                        self.panels_visible = true;
                        break;
                    }
                    if let Some(bytes) = record.win32_input() {
                        self.shell.send_raw(&bytes);
                    }
                    if !record.is_modifier() && matches!(&record, rawkeys::Record::Key { down: true, .. }) {
                        self.shell.note_keypress(record.is_enter_down());
                    }
                }
            }
        }
        Ok(resized)
    }

    /// The same on a Unix terminal: its bytes go to the shell as they came,
    /// except a Ctrl+O at a prompt (the bytes after it wait for the shell's
    /// screen to be back). The window's size is looked at here: crossterm,
    /// which hears SIGWINCH, is not reading.
    #[cfg(unix)]
    fn read_for_shell(&mut self) -> std::io::Result<Option<(u16, u16)>> {
        let resized = terminal::size().ok().filter(|size| *size != self.size);
        let ready = rawinput::wait(Duration::from_millis(5))?;
        if ready || !self.held_input.is_empty() {
            let mut bytes = std::mem::take(&mut self.held_input);
            // read only what poll announced: a read with nothing there waits for a key
            if ready {
                match rawinput::read()? {
                    Some(more) => bytes.extend(more),
                    None => {
                        self.quit = true;
                        return Ok(resized);
                    }
                }
            }
            if let Some((start, end)) = rawinput::find_ctrl_o(&bytes)
                && self.shell.ctrl_o_is_ours()
            {
                self.held_input = bytes.split_off(end);
                bytes.truncate(start);
                self.panels_visible = true;
            }
            if !bytes.is_empty() {
                self.shell.send_raw(&bytes);
                if !rawinput::is_report(&bytes) {
                    self.shell.note_keypress(rawinput::has_enter(&bytes));
                }
            }
        }
        Ok(resized)
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
        if key.code == KeyCode::Char('o')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && (self.panels_visible || self.shell.ctrl_o_is_ours())
        {
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
        if self.panels_visible && self.command_key(key) {
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
        self.send_to_shell(key);
    }

    /// F1: Far's help at `topic`.
    fn open_help(&mut self, topic: &str) {
        let (cols, rows) = self.size;
        self.overlay = Overlay::Help(Box::new(help::Help::new(topic, cols, rows)));
    }

    /// What the help asked for after a key or the mouse.
    fn help_outcome(&mut self, help: Box<help::Help>, outcome: help::Outcome) {
        match outcome {
            help::Outcome::Open => self.overlay = Overlay::Help(help),
            help::Outcome::Closed => {}
            help::Outcome::Dialog(dialog) => {
                self.under.push(Overlay::Help(help));
                self.overlay = Overlay::Dialog(dialog);
            }
            help::Outcome::Video => {
                self.run_command(menu::Cmd::VideoMode);
                self.overlay = Overlay::Help(help);
            }
        }
    }

    /// What depends on the settings, after far:config changed one.
    fn far_config_changed(&mut self) {
        lang::load();
        colors::reload();
        self.settings_changed();
    }

    /// Whether the command line holds Far's `far:config` (Enter opens the
    /// configuration editor instead of running it).
    fn typed_far_config(&self) -> bool {
        let typed = self.shell.typed().or_else(|| {
            let line = self.shell.cursor_line().0;
            line.rsplit_once('>').map(|(_, typed)| typed.trim().to_string())
        });
        typed.is_some_and(|t| t.eq_ignore_ascii_case("far:config"))
    }

    /// A key the shell gets, as typed on its line.
    fn send_to_shell(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Enter && key.modifiers.is_empty() && self.typed_far_config() {
            self.commands.add("far:config", history::Kind::Plain);
            self.shell.clear_line();
            self.overlay = Overlay::FarConfig(Box::new(farconfig::FarConfig::new()));
            return;
        }
        if let Some(bytes) = keys::encode(panels::to_shell(key)) {
            let runs = key.code == KeyCode::Enter;
            // Far's command history: what Enter runs from the prompt
            if runs && let Some(typed) = self.shell.typed() {
                self.commands.add(&typed, history::Kind::Plain);
            }
            self.shell.write(&bytes, runs);
            if runs {
                // the command's output is the shell's to show
                self.panels_visible = false;
            }
        }
    }

    fn request(&mut self, request: Request) {
        match request {
            // Viewer settings: an external viewer for F3 (Alt+F3 is the other one)
            Request::View(path) if settings::bool("Viewer.UseExternalViewer") && !settings::string("Viewer.ExternalViewerName").trim().is_empty() => {
                self.external_view(&path);
            }
            Request::View(path) => {
                self.views.add(&path.display().to_string(), history::Kind::View);
                self.overlay = match Viewer::open(&path) {
                    Ok(viewer) => Overlay::Viewer(viewer),
                    Err(error) => Overlay::Dialog(Dialog::error(&format!("{}: {error}", path.display()))),
                };
            }
            Request::Edit(path) => {
                self.views.add(&path.display().to_string(), history::Kind::Edit);
                // Whatever was typed gives way to the editor, as with any F-key.
                self.shell.clear_line();
                let line = editor::command_line(&editor::editor(), self.shell.kind(), &path);
                // The panels come back when the editor ends, if the shell
                // reports its prompts (see `Shell::prompts`); else Ctrl+O does.
                if self.panels_visible && self.shell.kind() != cwd::Kind::Other {
                    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    self.editing = Some(Editing { until: self.shell.prompts() + 1, name: Some(name) });
                }
                self.shell.run(&line);
                self.panels_visible = false;
            }
            // Confirmations > Copy / Move off: straight to the other panel
            Request::Copy(sources, other) if !settings::bool("Confirmations.Copy") => {
                let dest = default_dest(&sources, &other);
                self.start_job(Kind::Copy, &sources, &dialog::Values::with_text("dest", &dest));
            }
            Request::Move(sources, other) if !settings::bool("Confirmations.Move") => {
                let dest = default_dest(&sources, &other);
                self.start_job(Kind::Move, &sources, &dialog::Values::with_text("dest", &dest));
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
            // Confirmations > Delete off
            Request::Delete(sources) if !settings::bool("Confirmations.Delete") => self.accept(Action::Delete(sources), "ok", &dialog::Values::default()),
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
            // Options > Confirmations > Exit
            Request::Quit if !settings::bool("Confirmations.Exit") => self.quit(),
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
            Request::Extract(pick, dest, moving) => self.overlay = Overlay::Dialog(arcdialog::extract(pick, &dest, moving)),
            Request::AddToArchive(sources, view, moving) => {
                let base = self.panels.active_dir().to_path_buf();
                self.overlay = Overlay::Dialog(arcdialog::add_to_archive(sources, &base, view, moving));
            }
            Request::CreateDirInArchive(view) => self.overlay = Overlay::Dialog(arcdialog::create_folder(view)),
            Request::DeleteInArchive(pick) => {
                let action = Action::DeleteInArchive(Box::new(pick));
                self.overlay = Overlay::Dialog(Dialog::ok_cancel("ArcLite", "Do you want to delete selected files?", action));
            }
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
            Request::SelectGroup(add) => self.overlay = Overlay::Dialog(Dialog::select_group(add, &self.last_mask)),
            Request::Link(sources, other) => {
                let label = match sources.as_slice() {
                    [one] => format!("Create link to {} &in:", dialog::escape_ampersands(&dialog::quote_outer_space(&file_name(one)))),
                    many => format!("Create links to {} item{} &in:", many.len(), if many.len() == 1 { "" } else { "s" }),
                };
                let files_present = sources.iter().any(|path| !path.is_dir());
                self.overlay = Overlay::Dialog(Dialog::link(label, &default_dest(&sources, &other), files_present, sources));
            }
            Request::ApplyCommand(names) => {
                let dialog = Dialog::get_string("Apply command", "Enter command to process selected files", &self.last_command, Action::ApplyCommand(names));
                self.overlay = Overlay::Dialog(dialog);
            }
            Request::Describe(dir, names) => self.describe_next(dir, names),
            Request::ArchiveCommands(picks) => {
                self.overlay = Overlay::ArcMenu(vmenu::VMenu::new("ArcLite", vec!["Test archive files".into()]), picks);
            }
            Request::Command(cmd) => self.run_command(cmd),
            Request::Help => self.open_help("Contents"),
            Request::Menu => {
                let side = if self.panels.right_active() { 4 } else { 0 };
                self.overlay = Overlay::Menu(Menu::new(self.last_menu.unwrap_or(side), self.panels.view_modes(), self.panels.states()));
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
            menu::Command::Info { right } => self.panels.set_kind(right, panels::Kind::Info),
            menu::Command::QuickView { right } => self.panels.set_kind(right, panels::Kind::QuickView),
            menu::Command::LongNames { right } => self.panels.toggle_short_names(right),
            menu::Command::PanelOnOff { right } => {
                self.panels.toggle_visible(right);
            }
            menu::Command::Reread => self.panels.refresh(None),
            menu::Command::SwapPanels => self.panels.swap(),
            menu::Command::TogglePanels => self.panels_visible = !self.panels_visible,
            menu::Command::InvertSelection => self.panels.invert_marks(),
            menu::Command::SelectGroup(add) => self.request(Request::SelectGroup(add)),
            menu::Command::RestoreSelection => self.panels.restore_selection(),
            menu::Command::Ask(which) => {
                let asked = match which {
                    menu::Asked::Link => self.panels.link_request(),
                    menu::Asked::AddToArchive => self.panels.add_to_archive_request(),
                    menu::Asked::Extract => self.panels.extract_archive_request(),
                    menu::Asked::ArchiveCommands => self.panels.archive_commands_request(),
                    menu::Asked::ApplyCommand => self.panels.apply_command_request(),
                    menu::Asked::Describe => self.panels.describe_request(),
                };
                if let Key::Ask(request) = asked {
                    self.request(request);
                }
            }
            menu::Command::Do(cmd) => self.run_command(cmd),
            menu::Command::Set(opt) => self.run_option(opt),
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
            drives::Outcome::Open => return self.overlay = Overlay::Drives(list),
            drives::Outcome::Closed => {}
            drives::Outcome::Chosen(dir) => {
                self.panels.open_dir(list.right, dir);
                if self.shell.command_line_is_empty() {
                    self.shell.cd(self.panels.active_dir());
                }
            }
        }
        // Find file's Drive: the question again, on the new drive
        if std::mem::take(&mut self.find_after_drive) && matches!(self.overlay, Overlay::None) {
            self.overlay = Overlay::Dialog(findfile::dialog(&self.find_options, self.panels.active_dir()));
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
            Action::CreateDirInArchive(view) => {
                let name = values.text("name");
                if name.trim().is_empty() {
                    return;
                }
                let first = name.trim().split(['/', '\\']).find(|p| !p.is_empty()).unwrap_or_default().to_string();
                let work: fileops::Work = Box::new(move |reporter| {
                    reporter.progress().to = view.file.display().to_string();
                    archive::zip_create_dir(&view.file, &view.listing, &view.dir, &name)
                });
                self.overlay = Overlay::Job(fileops::spawn(Kind::Update, work), None);
                self.select_after_job = Some(first);
            }
            Action::DeleteInArchive(pick) => {
                let (file, items) = (pick.view.file.clone(), pick.items.clone());
                let work: fileops::Work = Box::new(move |reporter| {
                    reporter.progress().to = file.display().to_string();
                    archive::zip_delete(&file, &items)
                });
                self.overlay = Overlay::Job(fileops::spawn(Kind::Update, work), None);
            }
            Action::CreateArchive(setup) if setup.target.is_some() => {
                let (Some(options), Some(view)) = (setup.options.clone(), setup.target.clone()) else { return };
                let sources = setup.sources.clone();
                let work: fileops::Work = Box::new(move |reporter| {
                    let entries = archive::entries(&sources)?;
                    let progress = reporter.progress();
                    progress.to = view.file.display().to_string();
                    progress.bytes_total = entries.iter().map(|e| e.size).sum();
                    progress.files_total = entries.iter().filter(|e| !e.is_dir).count();
                    archive::zip_add(&view.file, &view.listing, &sources, &view.dir, &options, &mut ArcReport { reporter, remembered: None })
                });
                self.overlay = Overlay::Job(fileops::spawn(Kind::Update, work), None);
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
                let options = Options { recycle: cfg!(windows) && settings::bool("System.DeleteToRecycleBin"), ..delete_options() };
                self.overlay = Overlay::Job(fileops::start(Kind::Delete, items, options), None);
            }
            Action::Wipe(sources) => {
                let items = sources.into_iter().map(|path| (path.clone(), path)).collect();
                self.overlay = Overlay::Job(fileops::start(Kind::Wipe, items, delete_options()), None);
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
            Action::SelectGroup(add) => {
                let typed = values.text("mask");
                match masks::Masks::new(&typed) {
                    Some(masks) => {
                        self.last_mask = typed;
                        self.panels.select_group(add, &masks);
                    }
                    None => self.overlay = Overlay::Dialog(Dialog::warning_ok("Warning", "File-mask string contains errors")),
                }
            }
            Action::Link(sources) => {
                let typed = values.text("dest");
                let kind = links::LinkType::from_index(values.choice("kind"));
                let base = self.panels.active_dir().to_path_buf();
                // "Process multiple destinations": `;` or `,` between them
                let targets: Vec<String> = if values.checked("multiple") {
                    typed.split([';', ',']).map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect()
                } else {
                    vec![typed]
                };
                'all: for target in &targets {
                    for source in &sources {
                        if let Err(error) = links::create(source, &links::target(source, target, &base), kind) {
                            self.overlay = Overlay::Dialog(Dialog::error(&error));
                            break 'all;
                        }
                    }
                }
                self.panels.refresh(None);
            }
            Action::ApplyCommand(names) => {
                let typed = values.text("text");
                if typed.trim().is_empty() {
                    return;
                }
                self.last_command = typed.clone();
                let command = typed.trim_start();
                let lines: Vec<String> = names.iter().map(|name| self.substitute(command, name, &names)).filter(|line| !line.is_empty()).collect();
                if lines.is_empty() {
                    return;
                }
                self.shell.clear_line();
                // the panels come back when the last command is done (as after F4)
                if self.panels_visible && self.shell.kind() != cwd::Kind::Other {
                    self.editing = Some(Editing { until: self.shell.prompts() + lines.len() as u64, name: None });
                }
                for line in &lines {
                    self.shell.run(line);
                }
                for name in &names {
                    self.panels.unmark(name);
                }
                self.panels_visible = false;
            }
            Action::Describe(dir, mut names) => {
                let name = names.remove(0);
                let text = values.text("text");
                let mut diz = diz::Diz::read(&dir);
                if text.is_empty() { diz.erase(&name) } else { diz.set(&name, &text) }
                if let Err(error) = diz.flush() {
                    self.overlay = Overlay::Dialog(Dialog::error(&error));
                    return;
                }
                self.panels.unmark(&name);
                self.panels.refresh_descriptions();
                self.describe_next(dir, names);
            }
            Action::Quit => self.quit(),
            Action::Conflict | Action::Info => {}
            other => self.accept_command(other, id, values),
        }
    }

    /// The external viewer's command for the file, run in the shell (the
    /// panels come back after it, as after F4).
    fn external_view(&mut self, path: &Path) {
        let command = settings::string("Viewer.ExternalViewerName");
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let line = if command.contains('!') {
            self.substitute(&command, &name, std::slice::from_ref(&name))
        } else {
            format!("{} {}", command.trim(), editor::quote(self.shell.kind(), &path.display().to_string()))
        };
        self.views.add(&path.display().to_string(), history::Kind::View);
        self.shell.clear_line();
        if self.panels_visible && self.shell.kind() != cwd::Kind::Other {
            self.editing = Some(Editing { until: self.shell.prompts() + 1, name: Some(name) });
        }
        self.shell.run(&line);
        self.panels_visible = false;
    }

    /// Alt+F3: the viewer F3 does not use.
    pub(crate) fn alt_view(&mut self, path: &Path) {
        let external = !settings::string("Viewer.ExternalViewerName").trim().is_empty();
        if settings::bool("Viewer.UseExternalViewer") || !external {
            self.views.add(&path.display().to_string(), history::Kind::View);
            self.overlay = match Viewer::open(path) {
                Ok(viewer) => Overlay::Viewer(viewer),
                Err(error) => Overlay::Dialog(Dialog::error(&format!("{}: {error}", path.display()))),
            };
        } else {
            self.external_view(path);
        }
    }

    /// F10: with "Auto save setup" the panels' state is kept first.
    fn quit(&mut self) {
        if settings::bool("System.AutoSaveSetup") {
            self.save_setup();
        }
        self.quit = true;
    }

    /// After every turn of the loop: a folder the active panel came to goes
    /// into the folders history; what a closed dialog or viewer was opened
    /// over comes back (not while an editor runs in the shell).
    fn after_event(&mut self) {
        let dir = self.panels.active_dir();
        if dir != self.last_folder {
            self.last_folder = dir.to_path_buf();
            let name = dir.display().to_string();
            self.folders.add(&name, history::Kind::Plain);
        }
        if matches!(self.overlay, Overlay::None)
            && self.editing.is_none()
            && let Some(under) = self.under.pop()
        {
            self.overlay = under;
        }
    }

    /// Ctrl+Z: Far's DescribeFiles asks for each name in turn (Esc stops).
    fn describe_next(&mut self, dir: PathBuf, names: Vec<String>) {
        let Some(name) = names.first() else { return };
        let prev = diz::Diz::read(&dir).get(name).unwrap_or_default();
        let prompt = format!("Enter description for {}:", dialog::quote_outer_space(name));
        self.overlay = Overlay::Dialog(Dialog::get_string("Describe file", &prompt, &prev, Action::Describe(dir, names)));
    }

    /// Far's SubstFileName for one file of Ctrl+G.
    fn substitute(&self, command: &str, name: &str, names: &[String]) -> String {
        let (other_dir, other_name, other_short) = self.panels.passive_current();
        let this = subst::Subject { dir: self.panels.active_dir().to_path_buf(), name: name.to_string(), short: self.panels.short_name_of(name), list: names.to_vec() };
        let other = subst::Subject { dir: other_dir, name: other_name, short: other_short, list: vec![] };
        subst::substitute(command, &this, &other, self.panels.right_active())
    }

    /// arclite's "Test archive files": everything picked is read through
    /// (the checksums checked) into a folder that is removed afterwards.
    fn test_archives(&mut self, picks: Vec<panels::ArchivePick>) {
        let work: fileops::Work = Box::new(move |reporter| {
            let scratch = std::env::temp_dir().join(format!("farsh-test-{}", std::process::id()));
            let mut result = Ok(());
            for pick in &picks {
                let view = &pick.view;
                reporter.progress().to = view.file.display().to_string();
                result = archive::extract(&view.file, &view.listing, &pick.items, &view.dir, &scratch, view.password.as_deref(), archive::Overwrite::Overwrite, &mut ArcReport { reporter, remembered: None });
                let _ = std::fs::remove_dir_all(&scratch);
                if result.is_err() {
                    break;
                }
            }
            result
        });
        self.done_message = Some(("ArcLite".into(), "Archive test complete".into()));
        self.overlay = Overlay::Job(fileops::spawn(Kind::Test, work), None);
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
            ..delete_options()
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
        // Esc on "Access denied" is Skip, as in Far
        dialog::Outcome::Closed if matches!(job.question, Some(fileops::Question::Elevate { .. })) => {
            job.answer(Answer::Elevate(elevation::Approval { ok: false, for_all: true, dont_ask: false }));
            None
        }
        dialog::Outcome::Closed => {
            job.answer(Answer::Cancel);
            None
        }
        dialog::Outcome::Pressed(_, id, values) if matches!(job.question, Some(fileops::Question::Elevate { .. })) => {
            job.answer(Answer::Elevate(elevation::Approval { ok: id == "ok", for_all: values.checked("for_all"), dont_ask: values.checked("dont_ask") }));
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

/// Delete and Wipe as Options > Confirmations has them.
fn delete_options() -> Options {
    Options { confirm_folders: settings::bool("Confirmations.DeleteFolder"), confirm_read_only: settings::bool("Confirmations.RO"), ..Options::default() }
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
    fn exists(&mut self, _: &Path, _: &archive::Item, _: Option<&archive::Item>) -> std::io::Result<archive::Overwrite> {
        Ok(archive::Overwrite::Overwrite)
    }
}

/// What Extract files asked (arclite's ExtractOptions and plugin.cpp's
/// extract): where to, into a folder of the archive's name if asked, what
/// to do with a file that is there, then deleting the archive if asked.
/// What extracting or adding tells the progress box, and how it asks about
/// a file that is there (arclite's Overwrite file).
struct ArcReport<'a, 'b> {
    reporter: &'a mut fileops::Reporter<'b>,
    remembered: Option<archive::Overwrite>,
}

impl archive::Progress for ArcReport<'_, '_> {
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

impl archive::Extracting for ArcReport<'_, '_> {
        fn exists(&mut self, target: &Path, item: &archive::Item, in_archive: Option<&archive::Item>) -> std::io::Result<archive::Overwrite> {
            if let Some(choice) = self.remembered {
                return Ok(choice);
            }
            let old = match in_archive {
                Some(old) => (old.size, old.modified),
                None => {
                    let meta = std::fs::metadata(target).ok();
                    (meta.as_ref().map_or(0, |m| m.len()), meta.and_then(|m| m.modified().ok()))
                }
            };
            let question = fileops::Question::Exists { path: target.to_path_buf(), new: (item.size, item.modified), old, update: in_archive.is_some() };
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

        fn remembers(&self) -> bool {
            self.remembered.is_some()
        }
}

fn unpack(pick: &panels::ArchivePick, values: &dialog::Values, base: &Path, reporter: &mut fileops::Reporter) -> std::io::Result<()> {
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
    let mut report = ArcReport { reporter, remembered: None };
    archive::extract(&view.file, &view.listing, &pick.items, &view.dir, &dest, password.as_deref(), policy, &mut report)?;
    // Delete files from archive after extraction (F6 from a zip)
    if values.choice("movefiles") == 1 && archive::updatable(&view.listing) {
        archive::zip_delete(&view.file, &pick.items)?;
    }
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
            overwrite: archive::Overwrite::Ask,
        };
        archive::create(&options, &[dir.join("a.txt")], &mut Unattended).unwrap();
        let listing = archive::list(&options.path, None).unwrap();
        let view = panels::ArchiveView { file: options.path.clone(), listing, dir: String::new(), password: None };
        let pick = panels::ArchivePick { view, items: vec!["a.txt".into()] };
        let mut dialog = arcdialog::extract(pick.clone(), &dir.join("out"), false);
        let dialog::Outcome::Pressed(_, "ok", values) = dialog.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) else { panic!("OK") };
        let base = dir.clone();
        let mut job = fileops::spawn(Kind::Unpack, Box::new(move |reporter| unpack(&pick, &values, &base, reporter)));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut asked = 0;
        while job.result.is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            job.poll();
            if let Some(fileops::Question::Exists { path, new, old, update: false }) = job.question.clone() {
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
