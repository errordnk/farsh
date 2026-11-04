//! The Options menu over the panels: its dialogs (`crate::options`), the
//! languages, the groups of file masks, the code pages, Save setup.

use serde_json::{Map, Value};

use crate::builder::{self, Settings};
use crate::commands::List;
use crate::dialog::{Action, Dialog, Values};
use crate::menu::Opt;
use crate::vmenu::{Entry, Outcome, VMenu};
use crate::{codepages, lang, options, settings};

use super::{App, Overlay};

/// Far's default groups (ApplyDefaultMaskGroups).
const DEFAULT_MASKS: [(&str, &str); 3] = [
    (
        "arc",
        "*.zip,*.rar,*.[7bgxl]z,*.[bg]zip,*.tar,*.t[agbxl]z,*.z,*.ar[cj],*.r[0-9][0-9],*.a[0-9][0-9],*.bz2,*.cab,*.jar,*.lha,*.lzh,*.ha,*.ac[bei],*.pa[ck],*.rk,*.cpio,*.rpm,*.zoo,*.hqx,*.sit,*.ice,*.uc2,*.ain,*.imp,*.777,*.ufa,*.boa,*.bs[2a],*.sea,*.[ah]pk,*.ddi,*.x2,*.rkv,*.[lw]sz,*.h[ay]p,*.lim,*.sqz,*.chz,*.aa[br],*.msi",
    ),
    ("temp", "*.bak,*.tmp"),
    ("exec", "*.exe,*.cmd,*.bat,*.com,%PATHEXT%"),
];

/// The groups of file masks (`<arc>` in a mask is the group `arc`).
pub fn mask_groups() -> Vec<(String, String)> {
    match settings::value("Masks") {
        Some(Value::Object(map)) => map.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect(),
        _ => DEFAULT_MASKS.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect(),
    }
}

fn save_mask_groups(groups: &[(String, String)]) {
    let map: Map<String, Value> = groups.iter().map(|(n, v)| (n.clone(), Value::from(v.as_str()))).collect();
    settings::set("Masks", Value::Object(map));
}

/// FillMasksMenu: `name (10) │ masks`.
fn masks_menu(select: usize, only: Option<&str>) -> (VMenu, Vec<String>) {
    let groups = mask_groups();
    let shown: Vec<&(String, String)> = groups
        .iter()
        .filter(|(_, value)| only.is_none_or(|name| crate::masks::Masks::new(value).is_some_and(|m| m.check(name))))
        .collect();
    let entries = shown
        .iter()
        .map(|(name, value)| {
            let name: String = name.chars().take(10).collect();
            Entry::new(format!("{name:<10} \u{2502} {value}"))
        })
        .collect();
    let mut menu = VMenu::with("Groups of file masks", entries);
    menu.show_ampersand = true;
    menu.max_height_margin = Some(4);
    match only {
        Some(name) => {
            menu.title = name.to_string();
            menu.bottom = lang::trs(&format!("Total: {}", shown.len()));
        }
        None => menu.bottom = "Ins, Del, F4, F7, Ctrl+R".into(),
    }
    menu.select(select);
    (menu, shown.iter().map(|(n, _)| n.clone()).collect())
}

fn languages_menu(help: bool) -> VMenu {
    let key = if help { "Language.Help" } else { "Language.Main" };
    let current = settings::string(key);
    let entries = lang::LANGUAGES.iter().map(|(_, shown)| Entry::new(*shown)).collect();
    let mut menu = VMenu::with(if help { "Help language" } else { "Main language" }, entries);
    menu.auto_highlight = true;
    menu.select(lang::LANGUAGES.iter().position(|(name, _)| *name == current).unwrap_or(0));
    menu
}

