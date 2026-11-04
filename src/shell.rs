//! The user's shell in a PTY, and the screen it draws.

use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use anyhow::{Context, Result};
use crossterm::style::Color;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::cwd::{self, Kind, Osc7};
use crate::grid::{Cell, Grid};

pub struct Shell {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    /// The cursor line as the shell drew it when it last showed a bare prompt;
    /// the command line is empty while the line still equals this.
    prompt_line: String,
    /// A command was started (or the shell is new): the next output is a prompt.
    awaiting_prompt: bool,
    kind: Kind,
    osc7: Osc7,
    /// The directory the shell last reported, until `take_cwd` hands it over.
    cwd: Option<PathBuf>,
    /// Nothing has come from the PTY yet. Windows' ConPTY (created with
    /// INHERIT_CURSOR by portable-pty) starts with a cursor query and shows
    /// nothing until it is answered.
    first_output: bool,
    /// How many prompts the shell has shown (each one reports its directory).
    prompts: u64,
}

impl Shell {
    pub fn spawn(cols: u16, rows: u16) -> Result<Self> {
        Self::spawn_program(&shell_program(), cols, rows)
    }

    pub fn spawn_program(program: &str, cols: u16, rows: u16) -> Result<Self> {
        let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).context("open a PTY")?;
        let kind = Kind::of(program);
        let mut command = CommandBuilder::new(program);
        // A shell that cannot be hooked still works, its directory just is not followed.
        let scratch = std::env::temp_dir().join(format!("farsh-{}", std::process::id()));
        if let Ok(hooked) = cwd::integration(kind, &scratch) {
            command.args(hooked.args);
            for (name, value) in hooked.env {
                command.env(name, value);
            }
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("FARSH", "1");
        if let Ok(dir) = std::env::current_dir() {
            command.cwd(dir);
        }
        let child = pair.slave.spawn_command(command).context("start the shell")?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || sender.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Ok(Self { master: pair.master, writer, child, output, parser: vt100::Parser::new(nz(rows), nz(cols), 0), prompt_line: String::new(), awaiting_prompt: true, kind, osc7: Osc7::default(), cwd: None, first_output: true, prompts: 0 })
    }

    /// Feeds what the shell wrote to the screen; reports whether there was any.
    pub fn pump(&mut self) -> bool {
        let mut any = false;
        while let Ok(mut bytes) = self.output.try_recv() {
            if std::mem::take(&mut self.first_output) && bytes.starts_with(CURSOR_QUERY) {
                // the screen is new: the cursor is at the top left
                let _ = self.writer.write_all(b"\x1b[1;1R");
                let _ = self.writer.flush();
                bytes.drain(..CURSOR_QUERY.len());
            }
            self.parser.process(&bytes);
            if let Some(dir) = self.osc7.feed(&bytes) {
                self.cwd = Some(dir);
                self.prompts += 1;
                // The hook runs before the prompt is drawn: what follows is
                // the bare prompt, even if keys went to the command meanwhile
                // (an editor's) and `write` stopped waiting for it.
                self.awaiting_prompt = true;
            }
            any = true;
        }
        if any && self.awaiting_prompt {
            self.prompt_line = self.cursor_line().0;
        }
        any
    }

