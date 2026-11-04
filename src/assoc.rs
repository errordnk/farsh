//! Far's file associations (filetype.cpp), kept in the profile. Each has
//! masks, a description and six commands, each on or off: Enter,
//! Ctrl+PgDn, F3, Alt+F3, F4, Alt+F4. Commands > File associations is
//! EditFileTypes: `description │ masks`, Ins adds one before the cursor,
//! Del deletes, Enter/F4 edits (EditTypeRecord: 76 by 23), Ctrl+Up/Down
//! move. On a file, the commands of every association whose masks match
//! (ProcessLocalFileTypes) are offered: one is run at once, several make
//! the "Select association" menu.

use std::path::PathBuf;

use crate::masks::Masks;
use crate::profile;
use crate::vmenu::{Entry, VMenu};

const FILE: &str = "associations";

/// The six commands, in Far's FILETYPE_MODE order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Exec,
    AltExec,
    View,
    AltView,
    Edit,
    AltEdit,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assoc {
    pub masks: String,
    pub description: String,
    /// The command of each mode and whether it is on.
    pub commands: [(String, bool); 6],
}

pub struct Assocs {
    pub items: Vec<Assoc>,
    file: PathBuf,
}

impl Assocs {
    pub fn load() -> Self {
        Self::load_from(profile::path(FILE))
    }

    pub fn load_from(file: PathBuf) -> Self {
        let items = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let f = profile::fields(line);
                if f.len() != 14 {
                    return None;
                }
                let mut assoc = Assoc { masks: f[0].clone(), description: f[1].clone(), ..Assoc::default() };
                for (i, slot) in assoc.commands.iter_mut().enumerate() {
                    *slot = (f[2 + 2 * i].clone(), f[3 + 2 * i] == "1");
                }
                Some(assoc)
            })
            .collect();
        Self { items, file }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let lines: Vec<String> = self
            .items
            .iter()
            .map(|a| {
                let mut fields: Vec<&str> = vec![&a.masks, &a.description];
                for (command, on) in &a.commands {
                    fields.push(command);
                    fields.push(if *on { "1" } else { "0" });
                }
                profile::join(&fields)
            })
            .collect();
        if lines.is_empty() {
            return match std::fs::remove_file(&self.file) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.file, lines.join("\n") + "\n")
    }

    /// FillFileTypesMenu: the descriptions padded to the longest, `│`, the masks.
    pub fn menu(&self, select: usize) -> VMenu {
        let width = self.items.iter().map(|a| crate::vmenu::strip_hot(&a.description).chars().count()).max().unwrap_or(0);
        let entries = self
            .items
            .iter()
            .map(|a| {
                let pad = width - crate::vmenu::strip_hot(&a.description).chars().count();
                Entry::new(format!("{}{} \u{2502} {}", a.description, " ".repeat(pad), a.masks))
            })
            .collect();
        let mut menu = VMenu::with("File associations", entries);
        menu.max_height_margin = Some(4);
        menu.bottom = "Ins, Del, F4, Ctrl+Up, Ctrl+Down".into();
        menu.select(select);
        menu
    }

    /// The commands for `name` in `mode` (masks matching, the command on and
    /// not empty), each with what the menu shows: the description or the command.
    pub fn commands_for(&self, name: &str, mode: Mode) -> Vec<(String, String)> {
        self.items
            .iter()
            .filter(|a| Masks::new(&a.masks).is_some_and(|m| m.check(name)))
            .filter_map(|a| {
                let (command, on) = &a.commands[mode as usize];
                (*on && !command.trim().is_empty()).then(|| (a.description.clone(), command.clone()))
            })
            .collect()
    }
}

const COMMAND_IDS: [(&str, &str); 6] = [("c0", "e0"), ("c1", "e1"), ("c2", "e2"), ("c3", "e3"), ("c4", "e4"), ("c5", "e5")];
const COMMAND_LABELS: [&str; 6] = [
    "E&xecute command (used for Enter):",
    "Exec&ute command (used for Ctrl+PgDn):",
    "&View command (used for F3):",
    "V&iew command (used for Alt+F3):",
    "&Edit command (used for F4):",
    "Edit comm&and (used for Alt+F4):",
];

