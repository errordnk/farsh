//! Far's folder shortcuts (shortcuts.cpp): ten of them, kept in the
//! profile. Commands > Folder shortcuts is Shortcuts::Configure: one line a
//! shortcut, `Ctrl+Alt+0 │ C:\path` (`<none>` when it is not set), Ins sets
//! it to the active panel's folder, Del clears it, F4 edits its title and
//! path (EditItemImpl: 60 by 10), Enter goes there. Far jumps with
//! RightCtrl+0..9, which a terminal cannot tell from the left one: here it
//! is Ctrl+Alt+0..9; Ctrl+Shift+0..9 sets one, as in Far. A key has one
//! folder, not a list.

use std::path::{Path, PathBuf};

use crate::profile;
use crate::vmenu::{Entry, VMenu};

const FILE: &str = "shortcuts";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shortcut {
    pub title: String,
    pub folder: String,
}

pub struct Shortcuts {
    pub items: [Option<Shortcut>; 10],
    file: PathBuf,
}

impl Shortcuts {
    pub fn load() -> Self {
        Self::load_from(profile::path(FILE))
    }

    pub fn load_from(file: PathBuf) -> Self {
        let mut items: [Option<Shortcut>; 10] = Default::default();
        for (slot, line) in items.iter_mut().zip(std::fs::read_to_string(&file).unwrap_or_default().lines()) {
            let f = profile::fields(line);
            if let [title, folder] = &f[..]
                && !(title.is_empty() && folder.is_empty())
            {
                *slot = Some(Shortcut { title: title.clone(), folder: folder.clone() });
            }
        }
        Self { items, file }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let lines: Vec<String> = self
            .items
            .iter()
            .map(|s| s.as_ref().map(|s| profile::join(&[&s.title, &s.folder])).unwrap_or_default())
            .collect();
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.file, lines.join("\n") + "\n")
    }

    pub fn set(&mut self, index: usize, folder: &Path) {
        self.items[index] = Some(Shortcut { title: String::new(), folder: folder.display().to_string() });
        let _ = self.save();
    }

    /// The folder of shortcut `index`, with environment variables expanded.
    pub fn folder(&self, index: usize) -> Option<PathBuf> {
        self.items[index].as_ref().filter(|s| !s.folder.is_empty()).map(|s| PathBuf::from(expand(&s.folder)))
    }

    /// Configure's menu (MFolderShortcutsTitle, Ins Del F4 at the bottom).
    pub fn menu(&self, select: usize) -> VMenu {
        let entries = (0..10).map(|i| Entry::new(self.label(i))).collect();
        let mut menu = VMenu::with("Folder shortcuts", entries);
        menu.max_height_margin = Some(4);
        menu.bottom = "Ins, Del, F4".into();
        menu.select(select);
        menu
    }

    /// MakeListName: `Ctrl+Alt+&N │ name`.
    fn label(&self, index: usize) -> String {
        let name = match &self.items[index] {
            None => "<none>".to_string(),
            Some(s) if !s.title.is_empty() => expand(&s.title),
            Some(s) => crate::dialog::escape_ampersands(&expand(&s.folder)),
        };
        format!("Ctrl+Alt+&{index} \u{2502} {name}")
    }
}

/// EditItemImpl's DialogBuilder: Title and Path, 50 wide each, OK and Cancel (60 by 10).
pub fn edit_dialog(shortcut: &Shortcut, index: usize) -> crate::dialog::Dialog {
    use crate::dialog::{self, at, span, Item};
    let items = vec![
        Item::Label("Title:".into()),
        dialog::line("title", &shortcut.title, true),
        Item::Label("Path:".into()),
        dialog::line("path", &shortcut.folder, true),
        Item::Separator(None),
        Item::Buttons(vec![dialog::button("OK", "ok", true), dialog::button("Cancel", "cancel", false)]),
    ];
    let places = vec![at(5, 2), span(5, 3, 54), at(5, 4), span(5, 5, 54), at(-1, 6), at(-1, 7)];
    crate::dialog::Dialog::placed("Folder shortcuts".into(), (60, 10), items, places, 1, None, crate::dialog::Action::Shortcut(index))
}

/// os::env::expand: `%NAME%` on Windows, `$NAME` / `${NAME}` elsewhere.
pub fn expand(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(if cfg!(windows) { '%' } else { '$' }) {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let (name, tail) = if cfg!(windows) {
            match after.find('%') {
                Some(end) => (&after[..end], &after[end + 1..]),
                None => ("", after),
            }
        } else if let Some(braced) = after.strip_prefix('{') {
            match braced.find('}') {
                Some(end) => (&braced[..end], &braced[end + 1..]),
                None => ("", after),
            }
        } else {
            let end = after.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(after.len());
            (&after[..end], &after[end..])
        };
        match std::env::var(name).ok().filter(|_| !name.is_empty()) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[start..rest.len() - tail.len()]),
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_save_and_label() {
        let file = std::env::temp_dir().join(format!("farsh-shortcuts-{}", std::process::id()));
        let mut shortcuts = Shortcuts::load_from(file.clone());
        shortcuts.set(3, Path::new("/tmp/a&b"));
        let shortcuts = Shortcuts::load_from(file.clone());
        assert_eq!(shortcuts.label(3), "Ctrl+Alt+&3 \u{2502} /tmp/a&&b");
        assert_eq!(shortcuts.label(0), "Ctrl+Alt+&0 \u{2502} <none>");
        assert_eq!(shortcuts.folder(3), Some(PathBuf::from("/tmp/a&b")));
        std::fs::remove_file(file).unwrap();
    }
}
