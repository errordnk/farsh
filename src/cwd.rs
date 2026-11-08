//! Keeping the shell's directory and the panel's the same.
//!
//! Shell to panel: the shell is started with a hook that prints its directory
//! (OSC 7, `ESC ] 7 ; file://host/path BEL`) before every prompt, and
//! [`Osc7`] picks it out of its output. Panel to shell: [`cd_command`] is the
//! line that takes the shell where the panel went.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    PowerShell,
    Bash,
    Zsh,
    Other,
}

impl Kind {
    pub fn of(program: &str) -> Self {
        let name = Path::new(program).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        match name.as_str() {
            "pwsh" | "powershell" => Self::PowerShell,
            "bash" => Self::Bash,
            "zsh" => Self::Zsh,
            _ => Self::Other,
        }
    }
}

/// How to start a shell so that it reports its directory.
#[derive(Default, Debug, PartialEq)]
pub struct Integration {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// The arguments and environment that make `program` print OSC 7 before each
/// prompt; `dir` is a scratch folder for the start-up files bash and zsh need.
/// The user's own start-up files still run first.
///
/// `login`: the shell is a login shell (farsh is the user's shell), which
/// reads the user's login files. A real `bash -l` ignores `--rcfile`, and a
/// prompt tool of the user's (starship...) replaces a `PROMPT_COMMAND` given
/// in the environment, so bash's hook must come after the user's files:
/// the rcfile reads what a login bash reads, in its order, then the hook (the
/// differences: `shopt login_shell` is off, `.bash_logout` is run by a trap).
/// zsh gets stubs for `.zprofile` and `.zlogin` that read the user's.
pub fn integration(kind: Kind, dir: &Path, login: bool) -> std::io::Result<Integration> {
    Ok(match kind {
        Kind::PowerShell => Integration {
            // Runs after the profile, so the prompt it wraps is the user's own.
            env: if cfg!(windows) {
                std::fs::create_dir_all(dir)?;
                vec![("FARSH_CDFILE".to_string(), cd_file(dir).display().to_string())]
            } else {
                vec![]
            },
            args: [if login { vec!["-Login".to_string()] } else { vec![] }, vec!["-NoExit".into(), "-Command".into(), POWERSHELL_HOOK.into()]].concat(),
        },
        Kind::Bash => {
            std::fs::create_dir_all(dir)?;
            let rc = dir.join("bashrc");
            let init = if login { BASH_LOGIN } else { BASH_USER_RC };
            std::fs::write(&rc, format!("{DIR_FN}{init}{BASH_HOOK}"))?;
            Integration { args: vec!["--rcfile".into(), rc.display().to_string(), "-i".into()], env: vec![] }
        }
        Kind::Zsh => {
            std::fs::create_dir_all(dir)?;
            let stub = |file: &str| format!("[ -f \"${{FARSH_ZDOTDIR:-$HOME}}/{file}\" ] && . \"${{FARSH_ZDOTDIR:-$HOME}}/{file}\"\n");
            for (name, text) in [
                (".zshenv", ZSH_ENV.to_string()),
                (".zprofile", stub(".zprofile")),
                (".zshrc", format!("{DIR_FN}{ZSH_RC}")),
                (".zlogin", stub(".zlogin")),
            ] {
                std::fs::write(dir.join(name), text)?;
            }
            let real = std::env::var("ZDOTDIR").unwrap_or_else(|_| std::env::var("HOME").unwrap_or_default());
            Integration {
                args: if login { vec!["-l".into()] } else { vec![] },
                env: vec![("ZDOTDIR".into(), dir.display().to_string()), ("FARSH_ZDOTDIR".into(), real)],
            }
        }
        Kind::Other => Integration { args: if login { vec!["-l".into()] } else { vec![] }, env: vec![] },
    })
}

const POWERSHELL_HOOK: &str = "$global:__farsh_prompt = $function:prompt; \
function global:prompt { \
$p = (Get-Location).ProviderPath -replace '\\\\', '/'; \
[Console]::Out.Write(\"$([char]27)]7;file:///$p$([char]7)\"); \
& $global:__farsh_prompt }; try { Set-PSReadLineKeyHandler -Chord 'Ctrl+Alt+F12' -ScriptBlock { Set-Location -LiteralPath ([IO.File]::ReadAllText($env:FARSH_CDFILE)); $e = [Console]::OutputEncoding; [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); \n[Microsoft.PowerShell.PSConsoleReadLine]::InvokePrompt(); [Console]::OutputEncoding = $e } } catch {}";

/// What farsh sends to a PowerShell on Windows to make it read the folder in the
/// `FARSH_CDFILE` file and go there: the key the hook above binds. The shell
/// redraws its prompt in place, so nothing is typed into its screen.
pub const POWERSHELL_CD_KEY: &[u8] = b"[24;7~";

/// Where the folder for [`POWERSHELL_CD_KEY`] is put.
pub fn cd_file(dir: &Path) -> PathBuf {
    dir.join("cd")
}

/// The directory for OSC 7. On Windows (Git Bash, MSYS2, Cygwin) `$PWD` is a
/// path of their own (`/c/Users/x`, `/tmp`): `cygpath -m` gives the Windows
/// one (`C:/Users/x`, sent as `/C:/Users/x`). Elsewhere there is no
/// `cygpath` and `$PWD` goes as it is.
const DIR_FN: &str = "__farsh_cygpath=$(command -v cygpath 2>/dev/null)\n\
__farsh_dir() { if [ -n \"$__farsh_cygpath\" ]; then printf '/%s' \"$(cygpath -m \"$PWD\")\"; else printf '%s' \"$PWD\"; fi; }\n";

const BASH_USER_RC: &str = "[ -f ~/.bashrc ] && . ~/.bashrc\n";

/// What a login bash reads: /etc/profile, then the first of the user's three files.
const BASH_LOGIN: &str = "[ -f /etc/profile ] && . /etc/profile\n\
if [ -f ~/.bash_profile ]; then . ~/.bash_profile\n\
elif [ -f ~/.bash_login ]; then . ~/.bash_login\n\
elif [ -f ~/.profile ]; then . ~/.profile\n\
fi\n\
trap '[ -f ~/.bash_logout ] && . ~/.bash_logout' EXIT\n";

const BASH_HOOK: &str = "__farsh_osc7() { printf '\\033]7;file://%s%s\\007' \"$HOSTNAME\" \"$(__farsh_dir)\"; }\n\
PROMPT_COMMAND=\"__farsh_osc7${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"\n";

const ZSH_ENV: &str = "[ -f \"${FARSH_ZDOTDIR:-$HOME}/.zshenv\" ] && . \"${FARSH_ZDOTDIR:-$HOME}/.zshenv\"\n";

const ZSH_RC: &str = "ZDOTDIR=\"${FARSH_ZDOTDIR:-$HOME}\"\n\
[ -f \"$ZDOTDIR/.zshrc\" ] && . \"$ZDOTDIR/.zshrc\"\n\
autoload -Uz add-zsh-hook\n\
__farsh_osc7() { printf '\\033]7;file://%s%s\\007' \"$HOST\" \"$(__farsh_dir)\"; }\n\
add-zsh-hook precmd __farsh_osc7\n";

/// The line that makes the shell go to `dir`.
pub fn cd_command(kind: Kind, dir: &Path) -> String {
    let dir = dir.display().to_string();
    match kind {
        Kind::PowerShell => format!("Set-Location -LiteralPath '{}'", dir.replace('\'', "''")),
        Kind::Other if cfg!(windows) => format!("cd /d \"{dir}\""),
        _ => format!("cd -- '{}'", dir.replace('\'', "'\\''")),
    }
}

/// Finds the directory in a stream of OSC 7 sequences, across chunk borders.
#[derive(Default)]
pub struct Osc7 {
    pending: Vec<u8>,
}

impl Osc7 {
    /// Feeds `chunk`; returns the last directory it reported, if any.
    pub fn feed(&mut self, chunk: &[u8]) -> Option<PathBuf> {
        const START: &[u8] = b"\x1b]7;";
        self.pending.extend_from_slice(chunk);
        let mut found = None;
        loop {
            let Some(start) = find(&self.pending, START) else {
                // Keep a possible beginning of a sequence cut by the chunk.
                let keep = (1..START.len()).rev().find(|&n| self.pending.ends_with(&START[..n])).unwrap_or(0);
                let from = self.pending.len() - keep;
                self.pending.drain(..from);
                return found;
            };
            let body = &self.pending[start + START.len()..];
            let Some((len, end)) = terminator(body) else {
                // Unfinished: wait for the rest (but never hoard junk).
                self.pending.drain(..start);
                if self.pending.len() > 8192 {
                    self.pending.clear();
                }
                return found;
            };
            if let Some(path) = parse_uri(&body[..len]) {
                found = Some(path);
            }
            let consumed = start + START.len() + end;
            self.pending.drain(..consumed);
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Where an OSC body ends: (length of the text, bytes up to and with the terminator).
fn terminator(body: &[u8]) -> Option<(usize, usize)> {
    (0..body.len()).find_map(|i| match body[i] {
        0x07 => Some((i, i + 1)),
        0x1b if body.get(i + 1) == Some(&b'\\') => Some((i, i + 2)),
        _ => None,
    })
}

/// `file://host/path` (or `file:///C:/path`) as a local path.
fn parse_uri(uri: &[u8]) -> Option<PathBuf> {
    let uri = std::str::from_utf8(uri).ok()?.strip_prefix("file://")?;
    let path = &uri[uri.find('/')?..];
    let mut bytes = Vec::with_capacity(path.len());
    let raw = path.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            if let Ok(byte) = u8::from_str_radix(&path[i + 1..i + 3], 16) {
                bytes.push(byte);
                i += 3;
                continue;
            }
        }
        bytes.push(raw[i]);
        i += 1;
    }
    let text = String::from_utf8(bytes).ok()?;
    // `/C:/Users/x` is a Windows path.
    let windows = text.len() > 2 && text.as_bytes()[2] == b':' && text.as_bytes()[1].is_ascii_alphabetic();
    Some(PathBuf::from(if windows { text[1..].replace('/', "\\") } else { text }))
}

/// Writes the OSC 7 a hooked shell would print (for tests).
#[cfg(test)]
pub fn osc7(path: &str) -> Vec<u8> {
    use std::io::Write;
    let mut out = Vec::new();
    write!(out, "\x1b]7;file://host{path}\x07").unwrap();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reported_directory_is_found_among_other_output() {
        let mut osc = Osc7::default();
        let mut bytes = b"hello\r\n".to_vec();
        bytes.extend(osc7("/home/dnk/my%20dir"));
        bytes.extend(b"$ ");
        assert_eq!(osc.feed(&bytes), Some(PathBuf::from("/home/dnk/my dir")));
        assert_eq!(osc.feed(b"more output"), None);
    }

    #[test]
    fn a_sequence_cut_between_chunks_is_still_found() {
        let mut osc = Osc7::default();
        let bytes = osc7("/tmp");
        for (i, byte) in bytes.iter().enumerate() {
            let found = osc.feed(&[*byte]);
            assert_eq!(found.is_some(), i == bytes.len() - 1, "byte {i}");
        }
    }

    #[test]
    fn the_last_of_several_directories_wins() {
        let mut osc = Osc7::default();
        let mut bytes = osc7("/a");
        bytes.extend(osc7("/b"));
        assert_eq!(osc.feed(&bytes), Some(PathBuf::from("/b")));
    }

    #[test]
    fn a_string_terminator_ends_it_too() {
        let mut osc = Osc7::default();
        assert_eq!(osc.feed(b"\x1b]7;file://h/x\x1b\\"), Some(PathBuf::from("/x")));
    }

    #[test]
    fn a_windows_uri_is_a_windows_path() {
        let mut osc = Osc7::default();
        assert_eq!(osc.feed(b"\x1b]7;file:///C:/Users/dnk/a%20b\x07"), Some(PathBuf::from(r"C:\Users\dnk\a b")));
    }

    #[test]
    fn other_escape_sequences_are_left_alone() {
        let mut osc = Osc7::default();
        assert_eq!(osc.feed(b"\x1b]0;title\x07\x1b[31mred\x1b[0m"), None);
    }

    #[test]
    fn cd_commands_quote_for_each_shell() {
        assert_eq!(cd_command(Kind::PowerShell, Path::new("/it's")), "Set-Location -LiteralPath '/it''s'");
        assert_eq!(cd_command(Kind::Bash, Path::new("/it's")), "cd -- '/it'\\''s'");
        assert_eq!(cd_command(Kind::Zsh, Path::new("/a b")), "cd -- '/a b'");
    }

    #[test]
    fn shells_are_told_apart_by_program_name() {
        assert_eq!(Kind::of(r"C:\Program Files\PowerShell\7\pwsh.exe"), Kind::PowerShell);
        assert_eq!(Kind::of("/bin/bash"), Kind::Bash);
        assert_eq!(Kind::of("/usr/bin/zsh"), Kind::Zsh);
        assert_eq!(Kind::of("fish"), Kind::Other);
    }
}