/// The mask group dialog (a DialogBuilder: Name, the masks, 60 wide).
fn mask_group_dialog(name: &str, value: &str, old: Option<String>) -> Dialog {
    use crate::dialog::{at, span, Item};
    let items = vec![
        Item::Label(lang::tr("&Name:").into()),
        crate::dialog::line("name", name, false),
        Item::Label(lang::tr("A file &mask or several file masks:").into()),
        crate::dialog::line("masks", value, false),
        Item::Separator(None),
        Item::Buttons(vec![crate::dialog::button(lang::tr("OK"), "ok", true), crate::dialog::button(lang::tr("Cancel"), "cancel", false)]),
    ];
    let places = vec![at(5, 2), span(5, 3, 65), at(5, 4), span(5, 5, 65), at(-1, 6), at(-1, 7)];
    Dialog::placed(lang::tr("Groups of file masks").into(), (71, 10), items, places, 1, None, Action::MaskGroup(old))
}

impl App {
    pub(crate) fn run_option(&mut self, opt: Opt) {
        let dialog = match opt {
            Opt::System => options::system(),
            Opt::Panel => options::panel(),
            Opt::Interface => options::interface(),
            Opt::Dialog => options::dialogs(),
            Opt::Menu => options::menus(),
            Opt::Cmdline => options::cmdline(),
            Opt::AutoComplete => options::autocomplete(),
            Opt::InfoPanel => options::infopanel(),
            Opt::Confirmations => options::confirmations(),
            Opt::Descriptions => options::descriptions(),
            Opt::Viewer => options::viewer(),
            Opt::FolderInfo => options::folder_info(),
            Opt::SaveSetup => options::save_setup(),
            Opt::Languages => return self.overlay = Overlay::List(Box::new(List::Languages(languages_menu(false), false))),
            Opt::MaskGroups => {
                let (menu, names) = masks_menu(0, None);
                return self.overlay = Overlay::List(Box::new(List::MaskGroups(menu, names, false)));
            }
            Opt::CodePages => return self.overlay = Overlay::List(Box::new(List::CodePages(codepages::CodePagesMenu::new()))),
            Opt::Colors => return self.open_colors(),
            Opt::Highlighting => return self.open_highlighting(),
            Opt::PanelModes => return self.open_panel_modes(),
        };
        self.overlay = Overlay::Dialog(dialog);
    }

    pub(crate) fn languages_key(&mut self, mut menu: VMenu, help: bool, key: crossterm::event::KeyEvent) {
        match menu.key(key) {
            Outcome::Chosen(index) => {
                let name = lang::LANGUAGES[index].0;
                if help {
                    settings::set_string("Language.Help", name);
                } else {
                    settings::set_string("Language.Main", name);
                    lang::load();
                    // Far asks the help language next
                    self.overlay = Overlay::List(Box::new(List::Languages(languages_menu(true), true)));
                }
            }
            Outcome::Closed => {}
            _ => self.overlay = Overlay::List(Box::new(List::Languages(menu, help))),
        }
    }

