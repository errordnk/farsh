//! The key bar's four sets:
//!
//! - [`Set::Main`]: the panels (Far's, with the Shift, Ctrl and Alt variants).
//! - [`Set::Viewer`]: F3.
//! - [`Set::QuickView`]: Ctrl+Q, where the file panel keeps the focus: the keys
//!   that take the file under its cursor into a viewer or an editor, and leave.
//! - [`Set::Editor`]: F4.
//!
//! Only what works has a label: the viewer and the editor grow theirs as the
//! rest of Far's keys come (search, go to, code page...).

use crate::modkeys::{ALT, CTRL, SHIFT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Set {
    Main,
    Viewer,
    QuickView,
    Editor,
}

/// What the Shift, Ctrl and Alt held make of the bar: 0 none, 1 Shift, 2 Ctrl, 3 Alt.
fn variant(held: u8) -> usize {
    match held & (SHIFT | CTRL | ALT) {
        SHIFT => 1,
        CTRL => 2,
        ALT => 3,
        _ => 0,
    }
}

/// The twelve labels of `set` with `held` modifiers down.
pub fn labels(set: Set, held: u8, text: bool) -> [&'static str; 12] {
    let msg = crate::help::msg;
    let mut labels = [""; 12];
    match (set, variant(held)) {
        (Set::Main, _) => {
            for (slot, label) in labels.iter_mut().zip(crate::panels::key_bar_labels(held)) {
                *slot = label;
            }
        }
        (Set::Viewer, 0) => {
            labels[2] = msg("Quit", "Выход");
            // a document or a picture is not text: nothing to edit
            if text {
                labels[5] = msg("Edit", "Редакт");
            }
            labels[9] = msg("Quit", "Выход");
            labels[11] = msg("Screen", "Экраны");
        }
        (Set::QuickView, 0) => {
            labels[0] = msg("Help", "Помощь");
            labels[2] = msg("View", "Просм");
            if text {
                labels[3] = msg("Edit", "Редакт");
            }
            labels[8] = msg("ConfMn", "Меню");
            labels[9] = msg("Quit", "Выход");
            labels[11] = msg("Screen", "Экраны");
        }
        (Set::Editor, 0) => {
            labels[1] = msg("Save", "Сохран");
            labels[5] = msg("View", "Просм");
            labels[9] = msg("Quit", "Выход");
            labels[11] = msg("Screen", "Экраны");
        }
        // Shift+F10 saves and quits
        (Set::Editor, 1) => labels[9] = msg("SaveQuit", "СохрВых"),
        _ => {}
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(labels: &[&str; 12]) -> Vec<usize> {
        labels.iter().enumerate().filter(|(_, l)| !l.is_empty()).map(|(i, _)| i + 1).collect()
    }

    #[test]
    fn the_three_sets_have_their_own_keys() {
        assert_eq!(shown(&labels(Set::Viewer, 0, true)), vec![3, 6, 10, 12]);
        assert_eq!(shown(&labels(Set::Viewer, 0, false)), vec![3, 10, 12], "no Edit for a picture or a document");
        assert_eq!(shown(&labels(Set::QuickView, 0, true)), vec![1, 3, 4, 9, 10, 12]);
        assert_eq!(shown(&labels(Set::QuickView, 0, false)), vec![1, 3, 9, 10, 12]);
        assert_eq!(shown(&labels(Set::Editor, 0, true)), vec![2, 6, 10, 12]);
        assert!(labels(Set::Editor, SHIFT, true)[9].starts_with("Save"), "Shift+F10");
        assert_eq!(labels(Set::Main, 0, true)[4], crate::help::msg("Copy", "Копир"));
        // the main set is the panels' own
        assert!(shown(&labels(Set::Main, 0, true)).len() >= 10);
    }
}