    pub fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Passes a key's bytes to the shell. `runs` marks Enter: what the shell
    /// prints next ends in a fresh prompt, which becomes the empty command line.
    pub fn write(&mut self, bytes: &[u8], runs: bool) {
        self.awaiting_prompt = runs;
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// The directory the shell reported since the last call, if it moved.
    pub fn take_cwd(&mut self) -> Option<PathBuf> {
        self.cwd.take()
    }

    /// How many prompts the shell has shown: one more means the command
    /// started at the last one has ended. Only a shell with the directory
    /// hook counts (PowerShell, bash, zsh); for any other it stays at 0.
    pub fn prompts(&self) -> u64 {
        self.prompts
    }

    /// Empties the shell's command line, the way its own line editor does:
    /// Escape for PSReadLine's Windows mode and cmd, Ctrl+E Ctrl+U (to the
    /// end, then everything before) for readline, zle and PSReadLine's
    /// Emacs mode, the default outside Windows.
    pub fn clear_line(&mut self) {
        let keys: &[u8] = match self.kind {
            Kind::PowerShell | Kind::Other if cfg!(windows) => b"\x1b",
            _ => b"\x05\x15",
        };
        self.write(keys, false);
    }

    /// Makes the shell go to `dir`, as if `cd` were typed at an empty prompt.
    pub fn cd(&mut self, dir: &Path) {
        self.run(&cwd::cd_command(self.kind, dir));
    }

    /// Runs `line` as if it were typed at an empty prompt and Enter pressed.
    pub fn run(&mut self, line: &str) {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\r');
        self.write(&bytes, true);
    }

    /// How the program in the shell wants the mouse reported, if it does.
    pub fn mouse_mode(&self) -> (vt100::MouseProtocolMode, vt100::MouseProtocolEncoding) {
        let screen = self.parser.screen();
        (screen.mouse_protocol_mode(), screen.mouse_protocol_encoding())
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The line the shell's cursor is on (prompt and what is typed after it)
    /// and the cursor's column in it: the command line, as the shell has it.
    pub fn cursor_line(&self) -> (String, u16) {
        let screen = self.parser.screen();
        let (row, col) = screen.cursor_position();
        let line: String = (0..screen.size().1.get())
            .map(|c| screen.cell(row, c).map_or(String::new(), |cell| cell.contents().to_string()))
            .map(|text| if text.is_empty() { " ".to_string() } else { text })
            .collect();
        (line.trim_end().to_string(), col)
    }

    /// The screen row the shell's cursor is on.
    pub fn cursor_row(&self) -> u16 {
        self.parser.screen().cursor_position().0
    }

    /// Whether nothing is typed after the prompt.
    pub fn command_line_is_empty(&self) -> bool {
        self.cursor_line().0 == self.prompt_line
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        self.parser.set_size(nz(rows), nz(cols));
    }

    pub fn cursor(&self) -> Option<(u16, u16)> {
        let screen = self.parser.screen();
        (!screen.hide_cursor()).then(|| {
            let (row, col) = screen.cursor_position();
            (col, row)
        })
    }

    pub fn paint(&self, grid: &mut Grid) {
        let screen = self.parser.screen();
        for row in 0..grid.rows {
            for col in 0..grid.cols {
                let Some(source) = screen.cell(row, col) else { continue };
                if source.is_wide_continuation() {
                    if let Some(cell) = grid.cell_mut(col, row) {
                        cell.text = String::new();
                    }
                    continue;
                }
                let text = source.contents();
                if let Some(cell) = grid.cell_mut(col, row) {
                    *cell = Cell {
                        text: if text.is_empty() { " ".into() } else { text.to_string() },
                        fg: color(source.fgcolor()),
                        bg: color(source.bgcolor()),
                        bold: source.bold(),
                        underline: source.underline(),
                        reverse: source.inverse(),
                    };
                }
            }
        }
    }
}

fn color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::AnsiValue(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb { r, g, b },
    }
}

/// `$FARSH_SHELL` or `$SHELL`; otherwise bash on Linux, zsh on macOS and
/// PowerShell on Windows.
fn shell_program() -> String {
    if let Ok(shell) = std::env::var("FARSH_SHELL").or_else(|_| std::env::var("SHELL")) {
        return shell;
    }
    if cfg!(windows) {
        windows_powershell()
    } else if cfg!(target_os = "macos") {
        "/bin/zsh".into()
    } else {
        "/bin/bash".into()
    }
}

/// PowerShell 7 where it is installed (PATH, then its default folder), else
/// the Windows PowerShell that ships with the system.
fn windows_powershell() -> String {
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join("pwsh.exe"))
        .find(|candidate| candidate.is_file());
    let default = std::path::PathBuf::from(r"C:\Program Files\PowerShell\7\pwsh.exe");
    on_path
        .or_else(|| default.is_file().then_some(default))
        .map_or_else(|| "powershell.exe".into(), |path| path.display().to_string())
}