    pub(crate) fn mask_groups_key(&mut self, mut menu: VMenu, names: Vec<String>, filtered: bool, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let index = menu.selected();
        let name = names.get(index).cloned();
        // a filtered list goes back to all of them on Esc, Enter, F10
        if filtered && matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::F(10)) {
            let (menu, names) = masks_menu(0, None);
            return self.overlay = Overlay::List(Box::new(List::MaskGroups(menu, names, false)));
        }
        let outcome = menu.key(key);
        let park = |app: &mut App, menu: VMenu, names: Vec<String>, dialog: Dialog| {
            app.under.push(Overlay::List(Box::new(List::MaskGroups(menu, names, filtered))));
            app.overlay = Overlay::Dialog(dialog);
        };
        match outcome {
            Outcome::Closed => {}
            Outcome::Chosen(_) => {
                if let Some(name) = name {
                    let value = mask_groups().into_iter().find(|(n, _)| *n == name).map(|(_, v)| v).unwrap_or_default();
                    park(self, menu, names, mask_group_dialog(&name, &value, Some(name.clone())));
                }
            }
            Outcome::Key(k) => match k.code {
                KeyCode::F(4) if name.is_some() => {
                    let name = name.unwrap_or_default();
                    let value = mask_groups().into_iter().find(|(n, _)| *n == name).map(|(_, v)| v).unwrap_or_default();
                    park(self, menu, names, mask_group_dialog(&name, &value, Some(name.clone())));
                }
                KeyCode::Insert => park(self, menu, names, mask_group_dialog("", "", None)),
                KeyCode::Delete if name.is_some() => {
                    let name = name.unwrap_or_default();
                    let lines = [lang::tr("Do you wish to delete").to_string(), name.clone()];
                    let dialog = Dialog::message("Groups of file masks", &lines, &[("Delete", "ok"), ("Cancel", "cancel")], false, Action::DeleteMaskGroup(name));
                    park(self, menu, names, dialog);
                }
                KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    let lines = [lang::tr("Do you wish to restore default mask sets?").to_string()];
                    let dialog = Dialog::message("Groups of file masks", &lines, &[("Yes", "ok"), ("Cancel", "cancel")], true, Action::RestoreMaskGroups);
                    park(self, menu, names, dialog);
                }
                KeyCode::F(7) => {
                    let dialog = Dialog::get_string("Filter", "&Mask:", "", Action::FindMaskGroup);
                    park(self, menu, names, dialog);
                }
                _ => self.overlay = Overlay::List(Box::new(List::MaskGroups(menu, names, filtered))),
            },
            Outcome::Open => self.overlay = Overlay::List(Box::new(List::MaskGroups(menu, names, filtered))),
        }
    }

    pub(crate) fn code_pages_key(&mut self, mut menu: codepages::CodePagesMenu, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        if menu.key(key) {
            return self.overlay = Overlay::List(Box::new(List::CodePages(menu)));
        }
        if key.code == KeyCode::F(4)
            && let Some(cp) = menu.current().cloned()
        {
            use crate::dialog::{at, span, Item};
            let items = vec![
                crate::dialog::line("name", &cp.name, false),
                Item::Separator(None),
                Item::Buttons(vec![
                    crate::dialog::button(lang::tr("OK"), "ok", true),
                    crate::dialog::button(lang::tr("&Reset"), "reset", false),
                    crate::dialog::button(lang::tr("Cancel"), "cancel", false),
                ]),
            ];
            let places = vec![span(5, 2, 48), at(-1, 3), at(-1, 4)];
            let dialog = Dialog::placed(lang::tr("Rename the code page").into(), (54, 7), items, places, 0, None, Action::RenameCodePage(cp.cp));
            self.under.push(Overlay::List(Box::new(List::CodePages(menu))));
            return self.overlay = Overlay::Dialog(dialog);
        }
        match menu.menu.key(key) {
            Outcome::Closed | Outcome::Chosen(_) => {}
            _ => self.overlay = Overlay::List(Box::new(List::CodePages(menu))),
        }
    }

    /// OK in one of the Options dialogs.
    pub(crate) fn accept_option(&mut self, action: Action, id: &str, values: &Values) {
        match action {
            Action::Settings(kind, bindings) => {
                builder::save(&bindings, values);
                if kind == Settings::Panel && !values.checked("Panel.AutoUpdateEnabled") {
                    settings::set_int("Panel.AutoUpdateLimit", 0);
                }
                self.settings_changed();
            }
            Action::FolderInfo => {
                settings::set_string("System.FolderInfo", &values.text("text"));
                self.settings_changed();
            }
            Action::SaveSetup => self.save_setup(),
            Action::MaskGroup(old) => {
                let (name, value) = (values.text("name"), values.text("masks"));
                let mut groups = mask_groups();
                if let Some(old) = old {
                    groups.retain(|(n, _)| *n != old);
                }
                if !name.is_empty() {
                    groups.push((name, value));
                }
                save_mask_groups(&groups);
                self.refill_masks();
            }
            Action::DeleteMaskGroup(name) => {
                let mut groups = mask_groups();
                groups.retain(|(n, _)| *n != name);
                save_mask_groups(&groups);
                self.refill_masks();
            }
            Action::RestoreMaskGroups => {
                let mut groups = mask_groups();
                for (name, value) in DEFAULT_MASKS {
                    groups.retain(|(n, _)| n != name);
                    groups.push((name.to_string(), value.to_string()));
                }
                save_mask_groups(&groups);
                self.refill_masks();
            }
            Action::FindMaskGroup => {
                let wanted = values.text("text");
                if let Some(Overlay::List(list)) = self.under.last_mut() {
                    let (menu, names) = masks_menu(0, Some(&wanted));
                    **list = List::MaskGroups(menu, names, true);
                }
            }
            Action::Highlight { index, new, attributes, colors } => self.accept_highlight(index, new, attributes, colors, id, values),
            Action::DeleteHighlight(index) => {
                let mut groups = crate::highlight::load();
                if index < groups.len() {
                    groups.remove(index);
                    crate::highlight::save(&groups);
                }
                self.refill_highlight(index);
            }
            Action::RestoreHighlight => {
                crate::highlight::save(&crate::highlight::defaults());
                self.refill_highlight(0);
            }
            Action::PanelMode(n) => {
                let mode = crate::panels::Panels::view_of(n);
                if id == "reset" {
                    mode.set_text(None);
                } else {
                    let flags = MODE_FLAGS.iter().filter(|(id, _, _)| values.checked(id)).fold(0, |f, (_, _, flag)| f | flag);
                    let text = crate::panels::ModeText {
                        name: values.text("name"),
                        columns: values.text("types"),
                        widths: values.text("widths"),
                        status: values.text("stypes"),
                        status_widths: values.text("swidths"),
                        flags,
                    };
                    mode.set_text(Some(&text));
                }
                if let Some(Overlay::List(list)) = self.under.last_mut()
                    && let List::PanelModes(menu) = list.as_ref()
                {
                    let row = menu.selected();
                    **list = List::PanelModes(panel_modes_menu(row));
                }
                self.panels.refresh(None);
            }
            Action::RenameCodePage(cp) => {
                if let Some(Overlay::List(list)) = self.under.last_mut()
                    && let List::CodePages(menu) = list.as_mut()
                {
                    let name = if id == "reset" { String::new() } else { values.text("name") };
                    menu.rename(cp, &name);
                }
            }
            _ => {}
        }
    }

    fn refill_masks(&mut self) {
        if let Some(Overlay::List(list)) = self.under.last_mut()
            && let List::MaskGroups(menu, _, _) = list.as_ref()
        {
            let (menu, names) = masks_menu(menu.selected(), None);
            **list = List::MaskGroups(menu, names, false);
        }
    }

    /// What the settings change at once: the panels read again (hidden
    /// files, sorting, highlighting), the mouse.
    pub(crate) fn settings_changed(&mut self) {
        crate::sorting::load_options();
        self.panels.refresh(None);
        let mut out = std::io::stdout();
        let _ = if settings::bool("Interface.Mouse") {
            crossterm::execute!(out, crossterm::event::EnableMouseCapture)
        } else {
            crossterm::execute!(out, crossterm::event::DisableMouseCapture)
        };
    }

    /// Shift+F9 (and on exit with "Auto save setup"): the panels' state.
    pub(crate) fn save_setup(&mut self) {
        self.panels.save_state();
    }
}

