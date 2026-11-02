//! F4: the file goes to an external editor, started in the shell as if typed.

use std::path::{Path, PathBuf};

use crate::cwd::Kind;

/// `$FARSH_EDITOR`, `$VISUAL`, `$EDITOR`; else micro or fresh when installed;
/// else notepad on Windows and nano elsewhere.
pub fn editor() -> String {
    for name in ["FARSH_EDITOR", "VISUAL", "EDITOR"] {
        if let Some(value) = std::env::var(name).ok().filter(|v| !v.trim().is_empty()) {
            return value;
        }
    }
    ["micro", "fresh"]
        .into_iter()
        .find_map(find_on_path)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| if cfg!(windows) { "notepad".into() } else { "nano".into() })
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(&file)).find(|p| p.is_file())
}

/// The line that opens `path` in `editor` in a shell of `kind`.
pub fn command_line(editor: &str, kind: Kind, path: &Path) -> String {
    // An editor named by a path with spaces has to be quoted; one with
    // arguments ("code -w") is used as written.
    let program = if editor.contains(char::is_whitespace) && Path::new(editor).is_file() {
        match kind {
            Kind::PowerShell => format!("& {}", quote(kind, editor)),
            _ => quote(kind, editor),
        }
    } else {
        editor.to_string()
    };
    format!("{program} {}", quote(kind, &path.display().to_string()))
}

/// A file name as it goes on the command line (Ctrl+Enter): as it is when it
/// is plain, quoted when the shell would take it apart, with a space after it.
pub fn name_for_command_line(kind: Kind, name: &str) -> String {
    let plain = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || "._-+/@%:,=~".contains(c));
    format!("{} ", if plain { name.to_string() } else { quote(kind, name) })
}

/// `text` as one argument.
pub fn quote(kind: Kind, text: &str) -> String {
    match kind {
        Kind::PowerShell => format!("'{}'", text.replace('\'', "''")),
        Kind::Other if cfg!(windows) => format!("\"{text}\""),
        _ => format!("'{}'", text.replace('\'', "'\\''")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_is_quoted_for_the_shell() {
        let path = Path::new("/my docs/it's.txt");
        assert_eq!(command_line("micro", Kind::Bash, path), "micro '/my docs/it'\\''s.txt'");
        assert_eq!(command_line("micro", Kind::PowerShell, path), "micro '/my docs/it''s.txt'");
    }

    #[test]
    fn a_name_is_quoted_only_when_it_needs_to_be() {
        assert_eq!(name_for_command_line(Kind::Bash, "main.rs"), "main.rs ");
        assert_eq!(name_for_command_line(Kind::PowerShell, "my file.txt"), "'my file.txt' ");
        assert_eq!(name_for_command_line(Kind::Zsh, "it's (1).txt"), "'it'\\''s (1).txt' ");
        assert_eq!(name_for_command_line(Kind::Bash, "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}.txt"), "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}.txt ");
    }

    #[test]
    fn an_editor_with_arguments_is_left_as_written() {
        assert_eq!(command_line("code -w", Kind::Zsh, Path::new("/a")), "code -w '/a'");
    }

    #[test]
    fn an_installed_program_with_spaces_in_its_path_is_quoted() {
        let exe = std::env::current_exe().unwrap();
        let text = exe.display().to_string();
        if text.contains(' ') {
            assert!(command_line(&text, Kind::PowerShell, Path::new("a")).starts_with("& '"));
        } else {
            assert!(command_line(&text, Kind::Bash, Path::new("a")).starts_with(&text));
        }
    }
}
