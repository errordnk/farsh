//! farsh's own settings folder (Far's profile): the histories, folder
//! shortcuts, file associations, filters and the main user menu, each a
//! small text file. Windows: `%APPDATA%\farsh`; elsewhere
//! `$XDG_CONFIG_HOME/farsh` or `~/.config/farsh`.

use std::path::PathBuf;

pub fn dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FARSH_PROFILE") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.unwrap_or_else(std::env::temp_dir).join("farsh")
}

pub fn path(name: &str) -> PathBuf {
    dir().join(name)
}

/// A field of a tab-separated line, with `\t`, `\n` and `\\` escaped.
pub fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

pub fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The fields of a tab-separated line.
pub fn fields(line: &str) -> Vec<String> {
    line.split('\t').map(unescape).collect()
}

pub fn join(fields: &[&str]) -> String {
    fields.iter().map(|f| escape(f)).collect::<Vec<_>>().join("\t")
}