/// What a row of the highlighting menu is: a group, or the empty row at a part's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HiRow {
    Group(usize),
    End(u8),
}

/// FillMenu: each part's groups and an empty row, then the next part's title.
fn highlight_menu(select: usize) -> (VMenu, Vec<HiRow>) {
    let groups = crate::highlight::load();
    let titles = ["Upper sort group", "Lower sort group", "Lowest priority highlighting group"];
    let mut entries = Vec::new();
    let mut rows = Vec::new();
    for section in 0..4u8 {
        for (i, group) in groups.iter().enumerate().filter(|(_, g)| g.section == section) {
            entries.push(Entry::new(crate::filter::highlight_string(group)));
            rows.push(HiRow::Group(i));
        }
        entries.push(Entry::new(""));
        rows.push(HiRow::End(section));
        if let Some(title) = titles.get(section as usize) {
            entries.push(Entry::separator(lang::tr(title)));
            rows.push(HiRow::End(section));
        }
    }
    let mut menu = VMenu::with("Files highlighting", entries);
    menu.max_height_margin = Some(4);
    menu.bottom = "Ins, Del, F4, F5, Ctrl+Up, Ctrl+Down, Ctrl+R".into();
    menu.select(select);
    (menu, rows)
}

/// The predefined modes' names in the menu's order (Ctrl+1 .. Ctrl+9, Ctrl+0).
const MODE_NAMES: [&str; 10] = [
    "&Brief",
    "&Medium",
    "&Full",
    "&Wide",
    "Detai&led",
    "&Descriptions",
    "Lon&g descriptions",
    "File own&ers",
    "File lin&ks",
    "&Alternative full",
];

