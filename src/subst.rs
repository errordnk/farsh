//! Far's metasymbols (fnparce.cpp's ProcessMetasymbol) for Ctrl+G: each
//! file's command line is made from what the user typed. `!#` and `!^`
//! (`![`, `!]`) switch to the passive (active, left, right) panel for what
//! follows. Not done: the list files `!@!`/`!$!` and the input `!?title?init!`,
//! which are left as they are.

use std::path::{Path, PathBuf};

/// A panel as the metasymbols see it.
pub struct Subject {
    pub dir: PathBuf,
    /// The file (Far's Default().Normal.Name).
    pub name: String,
    /// Its 8.3 name, "" when it has none.
    pub short: String,
    /// The selected names (`!&`).
    pub list: Vec<String>,
}

impl Subject {
    fn short(&self) -> &str {
        if self.short.is_empty() { &self.name } else { &self.short }
    }
}

/// Far's name_ext: the extension from the last dot (a leading dot is not one).
fn name_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(at) if at > 0 => (&name[..at], &name[at..]),
        _ => (name, ""),
    }
}

fn extension(name: &str) -> &str {
    name_ext(name).1.trim_start_matches('.')
}

/// The extension of `name` without its dot ("" when it has none).
pub fn extension_of(name: &str) -> &str {
    extension(name)
}

fn with_slash(dir: &Path) -> String {
    let mut text = dir.display().to_string();
    if !text.ends_with(['\\', '/']) {
        text.push(std::path::MAIN_SEPARATOR);
    }
    text
}

fn quote(name: &str) -> String {
    format!("\"{name}\"")
}

fn quote_space(name: &str) -> String {
    if name.contains(' ') { quote(name) } else { name.to_string() }
}

/// Takes `token` off the front of `rest` when it is there.
fn eat(rest: &mut &str, token: &str) -> bool {
    match rest.strip_prefix(token) {
        Some(tail) => {
            *rest = tail;
            true
        }
        None => false,
    }
}

/// The command line for one file. `right_active` tells which panel `![` and `!]` mean.
pub fn substitute(command: &str, this: &Subject, other: &Subject, right_active: bool) -> String {
    let mut out = String::new();
    let mut passive = false;
    let mut rest = command;
    while !rest.is_empty() {
        let s = if passive { other } else { this };
        let not_input = |tail: &str| !tail.starts_with('?');
        if eat(&mut rest, "!#") {
            passive = true;
        } else if eat(&mut rest, "!^") {
            passive = false;
        } else if eat(&mut rest, "![") {
            passive = right_active;
        } else if eat(&mut rest, "!]") {
            passive = !right_active;
        } else if rest.starts_with("!!") && not_input(&rest[2..]) {
            out.push('!');
            rest = &rest[2..];
        } else if rest.starts_with("!.!") && not_input(&rest[3..]) {
            out.push_str(&s.name);
            rest = &rest[3..];
        } else if eat(&mut rest, "!~") {
            out.push_str(name_ext(s.short()).0);
        } else if eat(&mut rest, "!`~") {
            out.push_str(extension(s.short()));
        } else if eat(&mut rest, "!`") {
            out.push_str(extension(&s.name));
        } else if let Some(tail) = rest.strip_prefix("!&~").or_else(|| rest.strip_prefix("!&")).filter(|t| not_input(t)) {
            let (explicit_quote, no_quote) = (tail.starts_with('Q'), tail.starts_with('q'));
            let names: Vec<String> = if passive { vec![s.short().to_string()] } else { s.list.clone() };
            let names: Vec<String> = names
                .iter()
                .map(|n| if no_quote { quote_space(&n) } else { quote(&n) })
                .collect();
            out.push_str(&names.join(" "));
            rest = &tail[usize::from(explicit_quote || no_quote)..];
        } else if (rest.starts_with("!-!") || rest.starts_with("!+!")) && not_input(&rest[3..]) {
            out.push_str(s.short());
            rest = &rest[3..];
        } else if rest.starts_with("!:") {
            let dir = s.dir.display().to_string();
            let root = match dir.find(':') {
                Some(at) if cfg!(windows) => dir[..=at].to_string(),
                _ => String::new(),
            };
            out.push_str(&root);
            rest = &rest[2..];
        } else if rest.starts_with("!?!") {
            out.push_str(&crate::diz::Diz::read(&s.dir).get(&s.name).unwrap_or_default());
            rest = &rest[3..];
        } else if rest.starts_with("!\\") || rest.starts_with("!/") {
            out.push_str(&with_slash(&s.dir));
            rest = &rest[2..];
        } else if rest.starts_with("!=\\") || rest.starts_with("!=/") {
            let real = std::fs::canonicalize(&s.dir).map(|p| {
                let text = p.display().to_string();
                PathBuf::from(text.strip_prefix(r"\\?\").map(String::from).unwrap_or(text))
            });
            out.push_str(&with_slash(&real.unwrap_or_else(|_| s.dir.clone())));
            rest = &rest[3..];
        } else if rest.starts_with("!?") || rest.starts_with("!@") || rest.starts_with("!$") {
            out.push_str(&rest[..2]);
            rest = &rest[2..];
        } else if rest.starts_with('!') {
            out.push_str(name_ext(&s.name).0);
            rest = &rest[1..];
        } else {
            let c = rest.chars().next().unwrap_or_default();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_metasymbols() {
        let this = Subject { dir: PathBuf::from("/a"), name: "my file.tar.gz".into(), short: String::new(), list: vec!["x y".into(), "z".into()] };
        let other = Subject { dir: PathBuf::from("/b"), name: "other.txt".into(), short: String::new(), list: vec![] };
        let sub = |command: &str| substitute(command, &this, &other, false);
        assert_eq!(sub("echo \"!.!\""), "echo \"my file.tar.gz\"");
        assert_eq!(sub("!"), "my file.tar");
        assert_eq!(sub("!`"), "gz");
        assert_eq!(sub("!!"), "!");
        assert_eq!(sub("!&"), "\"x y\" \"z\"");
        assert_eq!(sub("!&q"), "\"x y\" z");
        assert_eq!(sub("cp !.! !#!\\"), format!("cp my file.tar.gz {}", with_slash(Path::new("/b"))));
        assert_eq!(sub("!#!.! !^!.!"), "other.txt my file.tar.gz");
        assert_eq!(sub("no symbols"), "no symbols");
    }
}