/// EditTypeRecord: 76 by 23; a command's line is enabled by its check box.
pub fn edit_dialog(assoc: &Assoc, index: usize, new: bool) -> crate::dialog::Dialog {
    use crate::dialog::{self, at, span, Item, Place};
    let mut items_places: Vec<(Item, Place)> = vec![
        (Item::Label("A file &mask or several file masks:".into()), at(5, 2)),
        (dialog::line("masks", &assoc.masks, true), span(5, 3, 70)),
        (Item::Label("&Description of the association:".into()), at(5, 4)),
        (dialog::line("description", &assoc.description, false), span(5, 5, 70)),
        (Item::Separator(None), at(-1, 6)),
    ];
    for (i, ((check, line), label)) in COMMAND_IDS.iter().zip(COMMAND_LABELS).enumerate() {
        let (command, on) = &assoc.commands[i];
        let on = new || *on;
        items_places.push((Item::Check { id: check, label, checked: on, enabled: true }, at(5, 7 + 2 * i as u16)));
        items_places.push((dialog::line(line, command, false), span(9, 8 + 2 * i as u16, 70)));
    }
    items_places.push((Item::Separator(None), at(-1, 19)));
    items_places.push((Item::Buttons(vec![dialog::button("OK", "ok", true), dialog::button("Cancel", "cancel", false)]), at(-1, 20)));
    let (items, places): (Vec<Item>, Vec<Place>) = items_places.into_iter().unzip();
    let mut dialog = crate::dialog::Dialog::placed("Edit file associations".into(), (76, 23), items, places, 1, Some(hook), crate::dialog::Action::Assoc { index, new });
    for i in 0..6 {
        dialog.enable(6 + 2 * i, new || assoc.commands[i].1);
    }
    dialog
}

/// EditTypeRecordDlgProc: a check box enables its line; OK wants good masks.
fn hook(dialog: &mut crate::dialog::Dialog, event: crate::dialog::Event) {
    use crate::dialog::Event;
    match event {
        Event::Toggled(id) => {
            if let Some(i) = COMMAND_IDS.iter().position(|(check, _)| *check == id)
                && let Some(index) = dialog.index_of(id)
            {
                let on = dialog.check_state(index) == 1;
                dialog.enable(6 + 2 * i, on);
            }
        }
        Event::Closing("ok") => {
            let masks = dialog.index_of("masks").map(|i| dialog.text_of(i)).unwrap_or_default();
            if Masks::new(&masks).is_none() {
                dialog.message_over("Warning", &["File-mask string contains errors"], &[("OK", "ok")]);
            }
        }
        _ => {}
    }
}

/// The association the dialog's OK made.
pub fn from_values(values: &crate::dialog::Values) -> Assoc {
    let mut assoc = Assoc { masks: values.text("masks"), description: values.text("description"), ..Assoc::default() };
    for (i, (check, line)) in COMMAND_IDS.iter().enumerate() {
        assoc.commands[i] = (values.text(line), values.checked(check));
    }
    assoc
}

/// The "Select association" menu.
pub fn select_menu(labels: Vec<String>) -> VMenu {
    let mut menu = VMenu::with("Select association", labels.into_iter().map(Entry::new).collect());
    menu.max_height_margin = Some(4);
    menu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_matched_and_listed() {
        let file = std::env::temp_dir().join(format!("farsh-assoc-{}", std::process::id()));
        let mut assocs = Assocs::load_from(file.clone());
        let mut assoc = Assoc { masks: "*.txt,*.md".into(), description: "Text".into(), ..Assoc::default() };
        assoc.commands[Mode::Exec as usize] = ("notepad !.!".into(), true);
        assoc.commands[Mode::View as usize] = ("less !.!".into(), false);
        assocs.items.push(assoc);
        assocs.save().unwrap();
        let assocs = Assocs::load_from(file.clone());
        assert_eq!(assocs.commands_for("a.md", Mode::Exec), vec![("Text".to_string(), "notepad !.!".to_string())]);
        assert!(assocs.commands_for("a.md", Mode::View).is_empty(), "off");
        assert!(assocs.commands_for("a.rs", Mode::Exec).is_empty());
        assert_eq!(assocs.menu(0).entries[0].text, "Text \u{2502} *.txt,*.md");
        std::fs::remove_file(file).unwrap();
    }
}