/// The menu's row of Far's mode `n` and back (RealModeToDisplay).
fn mode_number(row: usize) -> usize {
    (row + 1) % 10
}

fn panel_modes_menu(select: usize) -> VMenu {
    let entries = (0..10)
        .map(|row| {
            let name = crate::panels::Panels::view_of(mode_number(row)).text().name;
            Entry::new(if name.is_empty() { lang::tr(MODE_NAMES[row]).to_string() } else { name })
        })
        .collect();
    let mut menu = VMenu::with("Edit panel modes", entries);
    menu.max_height_margin = Some(4);
    menu.bottom = "Ins, Del, F4, Ctrl+Enter".into();
    menu.select(select);
    menu
}

const MODE_FLAGS: [(&str, &str, u32); 6] = [
    ("fullscreen", "&Fullscreen view", crate::panels::PVS_FULLSCREEN),
    ("alignext", "Ali&gn file extensions", crate::panels::PVS_ALIGNEXTENSIONS),
    ("folderext", "Align folder e&xtensions", crate::panels::PVS_FOLDERALIGNEXTENSIONS),
    ("folderupper", "Show folders in &uppercase", crate::panels::PVS_FOLDERUPPERCASE),
    ("fileslower", "Show files in &lowercase", crate::panels::PVS_FILELOWERCASE),
    ("uppertolower", "Show uppercase file names in lower&case", crate::panels::PVS_FILEUPPERTOLOWERCASE),
];

/// The mode's dialog (config.cpp's ModeDlg: 76 by 19).
fn panel_mode_dialog(n: usize, row: usize) -> Dialog {
    use crate::dialog::{at, line, span, Item};
    let text = crate::panels::Panels::view_of(n).text();
    let title = if text.name.is_empty() { crate::vmenu::strip_hot(lang::tr(MODE_NAMES[row])) } else { text.name.clone() };
    let mut items = vec![
        (Item::Label(lang::tr("&Name").into()), at(5, 2)),
        (line("name", &text.name, false), span(5, 3, 70)),
        (Item::Label(lang::tr("Column &types").into()), at(5, 4)),
        (line("types", &text.columns, false), span(5, 5, 35)),
        (Item::Label(lang::tr("Column &widths").into()), at(5, 6)),
        (line("widths", &text.widths, false), span(5, 7, 35)),
        (Item::Label(lang::tr("St&atus line column types").into()), at(38, 4)),
        (line("stypes", &text.status, false), span(38, 5, 70)),
        (Item::Label(lang::tr("Status l&ine column widths").into()), at(38, 6)),
        (line("swidths", &text.status_widths, false), span(38, 7, 70)),
        (Item::Separator(None), at(-1, 8)),
    ];
    for (i, (id, label, flag)) in MODE_FLAGS.iter().enumerate() {
        items.push((Item::Check { id, label: lang::tr(label), checked: text.flags & flag != 0, enabled: true }, at(5, 9 + i as u16)));
    }
    items.push((Item::Separator(None), at(-1, 15)));
    let buttons = vec![
        crate::dialog::button(lang::tr("OK"), "ok", true),
        crate::dialog::button(lang::tr("&Reset"), "reset", false),
        crate::dialog::button(lang::tr("Cancel"), "cancel", false),
    ];
    items.push((Item::Buttons(buttons), at(-1, 16)));
    let (items, places): (Vec<Item>, Vec<_>) = items.into_iter().unzip();
    Dialog::placed(title, (76, 19), items, places, 1, None, Action::PanelMode(n))
}

