//! farsh's own settings folder (Far's profile): `~/.config/farsh` on every
//! system (the user's rule), with `settings.json` and the histories, folder
//! shortcuts, file associations, filters and the main user menu, each a
//! small text file.

use std::path::PathBuf;

pub fn dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FARSH_PROFILE") {
        return PathBuf::from(dir);
    }
    // Windows: USERPROFILE (a Git Bash HOME may be `/c/Users/...`)
    let home = if cfg!(windows) { std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) } else { std::env::var_os("HOME") };
    let home = home.map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = home.join(".config").join("farsh");
    // farsh kept them in %APPDATA%\farsh at first: they move here once
    #[cfg(windows)]
    if !dir.exists()
        && let Some(old) = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("farsh")).filter(|p| p.is_dir())
    {
        let _ = std::fs::create_dir_all(dir.parent().unwrap_or(&home));
        let _ = std::fs::rename(old, &dir);
    }
    dir
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
