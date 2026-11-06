//! far:config, Far's configuration editor (config.cpp's AdvancedConfig of
//! Far 3.0.6446): every setting of Far in a list box, `name │type   │value`,
//! a `*` before the ones changed. Enter or F4 changes one (a boolean flips,
//! a 3-state goes round, a number or text opens its dialog with OK, Reset
//! and Cancel), Shift+F4 edits a number in hex, Alt+F4 in binary, Del brings
//! the default back, Ctrl+H shows only the changed ones. The settings farsh
//! does not use yet (the editor, plugins...) are shown with Far's defaults,
//! disabled. Shift+F1 is the setting's help (or its group's).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::builder::Builder;
use crate::colors::{bg, fg};
use crate::dialog::{Action, Dialog};
use crate::farconfig_data::FAR;
use crate::grid::Grid;
use crate::settings::{self, Def};

/// A setting's type (Far's GetType).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Bool,
    Three,
    Int,
    Str,
}

impl Ty {
    fn name(self) -> &'static str {
        match self {
            Ty::Bool => "boolean",
            Ty::Three => "3-state",
            Ty::Int => "integer",
            Ty::Str => "string",
        }
    }
}

/// How a value is edited (Far's edit_mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Hex,
    Bin,
}

const TITLE: &str = "Configuration editor";
const BOTTOM: &str = "Shift+F1 F4 Shift+F4 Alt+F4 Del Ctrl+H";

/// Whether farsh uses setting `index` of the list.
fn ours(index: usize) -> bool {
    settings::default(FAR[index].0).is_some()
}

/// The system's ANSI code page (`@acp` in the table).
fn acp() -> i64 {
    #[cfg(windows)]
    {
        i64::from(unsafe { windows_sys::Win32::Globalization::GetACP() })
    }
    #[cfg(not(windows))]
    {
        65001
    }
}

fn far_default(index: usize) -> &'static str {
    FAR[index].2
}

fn far_int(index: usize) -> i64 {
    match far_default(index) {
        "@acp" => acp(),
        text => text.parse().unwrap_or(0),
    }
}

fn three_name(n: i64) -> &'static str {
    match n {
        0 => "false",
        1 => "true",
        _ => "other",
    }
}

/// The value column: Far's toString and ExInfo (` = 0x…` for a number).
fn value_text(index: usize) -> String {
    let (key, ty, _) = FAR[index];
    let mine = ours(index);
    match ty {
        Ty::Bool => {
            let on = if mine { settings::bool(key) } else { far_default(index) == "true" };
            on.to_string()
        }
        Ty::Three => three_name(if mine { settings::int(key) } else { far_int(index) }).to_string(),
        Ty::Int => {
            let n = if mine { settings::int(key) } else { far_int(index) };
            format!("{n} = 0x{:X}", n as u64)
        }
        Ty::Str => {
            if mine {
                settings::string(key)
            } else {
                far_default(index).to_string()
            }
        }
    }
}

/// MakeListItem: the name padded to 42, the type fitted to 7.
fn row_text(index: usize) -> String {
    let (key, ty, _) = FAR[index];
    format!("{key:<42}\u{2502}{:<7}\u{2502}{}", ty.name(), value_text(index))
}

fn changed(index: usize) -> bool {
    ours(index) && !settings::is_default(FAR[index].0)
}

pub enum Outcome {
    Open,
    Closed,
    /// A setting changed: what depends on the settings follows.
    Changed,
    /// A value's dialog, to show over the list.
    Edit(Dialog),
    /// Shift+F1: the help topics to try, in turn (the setting's own, then
    /// its group's `…Settings`).
    Help(Vec<String>),
}

pub struct FarConfig {
    /// The rows shown (indices into FAR).
    shown: Vec<usize>,
    selected: usize,
    top: usize,
    rows: usize,
    hide_unchanged: bool,
    /// The list box's frame as last painted: left, top, right, bottom.
    frame: (u16, u16, u16, u16),
}