/// The device status report ConPTY sends first to learn where the cursor is.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(1)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn settle(shell: &mut Shell) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut quiet, mut seen) = (0, false);
        while Instant::now() < deadline && !(seen && quiet >= 5) {
            std::thread::sleep(Duration::from_millis(60));
            if shell.pump() {
                (quiet, seen) = (0, true);
            } else {
                quiet += 1;
            }
        }
    }

    #[test]
    fn a_hooked_powershell_reports_its_directory_and_obeys_cd() {
        let program = windows_powershell_or_skip();
        let Some(program) = program else { return };
        let mut shell = Shell::spawn_program(&program, 100, 30).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while shell.cwd.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(60));
            shell.pump();
        }
        let start = shell.take_cwd().expect("the first prompt reports the directory");
        assert_eq!(start, std::env::current_dir().unwrap());

        let target = std::env::temp_dir().canonicalize().unwrap();
        let target = PathBuf::from(target.display().to_string().trim_start_matches(r"\\?\"));
        shell.cd(&target);
        let deadline = Instant::now() + Duration::from_secs(10);
        while shell.cwd.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(60));
            shell.pump();
        }
        let reported = shell.take_cwd().expect("the prompt after cd reports the new directory");
        assert_eq!(reported.to_string_lossy().to_lowercase(), target.to_string_lossy().to_lowercase());
        shell.write(b"exit\r", true);
    }

    #[test]
    fn a_hooked_shell_counts_a_prompt_only_when_the_command_ends() {
        let Some(program) = windows_powershell_or_skip() else { return };
        let mut shell = Shell::spawn_program(&program, 100, 30).unwrap();
        let wait_for_prompt = |shell: &mut Shell, after: u64| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while shell.prompts() <= after && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(60));
                shell.pump();
            }
            assert!(shell.prompts() > after, "no prompt after {after}");
        };
        wait_for_prompt(&mut shell, 0);
        settle(&mut shell);
        let before = shell.prompts();
        // what an editor is to the shell: a command that holds the screen a while
        shell.run("Start-Sleep -Milliseconds 1500");
        std::thread::sleep(Duration::from_millis(700));
        shell.pump();
        assert_eq!(shell.prompts(), before, "a prompt counted while the command still runs");
        wait_for_prompt(&mut shell, before);
        assert_eq!(shell.prompts(), before + 1);
        shell.write(b"exit\r", true);
    }

    #[test]
    fn after_a_program_that_read_keys_the_command_line_is_empty_again() {
        let Some(program) = windows_powershell_or_skip() else { return };
        let mut shell = Shell::spawn_program(&program, 100, 30).unwrap();
        settle(&mut shell);
        let before = shell.prompts();
        // an editor: it takes keys, and quits on one
        shell.run("$null = [Console]::ReadKey($true)");
        settle(&mut shell);
        shell.write(b"q", false);
        let deadline = Instant::now() + Duration::from_secs(10);
        while shell.prompts() <= before && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(60));
            shell.pump();
        }
        settle(&mut shell);
        assert!(shell.command_line_is_empty(), "after the program: {:?} vs {:?}", shell.cursor_line(), shell.prompt_line);
        shell.write(b"exit\r", true);
    }

    #[test]
    fn clear_line_empties_what_is_typed() {
        let mut programs = vec![if cfg!(windows) { "cmd.exe".to_string() } else { "/bin/bash".to_string() }];
        programs.extend(windows_powershell_or_skip());
        for program in programs {
            let mut shell = Shell::spawn_program(&program, 100, 30).unwrap();
            settle(&mut shell);
            assert!(shell.command_line_is_empty(), "{program}: bare prompt: {:?}", shell.cursor_line());
            shell.write(b"echo hi there", false);
            settle(&mut shell);
            assert!(!shell.command_line_is_empty(), "{program}: typed: {:?}", shell.cursor_line());
            shell.clear_line();
            settle(&mut shell);
            assert!(shell.command_line_is_empty(), "{program}: cleared: {:?} vs {:?}", shell.cursor_line(), shell.prompt_line);
            shell.write(b"exit\r", true);
        }
    }

    fn windows_powershell_or_skip() -> Option<String> {
        cfg!(windows).then(windows_powershell).filter(|path| std::path::Path::new(path).is_file())
    }

    #[test]
    fn the_command_line_is_the_shells_own_line_after_its_prompt() {
        let mut shell = Shell::spawn_program(if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }, 100, 30).unwrap();
        // ConPTY's first output is a cursor query; the prompt follows its answer.
        let deadline = Instant::now() + Duration::from_secs(5);
        while shell.cursor_line().0.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(60));
            shell.pump();
        }
        settle(&mut shell);
        assert!(shell.command_line_is_empty(), "bare prompt: {:?}", shell.cursor_line());

        shell.write(b"echo hi", false);
        settle(&mut shell);
        assert!(!shell.command_line_is_empty(), "typed: {:?} vs {:?}", shell.cursor_line(), shell.prompt_line);
        assert!(shell.cursor_line().0.ends_with("echo hi"));

        shell.write(b"\x08\x08\x08\x08\x08\x08\x08", false);
        settle(&mut shell);
        assert!(shell.command_line_is_empty(), "erased: {:?} vs {:?}", shell.cursor_line(), shell.prompt_line);

        shell.write(b"echo hi\r", true);
        settle(&mut shell);
        assert!(shell.command_line_is_empty(), "new prompt: {:?}", shell.cursor_line());
        shell.write(b"exit\r", true);
    }
}
