//! farsh: the user's own shell (bash / zsh / PowerShell) in a PTY, with Far
//! Manager style panels that Ctrl+O shows and hides.

mod cwd;
mod dialog;
mod editor;
mod fileops;
mod grid;
mod keys;
mod mouse;
mod panels;
mod shell;
mod viewer;

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::terminal;

use dialog::{Action, Dialog};
use fileops::{Job, Kind};
use grid::Grid;
use panels::{Key, Panels, Request};
use shell::Shell;
use viewer::Viewer;

fn main() -> Result<()> {
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

/// What is on top of the shell and the panels.
enum Overlay {
    None,
    Dialog(Dialog),
    Viewer(Viewer),
    Job(Job),
}

struct App {
    shell: Shell,
    panels: Panels,
    panels_visible: bool,
    overlay: Overlay,
    quit: bool,
    /// The last left click: when and where, to tell a double click.
    last_click: Option<(std::time::Instant, u16, u16)>,
    size: (u16, u16),
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
            size: (cols, rows),
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
            let wait = if matches!(self.overlay, Overlay::Job(_)) { 50 } else { 10 };
            if !event::poll(Duration::from_millis(wait))? {
                dirty |= matches!(self.overlay, Overlay::Job(_));
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
                    self.size = (cols, rows);
                    self.shell.resize(cols, rows);
                    screen = Grid::new(cols, rows);
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
            Overlay::Job(job) => {
                dialog::paint_progress(job, frame);
                cursor = None;
            }
        }
        cursor
    }

    /// Takes the worker's news; when it is done, closes the box and reads the panels again.
    fn poll_job(&mut self) -> bool {
        let Overlay::Job(job) = &mut self.overlay else { return false };
        job.poll();
        let Some(result) = job.result.clone() else { return false };
        self.overlay = match result {
            Err(error) if error != "Cancelled" => Overlay::Dialog(Dialog::message("Error", &error)),
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
                dialog::Outcome::Accepted(action, text) => self.accept(action, &text),
            },
            Overlay::Viewer(mut viewer) => {
                if matches!(viewer.key(key), viewer::Outcome::Open) {
                    self.overlay = Overlay::Viewer(viewer);
                }
            }
            Overlay::Job(job) => {
                if key.code == KeyCode::Esc {
                    job.cancel();
                }
                self.overlay = Overlay::Job(job);
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
                    dialog::Outcome::Accepted(action, text) => self.accept(action, &text),
                }
            }
            Overlay::Viewer(mut viewer) => {
                viewer.mouse(event);
                self.overlay = Overlay::Viewer(viewer);
            }
            Overlay::Job(job) => self.overlay = Overlay::Job(job),
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
                    Err(error) => Overlay::Dialog(Dialog::message("Error", &format!("{}: {error}", path.display()))),
                };
            }
            Request::Edit(path) => {
                if !self.shell.command_line_is_empty() {
                    self.overlay = Overlay::Dialog(Dialog::message("Edit", "Clear the command line first"));
                    return;
                }
                let line = editor::command_line(&editor::editor(), self.shell.kind(), &path);
                self.shell.run(&line);
                // the editor takes the screen; Ctrl+O brings the panels back
                self.panels_visible = false;
            }
            Request::Copy(sources, other) => {
                let label = format!("Copy {} to:", describe(&sources));
                let prompt = Dialog::prompt("Copy", label, &other.display().to_string(), "Copy", Action::Copy(sources));
                self.overlay = Overlay::Dialog(prompt);
            }
            Request::Move(sources, other) => {
                let label = format!("Rename or move {} to:", describe(&sources));
                let prompt = Dialog::prompt("Rename/Move", label, &other.display().to_string(), "Rename", Action::Move(sources));
                self.overlay = Overlay::Dialog(prompt);
            }
            Request::MkDir => {
                let prompt = Dialog::prompt("Make folder", "Create the folder".into(), "", "OK", Action::MkDir);
                self.overlay = Overlay::Dialog(prompt);
            }
            Request::Delete(sources) => {
                let lines = vec!["Do you wish to delete".to_string(), describe(&sources)];
                self.overlay = Overlay::Dialog(Dialog::confirm("Delete", lines, "Delete", Action::Delete(sources)));
            }
            Request::Quit => {
                let lines = vec!["Do you want to quit farsh?".to_string()];
                self.overlay = Overlay::Dialog(Dialog::confirm("Quit", lines, "Quit", Action::Quit));
            }
        }
    }

    /// A dialog was accepted.
    fn accept(&mut self, action: Action, text: &str) {
        match action {
            Action::Copy(sources) => self.start_job(Kind::Copy, &sources, text),
            Action::Move(sources) => self.start_job(Kind::Move, &sources, text),
            Action::Delete(sources) => {
                let items = sources.into_iter().map(|path| (path.clone(), path)).collect();
                self.overlay = Overlay::Job(fileops::start(Kind::Delete, items));
            }
            Action::MkDir => {
                let name = text.trim();
                if name.is_empty() {
                    return;
                }
                match std::fs::create_dir_all(self.panels.active_dir().join(name)) {
                    Ok(()) => {
                        let first = name.split(['/', '\\']).find(|part| !part.is_empty()).unwrap_or(name).to_string();
                        self.panels.refresh(Some(&first));
                    }
                    Err(error) => {
                        self.overlay = Overlay::Dialog(Dialog::message("Error", &format!("{name}: {error}")));
                    }
                }
            }
            Action::Quit => self.quit = true,
        }
    }

    fn start_job(&mut self, kind: Kind, sources: &[PathBuf], typed: &str) {
        match fileops::targets(sources, typed, self.panels.active_dir()) {
            Ok(items) => self.overlay = Overlay::Job(fileops::start(kind, items)),
            Err(error) => self.overlay = Overlay::Dialog(Dialog::message("Error", &error)),
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

/// `file "a"`, `folder "b"` or `3 items`, for a dialog's text.
fn describe(sources: &[PathBuf]) -> String {
    match sources {
        [one] => {
            let name = one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            format!("{} \"{name}\"", if one.is_dir() { "folder" } else { "file" })
        }
        many => format!("{} items", many.len()),
    }
}