impl FarConfig {
    pub fn new() -> Self {
        let mut config = Self { shown: vec![], selected: 0, top: 0, rows: 1, hide_unchanged: false, frame: (0, 0, 0, 0) };
        config.refill(None);
        config
    }

    /// The rows again (after Ctrl+H or a change), on `keep` if it is still shown.
    fn refill(&mut self, keep: Option<usize>) {
        let hide = self.hide_unchanged;
        self.shown = (0..FAR.len()).filter(|&i| !hide || changed(i)).collect();
        let at = keep.and_then(|k| self.shown.iter().position(|&i| i == k));
        self.selected = at.unwrap_or(0);
        if self.shown.get(self.selected).is_some_and(|&i| !ours(i)) {
            self.step(1);
        }
    }

    fn current(&self) -> Option<usize> {
        self.shown.get(self.selected).copied().filter(|&i| ours(i))
    }

    /// One row on, over the disabled ones (Far's list skips them).
    fn step(&mut self, delta: isize) {
        let mut at = self.selected as isize;
        loop {
            at += delta;
            if at < 0 || at >= self.shown.len() as isize {
                return;
            }
            if ours(self.shown[at as usize]) {
                self.selected = at as usize;
                return;
            }
        }
    }

    /// To row `at` or the nearest enabled one, looking in `delta`'s way first.
    fn go(&mut self, at: usize, delta: isize) {
        let at = at.min(self.shown.len().saturating_sub(1));
        if self.shown.get(at).is_some_and(|&i| ours(i)) {
            self.selected = at;
            return;
        }
        let from = self.selected;
        self.selected = at;
        self.step(delta);
        if self.selected == at {
            self.step(-delta);
        }
        if self.selected == at {
            self.selected = from;
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let m = key.modifiers;
        match key.code {
            KeyCode::Esc | KeyCode::F(10) if m.is_empty() => return Outcome::Closed,
            KeyCode::Up if m.is_empty() => self.step(-1),
            KeyCode::Down if m.is_empty() => self.step(1),
            KeyCode::Home if m.is_empty() => self.go(0, 1),
            KeyCode::End if m.is_empty() => self.go(usize::MAX, -1),
            // VMenu's PgUp/PgDn: a whole box of rows
            KeyCode::PageUp if m.is_empty() => self.go(self.selected.saturating_sub(self.rows), -1),
            KeyCode::PageDown if m.is_empty() => self.go(self.selected + self.rows, 1),
            KeyCode::F(1) if m == KeyModifiers::SHIFT => {
                if let Some(&index) = self.shown.get(self.selected) {
                    let key = FAR[index].0;
                    let group = key.rsplit_once('.').map_or(key, |(group, _)| group);
                    return Outcome::Help(vec![key.to_string(), format!("{group}Settings")]);
                }
            }
            KeyCode::Enter | KeyCode::F(4) if m.is_empty() => return self.edit(Mode::Normal),
            KeyCode::F(4) if m == KeyModifiers::SHIFT => return self.edit(Mode::Hex),
            KeyCode::F(4) if m == KeyModifiers::ALT => return self.edit(Mode::Bin),
            KeyCode::Delete if m.is_empty() => {
                if let Some(index) = self.current() {
                    settings::reset(FAR[index].0);
                    self.refill(Some(index));
                    return Outcome::Changed;
                }
            }
            KeyCode::Char('h') if m == KeyModifiers::CONTROL => {
                self.hide_unchanged = !self.hide_unchanged;
                let keep = self.current();
                self.refill(keep);
            }
            _ => {}
        }
        Outcome::Open
    }

    /// The mouse: the wheel moves, a click picks a row, a double click (or
    /// a click on the picked one) changes it, as Enter does.
    pub fn click(&mut self, event: crossterm::event::MouseEvent) -> Outcome {
        use crossterm::event::{MouseButton, MouseEventKind};
        let (left, top, right, bottom) = self.frame;
        match event.kind {
            MouseEventKind::ScrollUp => self.step(-1),
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let inside = event.column > left && event.column < right && event.row > top && event.row < bottom;
                if inside {
                    let at = self.top + (event.row - top - 1) as usize;
                    if self.shown.get(at).is_some_and(|&i| ours(i)) {
                        if at == self.selected {
                            return self.edit(Mode::Normal);
                        }
                        self.selected = at;
                    }
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    /// FARConfigItem::Edit: a flag flips, a 3-state goes round, the rest
    /// open their dialog.
    fn edit(&mut self, mode: Mode) -> Outcome {
        let Some(index) = self.current() else { return Outcome::Open };
        let (key, ty, _) = FAR[index];
        match ty {
            Ty::Bool => settings::set_bool(key, !settings::bool(key)),
            Ty::Three => settings::set_int(key, (settings::int(key) + 1) % 3),
            Ty::Int | Ty::Str => return Outcome::Edit(edit_dialog(key, ty, mode)),
        }
        self.refill(Some(index));
        Outcome::Changed
    }

    /// After a value's dialog: the list as the settings are now.
    pub fn after_edit(&mut self) {
        let keep = self.current();
        self.refill(keep);
    }

    /// AdvancedConfig's dialog: max(ScrX-4, 60) by max(ScrY-2, 20) in the
    /// middle, the list box from (3, 1) to (W-4, H-2) in it.
    pub fn paint(&mut self, grid: &mut Grid) {
        let (cols, rows) = (grid.cols, grid.rows);
        let w = cols.saturating_sub(5).max(60);
        let h = rows.saturating_sub(3).max(20);
        let x = cols.saturating_sub(w) / 2;
        let y = rows.saturating_sub(h) / 2;
        let (text, back) = (fg("Dialog.Text"), bg("Dialog.Text"));
        grid.fill_with(x, y, w, h, text, back);
        crate::palette::drop_shadow(grid, x, y, w, h);

        let (left, top, right, bottom) = (x + 3, y + 1, x + w - 4, y + h - 2);
        self.frame = (left, top, right, bottom);
        let inner = (right - left - 1) as usize;
        let (box_fg, box_bg) = (fg("Dialog.List.Box"), bg("Dialog.List.Box"));
        let line = "\u{2550}".repeat(inner);
        grid.put(left, top, &format!("\u{2554}{line}\u{2557}"), box_fg, box_bg);
        grid.put(left, bottom, &format!("\u{255A}{line}\u{255D}"), box_fg, box_bg);
        let title = crate::lang::tr(TITLE);
        for (label, row, name) in [(title, top, "Dialog.List.Title"), (BOTTOM, bottom, "Dialog.List.Title")] {
            let shown: String = label.chars().take(inner.saturating_sub(2)).collect();
            let at = left + 1 + (inner.saturating_sub(shown.chars().count() + 2) / 2) as u16;
            grid.put(at, row, &format!(" {shown} "), fg(name), bg(name));
        }

        self.rows = (bottom - top - 1) as usize;
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + self.rows {
            self.top = self.selected + 1 - self.rows;
        }
        self.top = self.top.min(self.shown.len().saturating_sub(self.rows));
        let text_x = left + 3;
        let text_w = (right - 1).saturating_sub(text_x) as usize;
        for row in 0..self.rows {
            let ry = top + 1 + row as u16;
            grid.put(left, ry, "\u{2551}", box_fg, box_bg);
            grid.put(right, ry, "\u{2551}", box_fg, box_bg);
            let at = self.top + row;
            let Some(&index) = self.shown.get(at) else {
                grid.fill_with(left + 1, ry, inner as u16, 1, fg("Dialog.List.Text"), bg("Dialog.List.Text"));
                continue;
            };
            let name = if at == self.selected && ours(index) {
                "Dialog.List.Text.Selected"
            } else if ours(index) {
                "Dialog.List.Text"
            } else {
                "Dialog.List.Disabled"
            };
            let (f, b) = (fg(name), bg(name));
            grid.fill_with(left + 1, ry, inner as u16, 1, f, b);
            if changed(index) {
                grid.put(left + 1, ry, "*", f, b);
            }
            let text = row_text(index);
            let shown: String = text.chars().take(text_w).collect();
            grid.put(text_x, ry, &shown, f, b);
            // VMenu marks a cut line with » before the edge
            if text.chars().count() > text_w {
                grid.put(right - 1, ry, "\u{00BB}", f, b);
            }
        }
        if self.shown.len() > self.rows && self.rows >= 2 {
            let bar = crate::vmenu::scroll_bar(self.rows, self.top, self.top + self.rows, self.shown.len());
            let (sf, sb) = (fg("Dialog.List.Scrollbar"), bg("Dialog.List.Scrollbar"));
            for (i, c) in bar.chars().enumerate() {
                grid.put(right, top + 1 + i as u16, &c.to_string(), sf, sb);
            }
        }
    }
}

/// A number's or a text's dialog (FARConfigItem::Edit): `Name (type):`, the
/// field (IntOption::Edit: 20 wide, hex 0x and 16 digits, binary 64 with
/// the bit numbers under it; StringOption: 40), OK, Reset, Cancel.
fn edit_dialog(key: &'static str, ty: Ty, mode: Mode) -> Dialog {
    let mut b = Builder::new("");
    b.text_owned(format!("{key} ({}):", ty.name()));
    let mode = if ty == Ty::Int { mode } else { Mode::Normal };
    match (ty, mode) {
        (Ty::Int, Mode::Normal) => {
            b.free_edit("value", &settings::int(key).to_string(), 20);
        }
        (Ty::Int, Mode::Hex) => {
            // AddHexEditField: the 16 digits laid over the 0x mask, as Far does
            b.masked("value", &format!("0x{}", "H".repeat(16)), &format!("{:016X}", settings::int(key) as u64));
        }
        (Ty::Int, Mode::Bin) => {
            b.masked("value", &"B".repeat(64), &format!("{:064b}", settings::int(key) as u64));
            let high: String = (0..64).map(|i| char::from(b'0' + ((63 - i) / 10) as u8)).collect();
            let low: String = (0..64).map(|i| char::from(b'0' + ((63 - i) % 10) as u8)).collect();
            b.text_disabled(high);
            b.text_disabled(low);
        }
        _ => {
            b.free_edit("value", &settings::string(key), 41);
        }
    }
    b.buttons(&[("OK", "ok"), ("&Reset", "reset"), ("Cancel", "cancel")]);
    b.build_as(|_| Action::FarConfig { key, mode })
}

/// OK or Reset in a value's dialog.
pub fn accept(key: &'static str, mode: Mode, id: &str, value: &str) {
    match id {
        "reset" => settings::reset(key),
        "ok" => match settings::default(key) {
            Some(Def::Str(_)) => settings::set_string(key, value),
            Some(_) => {
                let digits = value.trim();
                let parsed = match mode {
                    Mode::Normal => digits.parse::<i64>().ok().or_else(|| digits.parse::<u64>().ok().map(|u| u as i64)),
                    Mode::Hex => u64::from_str_radix(&digits.trim_start_matches("0x").replace(' ', ""), 16).ok().map(|u| u as i64),
                    Mode::Bin => u64::from_str_radix(&digits.replace(' ', ""), 2).ok().map(|u| u as i64),
                };
                if let Some(n) = parsed {
                    settings::set_int(key, n);
                }
            }
            None => {}
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_list_rows_and_farsh_settings() {
        // every setting of farsh is in Far's list, with Far's type
        for (key, def) in settings::defaults() {
            let Some(&(_, ty, _)) = FAR.iter().find(|(k, _, _)| k == key) else { panic!("{key} is not Far's") };
            let want = match def {
                Def::Bool(_) => Ty::Bool,
                Def::Three(_) => Ty::Three,
                Def::Int(_) => Ty::Int,
                Def::Str(_) => Ty::Str,
            };
            assert_eq!(ty, want, "{key}");
        }
        let at = FAR.iter().position(|(k, _, _)| *k == "Cmdline.PromptFormat").unwrap();
        assert_eq!(row_text(at), format!("{:<42}\u{2502}string \u{2502}$p$g", "Cmdline.PromptFormat"));
        let at = FAR.iter().position(|(k, _, _)| *k == "Viewer.TabSize").unwrap();
        assert!(row_text(at).ends_with("\u{2502}integer\u{2502}8 = 0x8"));
        // of the editor's settings only the external editor is farsh's (its
        // internal editor is to be fresh, with fresh's own settings)
        let at = |key: &str| FAR.iter().position(|(k, _, _)| *k == key).unwrap();
        assert!(!ours(at("Editor.TabSize")), "the internal editor's settings are shown disabled");
        assert!(ours(at("Editor.UseExternalEditor")) && ours(at("Editor.ExternalEditorName")));
    }

    #[test]
    fn keys_skip_the_disabled_rows() {
        let mut config = FarConfig::new();
        assert_eq!(FAR[config.shown[config.selected]].0, "Cmdline.AutoComplete");
        // from the last dialog setting down to the next enabled one, over the
        // internal editor's twelve disabled rows to the external editor's command
        let before = FAR.iter().position(|(k, _, _)| *k == "Dialog.MouseButton").unwrap();
        config.selected = config.shown.iter().position(|&i| i == before).unwrap();
        config.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let next = FAR[config.shown[config.selected]].0;
        assert_eq!(next, "Editor.ExternalEditorName");
        assert!(ours(config.shown[config.selected]));
        assert!(matches!(config.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)), Outcome::Closed));
    }

    #[test]
    fn values_change_reset_and_ctrl_h_shows_the_changed() {
        let key = "Panel.Info.InfoUserNameFormat";
        accept(key, Mode::Normal, "ok", "3");
        assert_eq!(settings::int(key), 3);
        accept(key, Mode::Hex, "ok", "0x00000000000000A ");
        assert_eq!(settings::int(key), 10);
        accept(key, Mode::Bin, "ok", &format!("{:064b}", 5));
        assert_eq!(settings::int(key), 5);
        let at = FAR.iter().position(|(k, _, _)| *k == key).unwrap();
        assert!(changed(at));
        let mut config = FarConfig::new();
        config.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
        assert!(config.shown.contains(&at) && config.shown.iter().all(|&i| changed(i)));
        config.selected = config.shown.iter().position(|&i| i == at).unwrap();
        config.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(settings::int(key), 0, "Del: Far's default");
        assert!(!config.shown.contains(&at), "no longer changed");
    }

    #[test]
    fn every_kind_of_dialog_paints() {
        for (key, ty, mode) in [
            ("Cmdline.PromptFormat", Ty::Str, Mode::Normal),
            ("Viewer.TabSize", Ty::Int, Mode::Hex),
            ("Viewer.TabSize", Ty::Int, Mode::Bin),
        ] {
            let mut dialog = edit_dialog(key, ty, mode);
            let mut grid = Grid::new(100, 30);
            dialog.paint(&mut grid);
        }
    }

    #[test]
    fn a_number_dialog_has_ok_reset_cancel() {
        let mut dialog = edit_dialog("Viewer.TabSize", Ty::Int, Mode::Normal);
        let mut grid = Grid::new(100, 30);
        dialog.paint(&mut grid);
        let text: String = (0..30).map(|r| grid.row(r).iter().map(|c| c.text.clone()).collect::<String>() + "\n").collect();
        assert!(text.contains("Viewer.TabSize (integer):"), "{text}");
        assert!(text.contains("{ OK } [ Reset ] [ Cancel ]"), "{text}");
    }
}