impl App {
    pub(crate) fn open_colors(&mut self) {
        self.overlay = Overlay::List(Box::new(List::ColorGroups(crate::colorpick::groups_menu())));
    }

    pub(crate) fn color_groups_key(&mut self, mut menu: VMenu, key: crossterm::event::KeyEvent) {
        use crate::colorpick::{group_row, items_menu, themes_menu, GroupRow};
        match menu.key(key) {
            Outcome::Chosen(index) => {
                let next = match group_row(index) {
                    Some(GroupRow::Group(items)) => Overlay::List(Box::new(List::ColorItems(items_menu(items, (10, 5)), items, (10, 5)))),
                    Some(GroupRow::Themes) => Overlay::List(Box::new(List::Themes(themes_menu()))),
                    Some(GroupRow::Palette) => {
                        let mut b = crate::builder::Builder::new("Palette");
                        b.check("Interface.SetPalette", "Classic palette");
                        b.ok_cancel();
                        Overlay::Dialog(b.build(Settings::Interface))
                    }
                    None => return self.overlay = Overlay::List(Box::new(List::ColorGroups(menu))),
                };
                self.under.push(Overlay::List(Box::new(List::ColorGroups(menu))));
                self.overlay = next;
            }
            Outcome::Closed => {}
            _ => self.overlay = Overlay::List(Box::new(List::ColorGroups(menu))),
        }
    }

    pub(crate) fn color_items_key(&mut self, mut menu: VMenu, items: &'static [crate::colordata::Item], at: (u16, u16), key: crossterm::event::KeyEvent) {
        use crate::colordata::Item;
        match menu.key(key) {
            Outcome::Chosen(index) => {
                let next = match &items[index] {
                    Item::Sub(_, sub) => {
                        let deeper = (at.0 + 10, at.1 + 5);
                        Overlay::List(Box::new(List::ColorItems(crate::colorpick::items_menu(sub, deeper), sub, deeper)))
                    }
                    Item::Color(_, name) => Overlay::ColorPick(Box::new(crate::colorpick::Picker::new(name))),
                };
                self.under.push(Overlay::List(Box::new(List::ColorItems(menu, items, at))));
                self.overlay = next;
            }
            Outcome::Closed => {}
            _ => self.overlay = Overlay::List(Box::new(List::ColorItems(menu, items, at))),
        }
    }

    /// The color dialog's answer.
    pub(crate) fn color_pick_outcome(&mut self, picker: Box<crate::colorpick::Picker>, outcome: crate::colorpick::Outcome) {
        use crate::colorpick::Outcome as O;
        // a highlighting group's color: the group's dialog again
        if let Some(slot) = picker.slot {
            if matches!(outcome, O::Open) {
                return self.overlay = Overlay::ColorPick(picker);
            }
            if let Some((mut group, index, new)) = self.hl_pending.take() {
                match outcome {
                    O::Set(f, b) => group.colors[slot] = Some((f, b)),
                    O::Reset => group.colors[slot] = None,
                    _ => {}
                }
                self.overlay = Overlay::Dialog(crate::filter::highlight_dialog(&group, index, new));
            }
            return;
        }
        match outcome {
            O::Open => self.overlay = Overlay::ColorPick(picker),
            O::Closed => {}
            O::Set(f, b) => crate::colors::set(picker.name, Some((f, b))),
            O::Reset => crate::colors::set(picker.name, None),
        }
    }
    pub(crate) fn open_highlighting(&mut self) {
        let (menu, rows) = highlight_menu(0);
        self.overlay = Overlay::List(Box::new(List::Highlight(menu, rows)));
    }

