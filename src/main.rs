//! farsh: the user's own shell (bash / zsh / PowerShell) in a PTY, with Far
//! Manager style panels that Ctrl+O shows and hides.

mod cwd;
mod grid;
mod keys;
mod panels;
mod shell;

use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use grid::Grid;
use panels::Panels;
use shell::Shell;

fn main() -> Result<()> {
    let (cols, rows) = terminal::size()?;
    let mut shell = Shell::spawn(cols, rows)?;
    let mut panels = Panels::new(std::env::current_dir()?);
    let mut panels_visible = false;

    terminal::enable_raw_mode()?;
    let mut out = std::io::stdout();
    crossterm::execute!(out, terminal::EnterAlternateScreen, crossterm::cursor::Hide)?;
    let result = run(&mut shell, &mut panels, &mut panels_visible, cols, rows);
    crossterm::execute!(out, crossterm::cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    result
}

fn run(shell: &mut Shell, panels: &mut Panels, visible: &mut bool, cols: u16, rows: u16) -> Result<()> {
    let mut out = std::io::stdout();
    let mut screen = Grid::new(cols, rows);
    let mut frame = Grid::new(cols, rows);
    let mut dirty = true;

    loop {
        if shell.pump() {
            dirty = true;
        }
        if shell.exited() {
            return Ok(());
        }
        // The shell changed directory (a typed `cd`, a script): the panel follows.
        if let Some(dir) = shell.take_cwd() {
            panels.go_to(dir);
            dirty = true;
        }
        if dirty {
            frame.clear();
            shell.paint(&mut frame);
            let cursor = if *visible {
                let (line, col) = shell.cursor_line();
                panels.paint(&mut frame, &line, col)
            } else {
                shell.cursor()
            };
            frame.flush(&mut screen, cursor, &mut out)?;
            out.flush()?;
            dirty = false;
        }
        if !event::poll(Duration::from_millis(10))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                dirty = true;
                let dir_before = panels.active_dir().to_path_buf();
                if key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    *visible = !*visible;
                } else if *visible && panels.key(key, shell.command_line_is_empty()) {
                    // taken by the panels; if that took the panel to another
                    // folder, the shell goes there too
                    if panels.active_dir() != dir_before {
                        shell.cd(panels.active_dir());
                    }
                } else if let Some(bytes) = keys::encode(panels::to_shell(key)) {
                    let runs = key.code == KeyCode::Enter;
                    shell.write(&bytes, runs);
                    if runs {
                        // the command's output is the shell's to show
                        *visible = false;
                    }
                }
            }
            Event::Resize(cols, rows) => {
                shell.resize(cols, rows);
                screen = Grid::new(cols, rows);
                frame = Grid::new(cols, rows);
                dirty = true;
            }
            _ => {}
        }
    }
}