    pub(crate) fn highlight_key(&mut self, mut menu: VMenu, rows: Vec<HiRow>, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let row = menu.selected();
        let at = rows.get(row).copied();
        let mut groups = crate::highlight::load();
        let park = |app: &mut App, menu: VMenu, rows: Vec<HiRow>, dialog: Dialog| {
            app.under.push(Overlay::List(Box::new(List::Highlight(menu, rows))));
            app.overlay = Overlay::Dialog(dialog);
        };
        // where a new group goes: before the group under the cursor, or at its part's end
        let insert_at = |groups: &[crate::highlight::Group]| -> (usize, u8) {
            match at {
                Some(HiRow::Group(i)) => (i, groups[i].section),
                Some(HiRow::End(section)) => (groups.iter().rposition(|g| g.section <= section).map_or(0, |i| i + 1), section),
                None => (groups.len(), 3),
            }
        };
        match menu.key(key) {
            Outcome::Closed => {}
            Outcome::Chosen(_) => {
                if let Some(HiRow::Group(i)) = at {
                    park(self, menu, rows, crate::filter::highlight_dialog(&groups[i], i, false));
                } else {
                    self.overlay = Overlay::List(Box::new(List::Highlight(menu, rows)));
                }
            }
            Outcome::Key(k) => match k.code {
                KeyCode::F(4) if matches!(at, Some(HiRow::Group(_))) => {
                    let Some(HiRow::Group(i)) = at else { return };
                    park(self, menu, rows, crate::filter::highlight_dialog(&groups[i], i, false));
                }
                KeyCode::Insert | KeyCode::F(5) => {
                    let (index, section) = insert_at(&groups);
                    let mut group = match (k.code, at) {
                        (KeyCode::F(5), Some(HiRow::Group(i))) => groups[i].clone(),
                        _ => crate::highlight::Group { filter: crate::filter::Filter::default(), mark: String::new(), inherit_mark: true, colors: [None; 8], continue_processing: false, section },
                    };
                    group.section = section;
                    self.hl_section = section;
                    park(self, menu, rows, crate::filter::highlight_dialog(&group, index, true));
                }
                KeyCode::Delete if matches!(at, Some(HiRow::Group(_))) => {
                    let Some(HiRow::Group(i)) = at else { return };
                    let lines = [lang::tr("Do you wish to delete highlighting for").to_string(), format!("\"{}\"", groups[i].filter.title)];
                    park(self, menu, rows, Dialog::message("Files highlighting", &lines, &[("Delete", "ok"), ("Cancel", "cancel")], true, Action::DeleteHighlight(i)));
                }
                KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    let lines = [lang::tr("You will lose all changes").to_string(), lang::tr("Do you wish to restore default highlighting?").to_string()];
                    park(self, menu, rows, Dialog::message("Files highlighting", &lines, &[("Yes", "ok"), ("Cancel", "cancel")], true, Action::RestoreHighlight));
                }
                // Ctrl+Up/Down: a group moves, over the part's edge into the next part
                KeyCode::Up | KeyCode::Down if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(HiRow::Group(i)) = at {
                        let up = k.code == KeyCode::Up;
                        let section = groups[i].section;
                        let neighbour = if up { i.checked_sub(1) } else { Some(i + 1).filter(|&j| j < groups.len()) };
                        match neighbour {
                            Some(j) if groups[j].section == section => groups.swap(i, j),
                            _ if up && section > 0 => groups[i].section -= 1,
                            _ if !up && section < 3 => groups[i].section += 1,
                            _ => {}
                        }
                        let moved = groups[i].clone();
                        crate::highlight::save(&groups);
                        let (menu, rows) = highlight_menu(0);
                        let mut menu = menu;
                        if let Some(r) = rows.iter().position(|r| matches!(r, HiRow::Group(g) if crate::highlight::load()[*g] == moved)) {
                            menu.select(r);
                        }
                        self.panels.refresh(None);
                        return self.overlay = Overlay::List(Box::new(List::Highlight(menu, rows)));
                    }
                    self.overlay = Overlay::List(Box::new(List::Highlight(menu, rows)));
                }
                _ => self.overlay = Overlay::List(Box::new(List::Highlight(menu, rows))),
            },
            Outcome::Open => self.overlay = Overlay::List(Box::new(List::Highlight(menu, rows))),
        }
    }

    /// The highlighting dialog's buttons: OK keeps the group, a color button
    /// opens the color dialog for that color (the dialog comes back after it).
    fn accept_highlight(&mut self, index: usize, new: bool, attributes: Vec<u8>, colors: [Option<(u8, u8)>; 8], id: &str, values: &Values) {
        let mut groups = crate::highlight::load();
        let section = if new { self.hl_section } else { groups.get(index).map_or(0, |g| g.section) };
        let flags = [None; crate::filter::AREAS];
        let group = crate::highlight::Group {
            filter: crate::filter::from_values(values, attributes, flags),
            mark: values.text("mark"),
            inherit_mark: values.checked("inherit"),
            colors,
            continue_processing: values.checked("continue"),
            section,
        };
        if let Some(slot) = crate::filter::color_slot(id) {
            let (f, b) = colors[slot].unwrap_or((0, 0));
            let mut picker = crate::colorpick::Picker::with(f, b);
            picker.slot = Some(slot);
            self.hl_pending = Some((group, index, new));
            self.overlay = Overlay::ColorPick(Box::new(picker));
            return;
        }
        if new {
            groups.insert(index.min(groups.len()), group);
        } else if index < groups.len() {
            groups[index] = group;
        }
        crate::highlight::save(&groups);
        self.refill_highlight(index);
    }

    fn refill_highlight(&mut self, select_group: usize) {
        if let Some(Overlay::List(list)) = self.under.last_mut()
            && let List::Highlight(_, _) = list.as_ref()
        {
            let (mut menu, rows) = highlight_menu(0);
            if let Some(r) = rows.iter().position(|r| *r == HiRow::Group(select_group)) {
                menu.select(r);
            }
            **list = List::Highlight(menu, rows);
        }
        self.panels.refresh(None);
    }

    /// Options > File panel modes (SetFilePanelModes), on the active panel's mode.
    pub(crate) fn open_panel_modes(&mut self) {
        let row = (self.panels.active_mode() + 9) % 10;
        self.overlay = Overlay::List(Box::new(List::PanelModes(panel_modes_menu(row))));
    }

    pub(crate) fn panel_modes_key(&mut self, mut menu: VMenu, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let row = menu.selected();
        let edit = |app: &mut App, menu: VMenu| {
            app.under.push(Overlay::List(Box::new(List::PanelModes(menu))));
            app.overlay = Overlay::Dialog(panel_mode_dialog(mode_number(row), row));
        };
        match menu.key(key) {
            Outcome::Chosen(_) => edit(self, menu),
            Outcome::Closed => {}
            Outcome::Key(k) if k.code == KeyCode::F(4) => edit(self, menu),
            // Ctrl+Enter: the active panel takes the mode (no keys with two modifiers in farsh)
            Outcome::Key(k) if k.code == KeyCode::Enter && k.modifiers == KeyModifiers::CONTROL => {
                self.panels.set_view(self.panels.right_active(), mode_number(row) as u8);
                self.overlay = Overlay::List(Box::new(List::PanelModes(menu)));
            }
            _ => self.overlay = Overlay::List(Box::new(List::PanelModes(menu))),
        }
    }
}
