//! Far style dialogs: a grey (or red, for warnings) box with a shadow, a
//! double frame with the title in it, separators, input lines, radio
//! buttons, drop-downs, check boxes and buttons, with hot keys.
//!
//! The geometry is Far Manager 3's own: a box `W` columns wide with the
//! frame 3 columns and 1 row in (message.cpp, dialog.cpp), the controls
//! from column 5 of the box to `W - 6`, a history arrow at `W - 5`, button
//! rows centered as DIF_CENTERGROUP does. Messages (message.cpp) and
//! DialogBuilder dialogs are `MaxLength + 10` wide; the dialogs Far lays
//! out by hand (copy.cpp, mkdir.cpp) have their fixed width.

use std::path::PathBuf;
use std::time::SystemTime;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::Color;

use crate::attributes as attr;
use crate::fileops::{Conflict, Job, Kind};
use crate::grid::Grid;
use crate::palette;


/// What an accepted dialog asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Copy(Vec<PathBuf>),
    Move(Vec<PathBuf>),
    MkDir,
    Delete(Vec<PathBuf>),
    /// Alt+Del.
    Wipe(Vec<PathBuf>),
    /// Ctrl+A: what it opened with and what the user did since.
    Attributes(Box<crate::attributes::Setup>),
    /// Shift+F1: Create archive.
    CreateArchive(Box<crate::arcdialog::CreateSetup>),
    /// F5/F6 in an archive: Extract files.
    Extract(Box<crate::arcdialog::ExtractSetup>),
    /// F8 in a zip: arclite's "Do you want to delete selected files?".
    DeleteInArchive(Box<crate::panels::ArchivePick>),
    /// F7 in a zip: the folder's name.
    CreateDirInArchive(Box<crate::panels::ArchiveView>),
    /// Alt++ (true) / Alt+-: the mask.
    SelectGroup(bool),
    /// Alt+F6: links to these.
    Link(Vec<PathBuf>),
    /// Ctrl+G: the command for each of these names (in the active folder).
    ApplyCommand(Vec<String>),
    /// Ctrl+Z: the description of `names[0]`, then the rest in turn.
    Describe(PathBuf, Vec<String>),
    /// Folder shortcuts, F4: the title and path of shortcut N.
    Shortcut(usize),
    /// File associations: F4 on N, or (`new`) a new one before N.
    Assoc { index: usize, new: bool },
    DeleteAssoc(usize),
    /// A history menu's Del: "Clear history".
    ClearHistory,
    /// Ctrl+W's Del: kill this process.
    KillTask(u32),
    /// Hotplug's Del: remove device N.
    Eject(usize),
    /// User menu: Ins's "command or menu", F4's edit, Del's question; Edit
    /// user menu's "Main or Local".
    UserMenuInsert,
    UserMenuEdit { new: bool, submenu: bool },
    UserMenuDelete,
    UserMenuChoose,
    /// Ctrl+I: F4 on filter N, or a new one at N (Ins, F5); the attribute
    /// states the "..." dialog set (0 off, 1 on, 2 any).
    Filter { index: usize, new: bool, attributes: Vec<u8> },
    DeleteFilter(usize),
    /// Alt+F7.
    FindFile,
    /// An Options dialog: which, and the settings its items are bound to.
    Settings(crate::builder::Settings, Vec<crate::builder::Binding>),
    /// Options > Folder description files.
    FolderInfo,
    /// Shift+F9.
    SaveSetup,
    /// Groups of file masks: a group edited (its old name; none: a new one), deleted, defaults back.
    MaskGroup(Option<String>),
    DeleteMaskGroup(String),
    RestoreMaskGroups,
    /// Groups of file masks, F7: the name to look for.
    FindMaskGroup,
    /// Code pages, F4.
    RenameCodePage(u32),
    /// File panel modes: the mode (Far's number, Ctrl+N).
    PanelMode(usize),
    /// Files highlighting: group N edited (or a new one at N), with the
    /// attributes and the eight colors the dialog has so far.
    Highlight { index: usize, new: bool, attributes: Vec<u8>, colors: [Option<(u8, u8)>; 8] },
    /// Files highlighting: delete group N; Ctrl+R's defaults.
    DeleteHighlight(usize),
    RestoreHighlight,
    /// F7 in the help: what to search for.
    HelpSearch,
    /// far:config: a number's or a text's value.
    FarConfig { key: &'static str, mode: crate::farconfig::Mode },
    Quit,
    /// The answer to a file that already exists.
    Conflict,
    /// Just something to read.
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Normal,
    /// Red: deleting, errors, a file in the way.
    Warning,
}

struct Palette {
    bg: Color,
    fg: Color,
    hot: Color,
    focus_bg: Color,
    focus_fg: Color,
    edit_bg: Color,
    edit_fg: Color,
    /// An input line's text before the first key (Far's "unchanged" color).
    unchanged_fg: Color,
    /// A focused drop-down.
    selected_bg: Color,
    selected_fg: Color,
    dim: Color,
}

impl Style {
    /// Far's Dialog.* or WarnDialog.* colors (Options > Colors).
    fn palette(self) -> Palette {
        use crate::colors::{bg, fg};
        let k = |name: &str| -> String { format!("{}{name}", if self == Style::Warning { "WarnDialog." } else { "Dialog." }) };
        Palette {
            bg: bg(&k("Text")),
            fg: fg(&k("Text")),
            hot: fg(&k("Text.Highlight")),
            focus_bg: bg(&k("Button.Selected")),
            focus_fg: fg(&k("Button.Selected")),
            edit_bg: bg(&k("Edit")),
            edit_fg: fg(&k("Edit")),
            unchanged_fg: fg(&k("Edit.Unchanged")),
            selected_bg: bg(&k("List.Text.Selected")),
            selected_fg: fg(&k("List.Text.Selected")),
            dim: fg(&k("Disabled")),
        }
    }
}

pub struct Button {
    /// With `&` before the hot key, if it has one.
    pub label: &'static str,
    pub id: &'static str,
    pub default: bool,
    pub enabled: bool,
    /// DIF_BTNNOCLOSE: the dialog's hook handles it, the dialog stays.
    pub stay: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
}

pub enum Item {
    /// Plain text: `&` is just a character (DIF_SHOWAMPERSAND). A
    /// highlighted line is in the hot color (the names Delete lists).
    Text { text: String, align: Align, highlight: bool },
    /// A text with a hot key (`&`, `&&` for a plain one) that focuses the next control.
    Label(String),
    /// An input line from column `x` of the box to `W - 6`, its label (if
    /// any) at column 5; `history` puts Far's `↓` after it. Until the first
    /// key the text is "unchanged": a typed character replaces all of it.
    Edit { id: &'static str, label: Option<&'static str>, x: u16, text: Vec<char>, cursor: usize, unchanged: bool, enabled: bool, history: bool, secret: bool },
    Radio { id: &'static str, label: &'static str, options: Vec<&'static str>, selected: usize },
    Check { id: &'static str, label: &'static str, checked: bool, enabled: bool },
    /// A drop-down list from column `x` of the box, its label at column 5.
    Combo { id: &'static str, label: &'static str, x: u16, options: Vec<&'static str>, selected: usize, unchanged: bool },
    Separator(Option<&'static str>),
    Buttons(Vec<Button>),
    /// A check box with a third state (DIF_3STATE when `three`): 0 off,
    /// 1 on, 2 "leave as it is", shown `[?]`.
    Check3 { id: &'static str, label: &'static str, state: u8, three: bool, enabled: bool },
    /// DI_FIXEDIT with a mask (DIF_MASKEDIT): `9` takes a digit, `N` a digit
    /// or a space, anything else is fixed; the text is always as long as the mask.
    Masked { id: &'static str, mask: Vec<char>, text: Vec<char>, cursor: usize, unchanged: bool },
    /// A user separator (DIF_SEPARATORUSER): first, middle and last
    /// character, vertical when its place spans rows.
    Line([char; 3]),
    /// One radio button of the group `id` (Far's DI_RADIOBUTTON): the
    /// group's value is the `index` of the one that is on.
    Option { id: &'static str, index: usize, label: &'static str, on: bool },
}

/// Where an item of a dialog laid out by hand sits, in Far's dialog
/// coordinates (from the box's top left): `x` -1 centers it (a separator:
/// from frame to frame); `x2`/`y2` are inclusive ends, 0 when it has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub x: i16,
    pub y: u16,
    pub x2: u16,
    pub y2: u16,
}

pub(crate) const fn at(x: i16, y: u16) -> Place {
    Place { x, y, x2: 0, y2: 0 }
}

pub(crate) const fn span(x: i16, y: u16, x2: u16) -> Place {
    Place { x, y, x2, y2: 0 }
}

/// What the dialog's hook hears about (a check box toggled, a button
/// that does not close the dialog pressed).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Toggled(&'static str),
    Button(&'static str),
    /// The dialog opened over this one closed: with the button pressed and
    /// its values, or not (Esc, Cancel).
    SubClosed(Option<(&'static str, Values)>),
    /// A button that closes the dialog was pressed (Far's DN_CLOSE): the
    /// hook may keep it open by opening a message over it.
    Closing(&'static str),
    /// A key changed an edit field (Far's DN_EDITCHANGE).
    Edited(&'static str),
}

/// How the box is sized and placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// Far's Message(): as wide as the content, one row higher than centered.
    Message,
    /// A DialogBuilder dialog: as wide as the content, centered.
    Builder,
    /// A dialog with its own width, centered.
    Fixed(u16),
    /// A dialog laid out by hand (DlgX by DlgY), every item at its [`Place`].
    Placed(u16, u16),
}

/// What the dialog's controls say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values {
    texts: Vec<(&'static str, String)>,
    checks: Vec<(&'static str, bool)>,
    choices: Vec<(&'static str, usize)>,
}

impl Values {
    pub fn text(&self, id: &str) -> String {
        self.texts.iter().find(|(k, _)| *k == id).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    pub fn checked(&self, id: &str) -> bool {
        self.checks.iter().find(|(k, _)| *k == id).is_some_and(|(_, v)| *v)
    }

    pub fn choice(&self, id: &str) -> usize {
        self.choices.iter().find(|(k, _)| *k == id).map_or(0, |(_, v)| *v)
    }

    /// Values as if a dialog had this text (Copy with no question).
    pub fn with_text(id: &'static str, text: &str) -> Self {
        Self { texts: vec![(id, text.to_string())], ..Self::default() }
    }

    /// Whether the dialog had a choice control `id`.
    pub fn has_choice(&self, id: &str) -> bool {
        self.choices.iter().any(|(k, _)| *k == id)
    }
}

pub struct Dialog {
    title: String,
    style: Style,
    layout: Layout,
    items: Vec<Item>,
    focus: usize,
    /// The focused button, when the focus is on a row of buttons.
    button: usize,
    /// For [`Layout::Placed`]: where each item is.
    places: Vec<Place>,
    /// DIF_DISABLE and DIF_HIDDEN of each item (indices past the end are neither).
    disabled: Vec<bool>,
    hidden: Vec<bool>,
    /// What the dialog does itself on an [`Event`] (Far's dialog procedure).
    hook: Option<fn(&mut Dialog, Event)>,
    /// A dialog opened over this one (Attributes' "More"): it has the keys
    /// until it closes, and the hook hears how it closed.
    sub: Option<Box<Dialog>>,
    /// The hook decided to close the dialog with this button (after a
    /// question over it was answered).
    closing: Option<&'static str>,
    /// DialogBuilder's LinkFlags (DIF_AUTOMATION): while the check box is
    /// off, the other item is disabled.
    links: Vec<(usize, usize)>,
    /// The texts are in the interface language.
    localized: bool,
    action: Action,
}

pub enum Outcome {
    Open,
    Closed,
    /// A button was pressed (its id), with what the controls said.
    Pressed(Action, &'static str, Values),
}

// ---------------------------------------------------------------- building

pub(crate) fn text(text: impl Into<String>) -> Item {
    Item::Text { text: text.into(), align: Align::Left, highlight: false }
}

pub(crate) fn centered(text: impl Into<String>) -> Item {
    Item::Text { text: text.into(), align: Align::Center, highlight: false }
}

pub(crate) fn edit(id: &'static str, value: &str) -> Item {
    let text: Vec<char> = value.chars().collect();
    let cursor = text.len();
    let unchanged = !text.is_empty();
    Item::Edit { id, label: None, x: 5, text, cursor, unchanged, enabled: true, history: true, secret: false }
}

/// An input line for a dialog laid out by hand (its place gives the columns).
pub(crate) fn line(id: &'static str, value: &str, history: bool) -> Item {
    let text: Vec<char> = value.chars().collect();
    let cursor = text.len();
    Item::Edit { id, label: None, x: 0, text, cursor, unchanged: !value.is_empty(), enabled: true, history, secret: false }
}

pub(crate) fn button(label: &'static str, id: &'static str, default: bool) -> Button {
    Button { label, id, default, enabled: true, stay: false }
}

/// `"name"` only when it starts or ends with a space (Far's QuoteOuterSpace).
pub fn quote_outer_space(name: &str) -> String {
    if name.starts_with(' ') || name.ends_with(' ') {
        format!("\"{name}\"")
    } else {
        name.to_string()
    }
}

/// `&` doubled, to show it in a label with a hot key.
pub fn escape_ampersands(text: &str) -> String {
    text.replace('&', "&&")
}

impl Dialog {
    /// F5 and F6: where to, and how (copy.cpp: 76 wide).
    pub fn copy_or_move(kind: Kind, label: String, dest: &str, action: Action) -> Self {
        let moving = kind == Kind::Move;
        let items = vec![
            Item::Label(label),
            edit("dest", dest),
            Item::Separator(None),
            Item::Radio { id: "access", label: "&Access rights:", options: vec!["Defau&lt", "Co&py", "&Inherit"], selected: 0 },
            Item::Separator(None),
            Item::Combo { id: "existing", label: "Already e&xisting files:", x: 29, options: vec!["Ask", "Overwrite", "Skip"], selected: 0, unchanged: true },
            Item::Check { id: "timestamps", label: "Preserve all timestamps", checked: false, enabled: true },
            Item::Check { id: "links", label: "Cop&y contents of symbolic links", checked: false, enabled: true },
            // a move goes to one place only
            Item::Check { id: "multiple", label: "Process &multiple destinations", checked: false, enabled: !moving },
            Item::Separator(None),
            Item::Check { id: "filter", label: "&Use filter", checked: false, enabled: false },
            Item::Separator(None),
            Item::Buttons(vec![
                button(if moving { "&Rename" } else { "&Copy" }, "ok", true),
                Button { label: "&Filter", id: "filter", default: false, enabled: false, stay: false },
                button("Ca&ncel", "cancel", false),
            ]),
        ];
        Self { title: if moving { "Rename/Move" } else { "Copy" }.into(), style: Style::Normal, layout: Layout::Fixed(76), items, focus: 1, button: 0, places: vec![], disabled: vec![], hidden: vec![], hook: None, sub: None, closing: None, links: vec![], localized: false, action }
    }

    /// F7 (mkdir.cpp: 76 wide). Links are not made yet: the link type is
    /// only "none" and the target stays disabled, as Far shows it then.
    pub fn mkdir() -> Self {
        Self {
            title: "Make folder".into(),
            style: Style::Normal,
            layout: Layout::Fixed(76),
            items: vec![
                Item::Label("Create the &folder:".into()),
                edit("name", ""),
                Item::Separator(None),
                Item::Combo { id: "link", label: "&Link type:", x: 20, options: vec!["none"], selected: 0, unchanged: true },
                Item::Edit { id: "target", label: Some("&Target:"), x: 20, text: vec![], cursor: 0, unchanged: false, enabled: false, history: true, secret: false },
                Item::Check { id: "multiple", label: "Process &multiple names", checked: false, enabled: true },
                Item::Separator(None),
                Item::Buttons(vec![button("OK", "ok", true), button("Cancel", "cancel", false)]),
            ],
            focus: 1,
            button: 0,
            places: vec![],
            disabled: vec![],
            hidden: vec![],
            hook: None,
            sub: None,
            closing: None, links: vec![], localized: false,
            action: Action::MkDir,
        }
    }

    /// F8 (delete.cpp): for one item the question and its name centered
    /// (and where a link points); for several, the question, then the names
    /// highlighted.
    pub fn delete(question: String, names: Vec<String>, single: bool, action: Action) -> Self {
        let mut items = vec![if single { centered(question) } else { text(question) }];
        if !single {
            items.push(Item::Separator(None));
        }
        for name in names {
            let align = if single { Align::Center } else { Align::Left };
            items.push(Item::Text { text: name, align, highlight: !single });
        }
        items.push(Item::Separator(None));
        // delete.cpp's show_confirmation: Wipe has its own title and button
        let verb = if matches!(action, Action::Wipe(_)) { "Wipe" } else { "Delete" };
        items.push(Item::Buttons(vec![button(verb, "ok", true), button("Cancel", "cancel", false)]));
        let focus = items.len() - 1;
        Self { title: verb.into(), style: Style::Warning, layout: Layout::Builder, items, focus, button: 0, places: vec![], disabled: vec![], hidden: vec![], hook: None, sub: None, closing: None, links: vec![], localized: false, action }
    }

    /// Delete/Wipe meets a folder with something in it (delete.cpp's
    /// Message: "Delete folder", the folder, Delete/All/Skip/Cancel).
    pub fn folder_not_empty(path: &std::path::Path, wipe: bool) -> Self {
        let (title, line, first) = if wipe {
            ("Wipe folder", "The following folder will be wiped:", "&Wipe")
        } else {
            ("Delete folder", "The following folder will be deleted:", "&Delete")
        };
        let buttons = vec![button(first, "proceed", true), button("&All", "all", false), button("&Skip", "skip", false), button("&Cancel", "cancel", false)];
        Self::warning(title, vec![line.into(), path.display().to_string()], buttons)
    }

    /// Delete/Wipe meets a read-only file (ConfirmDeleteReadOnlyFile).
    pub fn read_only(path: &std::path::Path, wipe: bool) -> Self {
        let (ask, first) = if wipe { ("Do you wish to wipe it?", "&Wipe") } else { ("Do you wish to delete it?", "&Delete") };
        let buttons = vec![
            button(first, "proceed", true),
            button("&All", "all", false),
            button("&Skip", "skip", false),
            button("S&kip all", "skipall", false),
            button("&Cancel", "cancel", false),
        ];
        Self::warning("Warning", vec!["The file is read only".into(), path.display().to_string(), ask.into()], buttons)
    }

    /// Wipe meets a file with other names (confirm_erase_file_with_hardlinks).
    pub fn hard_links(path: &std::path::Path) -> Self {
        let lines = vec![
            path.display().to_string(),
            "Several hard links link to this file.".into(),
            "Wiping this file will void all files linking to it.".into(),
            "Do you wish to wipe this file?".into(),
        ];
        let buttons = vec![
            button("&Wipe", "proceed", true),
            button("&All", "all", false),
            button("&Skip", "skip", false),
            button("S&kip all", "skipall", false),
            button("&Cancel", "cancel", false),
        ];
        Self::warning("Error", lines, buttons)
    }

    /// Far's Message(MSG_WARNING): the lines centered, a separator, the buttons.
    fn warning(title: &str, lines: Vec<String>, buttons: Vec<Button>) -> Self {
        let mut items: Vec<Item> = lines.into_iter().map(centered).collect();
        items.push(Item::Separator(None));
        items.push(Item::Buttons(buttons));
        let focus = items.len() - 1;
        Self { title: title.into(), style: Style::Warning, layout: Layout::Message, items, focus, button: 0, places: vec![], disabled: vec![], hidden: vec![], hook: None, sub: None, closing: None, links: vec![], localized: false, action: Action::Conflict }
    }

    /// Far's Message with these buttons (`warning`: MSG_WARNING's red): the
    /// first button is the default, `cancel` closes it.
    pub fn message(title: &str, lines: &[String], buttons: &[(&'static str, &'static str)], warning: bool, action: Action) -> Self {
        let buttons = buttons.iter().enumerate().map(|(i, (label, id))| button(label, id, i == 0)).collect();
        let mut dialog = Self::warning(title, lines.to_vec(), buttons);
        if !warning {
            dialog.style = Style::Normal;
        }
        dialog.action = action;
        dialog
    }

    /// Far::message(... FMSG_MB_OKCANCEL) as arclite asks it: the first line is the title.
    pub fn ok_cancel(title: &str, line: &str, action: Action) -> Self {
        let items = vec![centered(line), Item::Separator(None), Item::Buttons(vec![button("OK", "ok", true), button("Cancel", "cancel", false)])];
        Self { title: title.into(), style: Style::Normal, layout: Layout::Message, items, focus: 2, button: 0, places: vec![], disabled: vec![], hidden: vec![], hook: None, sub: None, closing: None, links: vec![], localized: false, action }
    }

    /// A question with Yes and No (Far's Message with no hot keys).
    pub fn confirm(title: &str, line: &str, action: Action) -> Self {
        let items = vec![centered(line), Item::Separator(None), Item::Buttons(vec![button("Yes", "ok", true), button("No", "cancel", false)])];
        Self { title: title.into(), style: Style::Normal, layout: Layout::Message, items, focus: 2, button: 0, places: vec![], disabled: vec![], hidden: vec![], hook: None, sub: None, closing: None, links: vec![], localized: false, action }
    }

    /// Far::info_dlg / Message with OK: something to read, not red.
    pub fn info(title: &str, text: &str) -> Self {
        let mut items: Vec<Item> = text.lines().map(centered).collect();
        items.push(Item::Separator(None));
        items.push(Item::Buttons(vec![button("OK", "ok", true)]));
        let focus = items.len() - 1;
        Self { title: title.into(), style: Style::Normal, layout: Layout::Message, items, focus, button: 0, places: vec![], disabled: vec![], hidden: vec![], hook: None, sub: None, closing: None, links: vec![], localized: false, action: Action::Info }
    }

    /// Message(MSG_WARNING) with OK.
    pub fn warning_ok(title: &str, text: &str) -> Self {
        let mut dialog = Self::error(text);
        dialog.title = title.into();
        dialog
    }

    /// An error to read.
    pub fn error(text: &str) -> Self {
        let mut items: Vec<Item> = text.lines().map(centered).collect();
        items.push(Item::Separator(None));
        items.push(Item::Buttons(vec![button("OK", "ok", true)]));
        let focus = items.len() - 1;
        Self { title: "Error".into(), style: Style::Warning, layout: Layout::Message, items, focus, button: 0, places: vec![], disabled: vec![], hidden: vec![], hook: None, sub: None, closing: None, links: vec![], localized: false, action: Action::Info }
    }

    /// A file is in the way of a copy.
    pub fn conflict(conflict: &Conflict) -> Self {
        let info = |label: &str, size: u64, modified: Option<SystemTime>| format!("{label:<9}{:>16} bytes  {}", group(size), date(modified));
        let items = vec![
            centered("File already exists"),
            centered(conflict.dst.display().to_string()),
            Item::Separator(None),
            text(info("New", conflict.new_size, conflict.new_modified)),
            text(info("Existing", conflict.old_size, conflict.old_modified)),
            Item::Separator(None),
            Item::Check { id: "remember", label: "&Remember choice", checked: false, enabled: true },
            Item::Separator(None),
            Item::Buttons(vec![button("&Overwrite", "overwrite", true), button("&Skip", "skip", false), button("&Cancel", "cancel", false)]),
        ];
        Self { title: "Warning".into(), style: Style::Warning, layout: Layout::Fixed(76), items, focus: 8, button: 0, places: vec![], disabled: vec![], hidden: vec![], hook: None, sub: None, closing: None, links: vec![], localized: false, action: Action::Conflict }
    }

    /// Ctrl+A, laid out as setattr.cpp lays it out: 80 by 21 (DlgX, DlgY),
    /// the check boxes at C1 = 5, the times and the owner at C2 = 32 right of
    /// a vertical line, a row more for a link, two for "Process subfolders".
    pub fn attributes(setup: attr::Setup) -> Self {
        let link_row = u16::from(setup.link.is_some());
        let folder_rows = if setup.folders { 2 } else { 0 };
        let (dlg_x, dlg_y) = (80u16, 21 + link_row + folder_rows);
        let below = |y: u16| y + link_row;
        let mut items = Vec::new();
        let mut places = Vec::new();
        let mut add = |item: Item, place: Place| {
            items.push(item);
            places.push(place);
        };
        add(centered("Change file attributes for"), at(-1, 2));
        let name = match setup.mode {
            attr::Mode::Multiple => "selected objects".to_string(),
            _ => quote_outer_space(&setup.paths[0].file_name().map_or_else(|| setup.paths[0].display().to_string(), |n| n.to_string_lossy().into_owned())),
        };
        add(Item::Text { text: name, align: Align::Center, highlight: false }, span(5, 3, dlg_x - 6));
        if let Some((label, target)) = &setup.link {
            add(Item::Label(label.clone()), span(5, 4, 17));
            let text: Vec<char> = target.chars().collect();
            let cursor = text.len();
            add(Item::Edit { id: "link", label: None, x: 18, text, cursor, unchanged: true, enabled: false, history: false, secret: false }, span(18, 4, dlg_x - 6));
        }
        add(Item::Separator(None), at(-1, below(4)));
        for (i, (attribute, id, label)) in attr::MAIN.iter().enumerate() {
            let check = Item::Check3 { id, label, state: setup.current[i], three: setup.three[i], enabled: attr::settable(*attribute) };
            add(check, at(5, below(5 + i as u16)));
        }
        add(Item::Buttons(vec![stay_button("&More", "more", true)]), span(5, below(16), 12));
        let (date_mask, date_title) = setup.format.date_mask();
        let (time_mask, time_title) = setup.format.time_mask();
        add(text("Date/Time:"), at(32, below(6)));
        add(text(date_title), at(dlg_x as i16 - 33, below(6)));
        add(text(time_title), at(dlg_x as i16 - 21, below(6)));
        for (k, (label, (date_id, time_id))) in TIME_LABELS.iter().zip(TIME_IDS).enumerate() {
            let y = below(7 + k as u16);
            add(Item::Label((*label).into()), at(32, y));
            add(masked(date_id, &date_mask, &setup.dates[k].0), at(dlg_x as i16 - 33, y));
            add(masked(time_id, &time_mask, &setup.dates[k].1), at(dlg_x as i16 - 21, y));
        }
        let single = setup.mode != attr::Mode::Multiple;
        add(
            Item::Buttons(vec![stay_button("&Original", "original", single), stay_button("Curre&nt", "current", true), stay_button("&Blank", "blank", true)]),
            span(32, below(12), dlg_x - 6),
        );
        add(Item::Label("Owner&:".into()), at(32, below(15)));
        let owner: Vec<char> = setup.owner.chars().collect();
        let cursor = owner.len();
        add(Item::Edit { id: "owner", label: None, x: 32, text: owner, cursor, unchanged: true, enabled: true, history: false, secret: false }, span(32, below(16), dlg_x - 6));
        if setup.folders {
            add(Item::Separator(None), at(-1, below(17)));
            add(Item::Check3 { id: "subfolders", label: "Process sub&folders", state: 0, three: false, enabled: true }, at(5, below(18)));
        }
        add(Item::Separator(None), at(-1, dlg_y - 4));
        let system = Button { label: "System &properties", id: "system", default: false, enabled: single && attr::system_properties_available(), stay: false };
        add(Item::Buttons(vec![button("Set", "ok", true), system, button("Cancel", "cancel", false)]), at(-1, dlg_y - 3));
        add(Item::Line(['\u{252C}', '\u{2502}', '\u{2534}']), Place { x: 30, y: below(4), x2: 0, y2: below(17) });
        add(Item::Line(['\u{251C}', '\u{2500}', '\u{2562}']), span(30, below(14), dlg_x - 4));
        let focus = items.iter().position(|item| matches!(item, Item::Check3 { enabled: true, .. })).unwrap_or(0);
        Self {
            title: "Attributes".into(),
            style: Style::Normal,
            layout: Layout::Placed(dlg_x, dlg_y),
            items,
            focus,
            button: 0,
            places,
            disabled: vec![],
            hidden: vec![],
            hook: Some(attributes_hook),
            sub: None,
            closing: None, links: vec![], localized: false,
            action: Action::Attributes(Box::new(setup)),
        }
    }

    /// Attributes' "More" (a DialogBuilder dialog): the rest of the attributes.
    fn more_attributes(setup: &attr::Setup) -> Self {
        let mut items: Vec<Item> = attr::ADVANCED
            .iter()
            .enumerate()
            .map(|(i, (attribute, id, label))| Item::Check3 { id, label, state: setup.current[10 + i], three: setup.three[10 + i], enabled: attr::settable(*attribute) })
            .collect();
        items.push(Item::Separator(None));
        items.push(Item::Buttons(vec![button("OK", "ok", true), button("Cancel", "cancel", false)]));
        let focus = items.iter().position(|item| matches!(item, Item::Check3 { enabled: true, .. })).unwrap_or(items.len() - 1);
        Self {
            title: "Attributes".into(),
            style: Style::Normal,
            layout: Layout::Builder,
            items,
            focus,
            button: 0,
            places: vec![],
            disabled: vec![],
            hidden: vec![],
            hook: Some(more_attributes_hook),
            sub: None,
            closing: None, links: vec![], localized: false,
            action: Action::Info,
        }
    }

    pub fn values(&self) -> Values {
        let mut values = Values::default();
        for item in &self.items {
            match item {
                Item::Edit { id, text, .. } => values.texts.push((id, text.iter().collect())),
                Item::Check { id, checked, .. } => values.checks.push((id, *checked)),
                Item::Radio { id, selected, .. } | Item::Combo { id, selected, .. } => values.choices.push((id, *selected)),
                Item::Check3 { id, state, .. } => values.choices.push((id, *state as usize)),
                Item::Masked { id, text, .. } => values.texts.push((id, text.iter().collect())),
                Item::Option { id, index, on: true, .. } => values.choices.push((id, *index)),
                _ => {}
            }
        }
        values
    }

    /// The index of the item with `id`.
    pub(crate) fn index_of(&self, id: &str) -> Option<usize> {
        self.items.iter().position(|item| match item {
            Item::Edit { id: i, .. }
            | Item::Radio { id: i, .. }
            | Item::Check { id: i, .. }
            | Item::Combo { id: i, .. }
            | Item::Check3 { id: i, .. }
            | Item::Masked { id: i, .. } => *i == id,
            Item::Option { id: i, on: true, .. } => *i == id,
            Item::Buttons(buttons) => buttons.iter().any(|b| b.id == id),
            _ => false,
        })
    }

    /// A dialog laid out by hand (Far's own coordinates, or arclite's
    /// layout, see `crate::arcdialog`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn placed(title: String, size: (u16, u16), items: Vec<Item>, places: Vec<Place>, focus: usize, hook: Option<fn(&mut Dialog, Event)>, action: Action) -> Self {
        let count = items.len();
        Self {
            title,
            style: Style::Normal,
            layout: Layout::Placed(size.0, size.1),
            items,
            focus,
            button: 0,
            places,
            disabled: vec![false; count],
            hidden: vec![false; count],
            hook,
            sub: None,
            closing: None, links: vec![], localized: false,
            action,
        }
    }

    /// Far's FDLG_WARNING: the red colors.
    pub(crate) fn set_warning(&mut self) {
        self.style = Style::Warning;
    }

    /// From the hook: close the dialog as if `id` was pressed.
    pub(crate) fn close_with(&mut self, id: &'static str) {
        self.closing = Some(id);
    }

    /// From the hook: a dialog over this one; the hook hears how it closed.
    pub(crate) fn open_sub(&mut self, sub: Dialog) {
        self.sub = Some(Box::new(sub));
    }

    /// A DialogBuilder dialog (as wide as its content, centered).
    pub(crate) fn builder(title: &str, items: Vec<Item>, focus: usize, hook: Option<fn(&mut Dialog, Event)>, action: Action) -> Self {
        Self { title: title.into(), style: Style::Normal, layout: Layout::Builder, items, focus, button: 0, places: vec![], disabled: vec![], hidden: vec![], hook, sub: None, closing: None, links: vec![], localized: false, action }
    }

    /// DM_ENABLE of a button in a row of buttons.
    pub(crate) fn enable_button(&mut self, id: &str, on: bool) {
        for item in &mut self.items {
            if let Item::Buttons(buttons) = item {
                for button in buttons.iter_mut().filter(|b| b.id == id) {
                    button.enabled = on;
                }
            }
        }
    }

    pub(crate) fn set_check(&mut self, index: usize, on: bool) {
        match self.items.get_mut(index) {
            Some(Item::Check { checked, .. }) => *checked = on,
            Some(Item::Check3 { state, .. }) => *state = u8::from(on),
            _ => {}
        }
    }

    pub(crate) fn is_disabled(&self, index: usize) -> bool {
        self.disabled.get(index).copied().unwrap_or(false) || self.hidden.get(index).copied().unwrap_or(false)
    }

    /// Far's DM_ENABLE.
    pub(crate) fn enable(&mut self, index: usize, on: bool) {
        if let Some(flag) = self.disabled.get_mut(index) {
            *flag = !on;
        }
    }

    /// Far's DM_SHOWITEM.
    pub(crate) fn show(&mut self, index: usize, on: bool) {
        if let Some(flag) = self.hidden.get_mut(index) {
            *flag = !on;
        }
    }

    pub(crate) fn action_mut(&mut self) -> &mut Action {
        &mut self.action
    }

    /// The state of a check box: 0, 1 or 2.
    pub(crate) fn check_state(&self, index: usize) -> u8 {
        match self.items.get(index) {
            Some(Item::Check { checked, .. }) => u8::from(*checked),
            Some(Item::Check3 { state, .. }) => *state,
            Some(Item::Option { on, .. }) => u8::from(*on),
            _ => 0,
        }
    }

    pub(crate) fn text_of(&self, index: usize) -> String {
        match self.items.get(index) {
            Some(Item::Edit { text, .. } | Item::Masked { text, .. }) => text.iter().collect(),
            _ => String::new(),
        }
    }

    pub(crate) fn set_text_at(&mut self, index: usize, value: &str) {
        if let Some(Item::Edit { text, cursor, unchanged, .. }) = self.items.get_mut(index) {
            *text = value.chars().collect();
            *cursor = text.len();
            *unchanged = false;
        }
    }

    /// Turns on the radio button `index` and off the others of its group.
    pub(crate) fn select_option(&mut self, index: usize) {
        let Some(Item::Option { id: group, .. }) = self.items.get(index) else { return };
        let group = *group;
        for (i, item) in self.items.iter_mut().enumerate() {
            if let Item::Option { id, on, .. } = item
                && *id == group
            {
                *on = i == index;
            }
        }
    }

    /// Sets the text of the input line or masked field `id` (fitted to its mask).
    pub(crate) fn set_text(&mut self, id: &str, value: &str, make_unchanged: bool) {
        let Some(index) = self.index_of(id) else { return };
        match &mut self.items[index] {
            Item::Edit { text, cursor, unchanged, .. } => {
                *text = value.chars().collect();
                *cursor = text.len();
                *unchanged = make_unchanged;
            }
            Item::Masked { mask, text, cursor, unchanged, .. } => {
                *text = fit_mask(mask, value);
                *cursor = 0;
                *unchanged = make_unchanged;
            }
            _ => {}
        }
    }

    fn set_check3(&mut self, id: &str, new_state: u8, new_three: bool) {
        if let Some(index) = self.index_of(id)
            && let Item::Check3 { state, three, .. } = &mut self.items[index]
        {
            *three = new_three;
            *state = new_state;
        }
    }

    fn check3(&self, id: &str) -> u8 {
        match self.index_of(id).map(|index| &self.items[index]) {
            Some(Item::Check3 { state, .. }) => *state,
            _ => 0,
        }
    }

    pub(crate) fn focus_on(&mut self, id: &str) {
        if let Some(index) = self.index_of(id) {
            self.focus = index;
        }
    }

    /// Whether the input line or masked field `id` still has the text it was given.
    fn is_unchanged(&self, id: &str) -> bool {
        match self.index_of(id).map(|index| &self.items[index]) {
            Some(Item::Edit { unchanged, text, .. }) => *unchanged || text.is_empty(),
            Some(Item::Masked { unchanged, text, .. }) => *unchanged || !text.iter().any(char::is_ascii_digit),
            _ => false,
        }
    }
}

const TIME_LABELS: [&str; 4] = ["&write:", "crea&tion:", "&access:", "chan&ge:"];

/// The date and time fields of the four times, in Far's TimeMap order.
pub const TIME_IDS: [(&str, &str); 4] = [("wdate", "wtime"), ("cdate", "ctime"), ("adate", "atime"), ("xdate", "xtime")];

pub(crate) fn stay_button(label: &'static str, id: &'static str, enabled: bool) -> Button {
    Button { label, id, default: false, enabled, stay: true }
}

pub(crate) fn masked(id: &'static str, mask: &str, value: &str) -> Item {
    let mask: Vec<char> = mask.chars().collect();
    let text = fit_mask(&mask, value);
    let unchanged = text.iter().any(char::is_ascii_digit);
    Item::Masked { id, mask, text, cursor: 0, unchanged }
}

/// Far's SetAttrDlgProc: Compressed and Encrypted exclude each other,
/// "Process subfolders" turns the check boxes to "leave as they are" and
/// blanks a folder's times and owner, Original/Current/Blank fill the times,
/// More opens the rest of the attributes.
fn attributes_hook(dialog: &mut Dialog, event: Event) {
    let Action::Attributes(mut setup) = std::mem::replace(&mut dialog.action, Action::Info) else { return };
    match event {
        Event::Toggled("subfolders") => {
            let on = dialog.check3("subfolders") == 1;
            for i in 0..20 {
                if on {
                    setup.three[i] = true;
                    if !setup.manual[i] {
                        setup.current[i] = 2;
                    }
                } else {
                    if setup.mode == attr::Mode::Folder {
                        setup.three[i] = false;
                    }
                    if !setup.manual[i] {
                        setup.current[i] = setup.initial[i];
                    }
                }
            }
            for (i, (_, id, _)) in attr::MAIN.iter().enumerate() {
                dialog.set_check3(id, setup.current[i], setup.three[i]);
            }
            if setup.mode == attr::Mode::Folder {
                for (k, (date_id, time_id)) in TIME_IDS.iter().enumerate() {
                    for (id, value) in [(date_id, &setup.dates[k].0), (time_id, &setup.dates[k].1)] {
                        if dialog.is_unchanged(id) {
                            dialog.set_text(id, if on { "" } else { value }, true);
                        }
                    }
                }
            }
            if dialog.is_unchanged("owner") {
                dialog.set_text("owner", if on { "" } else { &setup.owner }, true);
            }
        }
        Event::Toggled(id) => {
            if let Some(i) = attr::MAIN.iter().position(|(_, main, _)| *main == id) {
                setup.current[i] = dialog.check3(id);
                setup.manual[i] = true;
                // Compressed / Encrypted are mutually exclusive
                let other = match id {
                    "compressed" => Some("encrypted"),
                    "encrypted" => Some("compressed"),
                    _ => None,
                };
                if let Some(other) = other
                    && setup.current[i] == 1
                    && dialog.check3(other) != 0
                    && let Some(j) = attr::MAIN.iter().position(|(_, main, _)| *main == other)
                {
                    dialog.set_check3(other, 0, setup.three[j]);
                    setup.current[j] = 0;
                }
            }
        }
        Event::Button("original") => {
            for (k, (date_id, time_id)) in TIME_IDS.iter().enumerate() {
                dialog.set_text(date_id, &setup.dates[k].0, true);
                dialog.set_text(time_id, &setup.dates[k].1, true);
            }
            dialog.focus_on("wdate");
        }
        Event::Button(which @ ("current" | "blank")) => {
            let now = (which == "current").then(|| setup.format.to_strings(SystemTime::now()));
            for (date_id, time_id) in TIME_IDS {
                let (date, time) = now.clone().unwrap_or_default();
                dialog.set_text(date_id, &date, false);
                dialog.set_text(time_id, &time, false);
            }
            dialog.focus_on("wdate");
        }
        Event::Button("more") => dialog.sub = Some(Box::new(Dialog::more_attributes(&setup))),
        Event::SubClosed(Some((_, values))) => {
            for (i, (_, id, _)) in attr::ADVANCED.iter().enumerate() {
                let state = values.choice(id) as u8;
                if state != setup.current[10 + i] {
                    setup.current[10 + i] = state;
                    setup.manual[10 + i] = true;
                }
            }
        }
        _ => {}
    }
    dialog.action = Action::Attributes(setup);
}

/// A reparse point can only be removed, never set (Far's AdvancedAttributesDialog).
fn more_attributes_hook(dialog: &mut Dialog, event: Event) {
    if event == Event::Toggled("reparse") && dialog.check3("reparse") == 1 {
        let Some(index) = dialog.index_of("reparse") else { return };
        if let Item::Check3 { state, three, .. } = &mut dialog.items[index] {
            *state = if *three { 2 } else { 0 };
        }
    }
}

fn option_id(item: &Item) -> &'static str {
    match item {
        Item::Option { id, .. } => id,
        _ => "",
    }
}

/// What a masked field with `mask` shows for `value`.
pub fn masked_text(mask: &str, value: &str) -> String {
    fit_mask(&mask.chars().collect::<Vec<_>>(), value).into_iter().collect()
}

/// `value` laid over `mask` as Far's masked edit takes it: the mask's fixed
/// characters stay, one that `value` has at its place is used up by it, and
/// the rest of `value` fills the places in turn (a blank field is all
/// spaces). So `0000000000010001` in `0xHHHHHHHHHHHHHHHH` is `0x` and the
/// last fifteen digits, as Far shows it.
fn fit_mask(mask: &[char], value: &str) -> Vec<char> {
    let mut value = value.chars().peekable();
    mask.iter()
        .map(|&m| {
            if is_mask_slot(m) {
                value.next().and_then(|c| mask_fits(m, c)).unwrap_or(' ')
            } else {
                if value.peek() == Some(&m) {
                    value.next();
                }
                m
            }
        })
        .collect()
}

/// Far's mask places: `9` a digit, `N` a digit or space, `H` a hex digit
/// (as a capital), `B` a binary one.
fn is_mask_slot(m: char) -> bool {
    matches!(m, '9' | 'N' | 'H' | 'B')
}

/// `c` as mask place `m` takes it, if it does.
fn mask_fits(m: char, c: char) -> Option<char> {
    match m {
        '9' => c.is_ascii_digit().then_some(c),
        'N' => (c.is_ascii_digit() || c == ' ').then_some(c),
        'H' => c.is_ascii_hexdigit().then(|| c.to_ascii_uppercase()),
        'B' => matches!(c, '0' | '1').then_some(c),
        _ => None,
    }
}

// ---------------------------------------------------------------- layout

/// Where a dialog sits and where its parts are.
struct Frame {
    /// The grey box, with the shadow beside it.
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    /// The double frame inside it.
    fx: u16,
    fy: u16,
    fw: u16,
    fh: u16,
}

impl Frame {
    /// A box `w` by `h` centered on a screen of `size`; a message sits one row higher.
    fn new(size: (u16, u16), w: u16, h: u16, message: bool) -> Self {
        let (cols, rows) = size;
        let (w, h) = (w.min(cols), h.min(rows));
        let x = (cols - w) / 2;
        let y = if message { rows.saturating_sub(1).saturating_sub(h) / 2 } else { (rows - h) / 2 };
        let (fx_off, fy_off) = (3.min(w / 2), 1.min(h / 2));
        Self { x, y, w, h, fx: x + fx_off, fy: y + fy_off, fw: w - 2 * fx_off, fh: h - 2 * fy_off }
    }

    /// The progress boxes' old sizing: a frame at least `want` wide for `rows` rows.
    fn sized(size: (u16, u16), title: &str, want: u16, rows: u16) -> Self {
        Self::new(size, want.max(title.chars().count() as u16 + 8) + 6, rows + 4, false)
    }

    fn text_x(&self) -> u16 {
        self.x + 5.min(self.w)
    }

    fn text_w(&self) -> u16 {
        self.w.saturating_sub(10)
    }

    fn row_y(&self, index: usize) -> u16 {
        self.fy + 1 + index as u16
    }

    /// The shadow, the box, the double frame and the title in it.
    fn draw(&self, grid: &mut Grid, title: &str, p: &Palette) {
        grid.fill_with(self.x, self.y, self.w, self.h, p.fg, p.bg);
        palette::drop_shadow(grid, self.x, self.y, self.w, self.h);
        let line = "\u{2550}".repeat(self.fw.saturating_sub(2) as usize);
        grid.put(self.fx, self.fy, &format!("\u{2554}{line}\u{2557}"), p.fg, p.bg);
        grid.put(self.fx, self.fy + self.fh.saturating_sub(1), &format!("\u{255A}{line}\u{255D}"), p.fg, p.bg);
        for row in self.fy + 1..(self.fy + self.fh).saturating_sub(1) {
            grid.put(self.fx, row, "\u{2551}", p.fg, p.bg);
            grid.put(self.fx + self.fw.saturating_sub(1), row, "\u{2551}", p.fg, p.bg);
        }
        // a dialog without a title (DialogBuilder's default) has a plain frame
        if title.is_empty() {
            return;
        }
        let title = format!(" {title} ");
        grid.put(self.fx + self.fw.saturating_sub(title.chars().count() as u16) / 2, self.fy, &title, p.fg, p.bg);
    }

    /// A separator from edge to edge of the frame, with a title in it if given.
    fn separator(&self, grid: &mut Grid, row: u16, title: Option<&str>, p: &Palette) {
        let line = "\u{2500}".repeat(self.fw.saturating_sub(2) as usize);
        grid.put(self.fx, row, &format!("\u{255F}{line}\u{2562}"), p.fg, p.bg);
        if let Some(title) = title {
            let title = format!(" {title} ");
            grid.put(self.fx + self.fw.saturating_sub(title.chars().count() as u16) / 2, row, &title, p.fg, p.bg);
        }
    }
}

/// What a click or the focus can land on: (sub item, x, width).
type Span = (usize, u16, u16);

/// The length of a label as shown: `&` marks the hot key, `&&` is one `&`.
fn label_len(label: &str) -> u16 {
    shown(label).chars().count() as u16
}

/// A label without its hot key mark.
fn shown(label: &str) -> String {
    let mut out = String::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                out.push('&');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Buttons in a row, each `[ label ]`, one space apart (DIF_CENTERGROUP).
/// A button's width: `[ label ]`, or the label alone (DIF_NOBRACKETS: an id starting with `~`).
fn button_width(b: &Button) -> u16 {
    label_len(b.label) + if b.id.starts_with('~') { 0 } else { 4 }
}

fn buttons_len(buttons: &[Button]) -> u16 {
    let total: u16 = buttons.iter().map(|b| button_width(b) + 1).sum();
    total.saturating_sub(1)
}

/// The parts of an item along its row.
#[cfg(test)]
fn spans(item: &Item, frame: &Frame) -> Vec<Span> {
    spans_at(item, frame, None)
}

/// The parts of an item along its row, at its `place` in a dialog laid out by hand.
fn spans_at(item: &Item, frame: &Frame, place: Option<&Place>) -> Vec<Span> {
    let x = place.map_or(frame.text_x(), |p| frame.x + p.x.max(0) as u16);
    // input lines and drop-downs end at column W - 6, or where they are placed to end
    let field = |at: u16| {
        let start = place.map_or(frame.x + at, |p| frame.x + p.x.max(0) as u16);
        let end = match place {
            Some(p) if p.x2 > 0 => frame.x + p.x2 + 1,
            _ => (frame.x + frame.w).saturating_sub(5),
        };
        (start, end.saturating_sub(start))
    };
    match item {
        Item::Text { .. } | Item::Label(_) | Item::Separator(_) | Item::Line(_) => vec![],
        Item::Edit { x: at, .. } | Item::Combo { x: at, .. } => {
            let (start, width) = field(*at);
            vec![(0, start, width)]
        }
        Item::Masked { mask, .. } => vec![(0, x, mask.len() as u16)],
        Item::Check { label, .. } | Item::Check3 { label, .. } | Item::Option { label, .. } => vec![(0, x, 4 + label_len(label))],
        Item::Radio { label, options, .. } => {
            let mut at = x + label_len(label) + 1;
            options
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let width = 4 + label_len(name);
                    let span = (i, at, width);
                    at += width + 1;
                    span
                })
                .collect()
        }
        Item::Buttons(buttons) => {
            // DIF_CENTERGROUP: centered on the dialog, or on the stretch it is placed in
            let (from, width) = match place {
                Some(p) if p.x2 > 0 => (frame.x + p.x.max(0) as u16, p.x2 + 1 - p.x.max(0) as u16),
                _ => (frame.x, frame.w),
            };
            let mut at = from + width.saturating_sub(buttons_len(buttons)) / 2;
            buttons
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let width = button_width(b);
                    let span = (i, at, width);
                    at += width + 1;
                    span
                })
                .collect()
        }
    }
}

/// The width the content wants (Far's MaxLength), for the dialogs sized by it.
fn natural_width(item: &Item) -> u16 {
    match item {
        Item::Text { text, .. } => text.chars().count() as u16,
        Item::Label(label) => label_len(label),
        Item::Edit { .. } | Item::Separator(_) | Item::Line(_) => 0,
        Item::Masked { mask, .. } => mask.len() as u16,
        Item::Check { label, .. } | Item::Check3 { label, .. } | Item::Option { label, .. } => 4 + label_len(label),
        Item::Radio { label, options, .. } => label_len(label) + 1 + options.iter().map(|o| 5 + label_len(o)).sum::<u16>(),
        Item::Combo { label, options, .. } => label_len(label) + 3 + options.iter().map(|o| o.chars().count() as u16).max().unwrap_or(0),
        Item::Buttons(buttons) => buttons_len(buttons),
    }
}

/// Draws `label` with the letter after `&` in the hot color (`&&` is a plain `&`).
fn put_label(grid: &mut Grid, x: u16, y: u16, label: &str, fg: Color, bg: Color, hot: Color) {
    let mut col = x;
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        let (c, color) = match c {
            '&' if chars.peek() == Some(&'&') => {
                chars.next();
                ('&', fg)
            }
            '&' => match chars.next() {
                Some(next) => (next, hot),
                None => break,
            },
            c => (c, fg),
        };
        grid.put(col, y, &c.to_string(), color, bg);
        col += 1;
    }
}

fn hot_key(label: &str) -> Option<char> {
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.next() {
                Some('&') => continue,
                Some(next) => return next.to_lowercase().next(),
                None => return None,
            }
        }
    }
    None
}

/// `text` cut in the middle with `…` to `width` (Far's truncate_center).
fn truncate_center(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let left = (width - 1) / 2;
    let right = width - 1 - left;
    let mut out: String = chars[..left].iter().collect();
    out.push('\u{2026}');
    out.extend(&chars[chars.len() - right..]);
    out
}

// ---------------------------------------------------------------- drawing

impl Dialog {
    fn frame(&self, size: (u16, u16)) -> Frame {
        let h = self.items.len() as u16 + 4;
        let content = |cap: u16| {
            let widest = self.items.iter().map(natural_width).max().unwrap_or(0);
            let buttons = self.items.iter().map(|i| if let Item::Buttons(b) = i { buttons_len(b) } else { 0 }).max().unwrap_or(0);
            widest.max(self.title.chars().count() as u16 + 2).min(cap.max(buttons)) + 10
        };
        match self.layout {
            // message.cpp: no wider than the screen less 10
            Layout::Message => Frame::new(size, content(size.0.saturating_sub(11)), h, true),
            // delete.cpp truncates its lines to the screen less 12
            Layout::Builder => Frame::new(size, content(size.0.saturating_sub(12)), h, false),
            Layout::Fixed(w) => Frame::new(size, w, h, false),
            Layout::Placed(w, h) => Frame::new(size, w, h, false),
        }
    }

    /// The row of item `i`.
    fn item_y(&self, frame: &Frame, i: usize) -> u16 {
        self.places.get(i).map_or(frame.row_y(i), |p| frame.y + p.y)
    }

    fn item_spans(&self, frame: &Frame, i: usize) -> Vec<Span> {
        spans_at(&self.items[i], frame, self.places.get(i))
    }

    /// Draws the dialog centered on `grid`; returns where the text cursor goes.
    pub fn paint(&mut self, grid: &mut Grid) -> Option<(u16, u16)> {
        self.localize();
        if let Some(sub) = &mut self.sub {
            sub.localize();
        }
        self.draw(grid)
    }

    /// Draws the dialog (and the one over it) as it is.
    pub(crate) fn draw(&self, grid: &mut Grid) -> Option<(u16, u16)> {
        let p = self.style.palette();
        let frame = self.frame((grid.cols, grid.rows));
        frame.draw(grid, &self.title, &p);
        let mut cursor = None;
        let text_w = frame.text_w() as usize;
        let base = p;
        let dim = Palette { fg: base.dim, hot: base.dim, edit_fg: base.dim, unchanged_fg: base.dim, ..self.style.palette() };
        for (i, item) in self.items.iter().enumerate() {
            if self.hidden.get(i).copied().unwrap_or(false) {
                continue;
            }
            let p = if self.disabled.get(i).copied().unwrap_or(false) { &dim } else { &base };
            let y = self.item_y(&frame, i);
            let place = self.places.get(i);
            let x = place.map_or(frame.text_x(), |p| frame.x + p.x.max(0) as u16);
            let focused = i == self.focus;
            let spans = self.item_spans(&frame, i);
            match item {
                Item::Text { text, align, highlight } => {
                    let text = truncate_center(text, text_w);
                    let len = text.chars().count() as u16;
                    let at = match (place, align) {
                        (Some(p), _) if p.x < 0 => frame.x + frame.w.saturating_sub(len) / 2,
                        (Some(p), Align::Center) if p.x2 > 0 => x + (p.x2 + 1 - p.x as u16).saturating_sub(len) / 2,
                        (Some(_), _) | (None, Align::Left) => x,
                        (None, Align::Center) => frame.x + frame.w.saturating_sub(len) / 2,
                    };
                    grid.put(at, y, &text, if *highlight { p.hot } else { p.fg }, p.bg);
                }
                Item::Line([first, middle, last]) => {
                    let Some(place) = place else { continue };
                    if place.y2 > place.y {
                        for row in place.y..=place.y2 {
                            let c = if row == place.y { first } else if row == place.y2 { last } else { middle };
                            grid.put(x, frame.y + row, &c.to_string(), p.fg, p.bg);
                        }
                    } else {
                        let from = x;
                        let to = frame.x + place.x2;
                        for col in from..=to {
                            let c = if col == from { first } else if col == to { last } else { middle };
                            grid.put(col, y, &c.to_string(), p.fg, p.bg);
                        }
                    }
                }
                Item::Check3 { label, state, enabled, .. } => {
                    let (fg, hot) = if *enabled { (p.fg, p.hot) } else { (p.dim, p.dim) };
                    let mark = match state {
                        1 => 'x',
                        2 => '?',
                        _ => ' ',
                    };
                    grid.put(spans[0].1, y, &format!("[{mark}] "), fg, p.bg);
                    put_label(grid, spans[0].1 + 4, y, label, fg, p.bg, hot);
                    if focused {
                        cursor = Some((spans[0].1 + 1, y));
                    }
                }
                Item::Option { label, on, .. } => {
                    grid.put(x, y, &format!("({}) ", if *on { '\u{2022}' } else { ' ' }), p.fg, p.bg);
                    put_label(grid, x + 4, y, label, p.fg, p.bg, p.hot);
                    if focused {
                        cursor = Some((x + 1, y));
                    }
                }
                Item::Masked { text, cursor: at, unchanged, .. } => {
                    let (fx, w) = (spans[0].1, spans[0].2);
                    let fg = if *unchanged { p.unchanged_fg } else { p.edit_fg };
                    grid.fill_with(fx, y, w, 1, fg, p.edit_bg);
                    grid.put(fx, y, &text.iter().collect::<String>(), fg, p.edit_bg);
                    if focused {
                        cursor = Some((fx + *at as u16, y));
                    }
                }
                Item::Label(label) => put_label(grid, x, y, label, p.fg, p.bg, p.hot),
                Item::Separator(title) => frame.separator(grid, y, *title, &p),
                Item::Edit { label, text, cursor: at, unchanged, enabled, history, secret, .. } => {
                    if let Some(label) = label {
                        // Far keeps the label's colors when the line is disabled
                        put_label(grid, x, y, label, p.fg, p.bg, p.hot);
                    }
                    let (fx, w) = (spans[0].1, spans[0].2);
                    let start = (*at + 1).saturating_sub(w as usize);
                    let visible: String = text.iter().skip(start).take(w as usize).map(|&c| if *secret { '*' } else { c }).collect();
                    let fg = if *enabled { p.edit_fg } else { p.dim };
                    grid.fill_with(fx, y, w, 1, fg, p.edit_bg);
                    grid.put(fx, y, &visible, fg, p.edit_bg);
                    if *unchanged && *enabled {
                        grid.put(fx, y, &visible, p.unchanged_fg, p.edit_bg);
                    }
                    // Dialog settings: History in dialog edit controls
                    if *history && crate::settings::bool("Dialog.EditHistory") {
                        grid.put(fx + w, y, "\u{2193}", if *enabled { p.fg } else { p.dim }, p.bg);
                    }
                    if focused {
                        cursor = Some((fx + at.saturating_sub(start) as u16, y));
                    }
                }
                Item::Check { label, checked, enabled, .. } => {
                    let (fg, hot) = if *enabled { (p.fg, p.hot) } else { (p.dim, p.dim) };
                    grid.put(spans[0].1, y, &format!("[{}] ", if *checked { 'x' } else { ' ' }), fg, p.bg);
                    put_label(grid, spans[0].1 + 4, y, label, fg, p.bg, hot);
                    if focused {
                        cursor = Some((spans[0].1 + 1, y));
                    }
                }
                Item::Radio { label, options, selected, .. } => {
                    put_label(grid, x, y, label, p.fg, p.bg, p.hot);
                    for ((i, ox, _), name) in spans.iter().zip(options) {
                        let on = i == selected;
                        grid.put(*ox, y, &format!("({}) ", if on { '\u{2022}' } else { ' ' }), p.fg, p.bg);
                        put_label(grid, *ox + 4, y, name, p.fg, p.bg, p.hot);
                        if focused && on {
                            cursor = Some((*ox + 1, y));
                        }
                    }
                }
                Item::Combo { label, options, selected, unchanged, .. } => {
                    put_label(grid, x, y, label, p.fg, p.bg, p.hot);
                    let (fx, w) = (spans[0].1, spans[0].2);
                    let (fg, bg) = if focused {
                        (p.selected_fg, p.selected_bg)
                    } else if *unchanged {
                        (p.unchanged_fg, p.edit_bg)
                    } else {
                        (p.edit_fg, p.edit_bg)
                    };
                    let value = shown(options[*selected]);
                    grid.fill_with(fx, y, w, 1, fg, bg);
                    grid.put(fx, y, &value, fg, bg);
                    grid.put(fx + w, y, "\u{2193}", p.fg, p.bg);
                    if focused {
                        cursor = Some((fx + value.chars().count() as u16, y));
                    }
                }
                Item::Buttons(buttons) => {
                    for ((i, bx, _), b) in spans.iter().zip(buttons) {
                        let on = focused && *i == self.button;
                        let (fg, bg, hot) = if !b.enabled {
                            (p.dim, p.bg, p.dim)
                        } else if on {
                            (p.focus_fg, p.focus_bg, p.hot)
                        } else {
                            (p.fg, p.bg, p.hot)
                        };
                        if b.id.starts_with('~') {
                            put_label(grid, *bx, y, b.label, fg, bg, hot);
                            continue;
                        }
                        let (open, close) = if b.default { ('{', '}') } else { ('[', ']') };
                        grid.put(*bx, y, &format!("{open} "), fg, bg);
                        put_label(grid, *bx + 2, y, b.label, fg, bg, hot);
                        grid.put(*bx + 2 + label_len(b.label), y, &format!(" {close}"), fg, bg);
                    }
                }
            }
        }
        match &self.sub {
            Some(sub) => sub.draw(grid),
            None => cursor,
        }
    }
}

// ---------------------------------------------------------------- Files menu

impl Dialog {
    /// Alt++ / Alt+- (filelist.cpp's SelectFiles): 55 by 7, the mask from
    /// 5 to 49, OK / Filter / Cancel. The filter is not done: disabled.
    pub fn select_group(add: bool, mask: &str) -> Self {
        let items = vec![
            edit("mask", mask),
            Item::Separator(None),
            Item::Buttons(vec![button("OK", "ok", true), Button { label: "&Filter", id: "filter", default: false, enabled: false, stay: false }, button("Cancel", "cancel", false)]),
        ];
        let places = vec![span(5, 2, 49), at(-1, 3), at(-1, 4)];
        Self::placed(if add { "Select" } else { "Deselect" }.into(), (55, 7), items, places, 0, None, Action::SelectGroup(add))
    }

    /// Far's GetString with FIB_BUTTONS (stddlg.cpp): 76 by 8, the prompt
    /// at (5, 2) cut to 66, the line from 5 to 70, OK and Cancel.
    pub fn get_string(title: &str, prompt: &str, value: &str, action: Action) -> Self {
        let prompt: String = prompt.chars().take(66).collect();
        let items = vec![
            text(prompt),
            edit("text", value),
            Item::Separator(None),
            Item::Buttons(vec![button("OK", "ok", true), button("Cancel", "cancel", false)]),
        ];
        let places = vec![at(5, 2), span(5, 3, 70), at(-1, 4), at(-1, 5)];
        Self::placed(title.into(), (76, 8), items, places, 1, None, action)
    }

    /// Alt+F6 (copy.cpp with Link): the copy dialog, 76 by 14, without the
    /// access rights, the timestamps and the link contents; the link type
    /// where "Already existing files" is, a hard link when files are among
    /// the sources, else a junction.
    pub fn link(label: String, dest: &str, files_present: bool, sources: Vec<PathBuf>) -> Self {
        let items = vec![
            Item::Label(label),
            edit("dest", dest),
            Item::Separator(None),
            Item::Label("Link t&ype:".into()),
            Item::Combo { id: "kind", label: "", x: 29, options: crate::links::LinkType::LABELS.to_vec(), selected: if files_present { 0 } else { 1 }, unchanged: true },
            Item::Check { id: "multiple", label: "Process &multiple destinations", checked: false, enabled: true },
            Item::Separator(None),
            Item::Check { id: "filter", label: "&Use filter", checked: false, enabled: false },
            Item::Separator(None),
            Item::Buttons(vec![
                button("Create &link", "ok", true),
                Button { label: "Filt&er", id: "filter", default: false, enabled: false, stay: false },
                button("Ca&ncel", "cancel", false),
            ]),
        ];
        let places = vec![at(5, 2), span(5, 3, 70), at(-1, 4), at(5, 5), span(29, 5, 70), at(5, 7), at(-1, 8), at(5, 9), at(-1, 10), at(-1, 11)];
        Self::placed("Link".into(), (76, 14), items, places, 1, None, Action::Link(sources))
    }
}

// ---------------------------------------------------------------- input

impl Dialog {
    fn focusable(&self, index: usize) -> bool {
        if self.is_disabled(index) {
            return false;
        }
        match &self.items[index] {
            Item::Radio { .. } | Item::Combo { .. } | Item::Masked { .. } => true,
            // a group is entered at the one that is on
            Item::Option { on, .. } => *on,
            Item::Edit { enabled, .. } | Item::Check { enabled, .. } | Item::Check3 { enabled, .. } => *enabled,
            Item::Buttons(buttons) => buttons.iter().any(|b| b.enabled),
            Item::Text { .. } | Item::Label(_) | Item::Separator(_) | Item::Line(_) => false,
        }
    }

    /// Tells the hook, if the dialog has one.
    fn notify(&mut self, event: Event) {
        if let Some(hook) = self.hook {
            hook(self, event);
        }
    }

    /// Space or a hot key on a check box: the next state, and the hook hears of it.
    fn toggle(&mut self, index: usize) {
        let id = match &mut self.items[index] {
            Item::Check { id, checked, .. } => {
                *checked = !*checked;
                *id
            }
            // Far's order: off, on, then (3-state) "leave as it is"
            Item::Check3 { id, state, three, .. } => {
                *state = (*state + 1) % if *three { 3 } else { 2 };
                *id
            }
            _ => return,
        };
        self.apply_links();
        self.notify(Event::Toggled(id));
    }

    /// LinkFlags: each linked item enabled while its check box is on.
    pub(crate) fn apply_links(&mut self) {
        for (parent, target) in self.links.clone() {
            let on = self.check_state(parent) == 1 && !self.is_disabled(parent);
            self.enable(target, on);
        }
    }

    pub(crate) fn link_flags(&mut self, parent: usize, target: usize) {
        self.links.push((parent, target));
    }

    /// The texts into the interface language (Far's English ones are looked up).
    fn localize(&mut self) {
        if std::mem::replace(&mut self.localized, true) || !crate::lang::russian() {
            return;
        }
        use crate::lang::{tr, trs};
        self.title = trs(&self.title);
        for item in &mut self.items {
            match item {
                Item::Text { text, .. } | Item::Label(text) => *text = trs(text),
                Item::Edit { label: Some(label), .. } => *label = tr(label),
                Item::Radio { label, options, .. } | Item::Combo { label, options, .. } => {
                    *label = tr(label);
                    for option in options.iter_mut() {
                        *option = tr(option);
                    }
                }
                Item::Check { label, .. } | Item::Check3 { label, .. } | Item::Option { label, .. } => *label = tr(label),
                Item::Separator(Some(text)) => *text = tr(text),
                Item::Buttons(buttons) => {
                    for button in buttons.iter_mut() {
                        button.label = tr(button.label);
                    }
                }
                _ => {}
            }
        }
    }

    fn move_focus(&mut self, forward: bool) {
        // each button is a control of its own: Tab goes through them in turn
        if let Item::Buttons(buttons) = &self.items[self.focus] {
            let next = if forward {
                (self.button + 1..buttons.len()).find(|&i| buttons[i].enabled)
            } else {
                (0..self.button).rev().find(|&i| buttons[i].enabled)
            };
            if let Some(next) = next {
                self.button = next;
                return;
            }
        }
        let count = self.items.len();
        let mut at = self.focus;
        for _ in 0..count {
            at = if forward { (at + 1) % count } else { (at + count - 1) % count };
            if self.focusable(at) {
                self.focus = at;
                if let Item::Buttons(buttons) = &self.items[at] {
                    let mut enabled = buttons.iter().enumerate().filter(|(_, b)| b.enabled).map(|(i, _)| i);
                    self.button = if forward { enabled.next() } else { enabled.last() }.unwrap_or(0);
                }
                return;
            }
        }
    }

    /// Focuses the first control after `index` (what a label's hot key does).
    fn focus_after(&mut self, index: usize) {
        if let Some(next) = (index + 1..self.items.len()).find(|&i| self.focusable(i)) {
            self.focus = next;
            self.button = self.first_enabled_button();
        }
    }

    fn first_enabled_button(&self) -> usize {
        match &self.items[self.focus] {
            Item::Buttons(buttons) => buttons.iter().position(|b| b.enabled && b.default).or_else(|| buttons.iter().position(|b| b.enabled)).unwrap_or(0),
            _ => 0,
        }
    }

    /// Presses the default button (Enter anywhere but on a button).
    fn press_default(&mut self) -> Outcome {
        let id = self.items.iter().find_map(|item| match item {
            Item::Buttons(buttons) => buttons.iter().find(|b| b.default && b.enabled).map(|b| b.id),
            _ => None,
        });
        // no buttons (Far's InputBox): the line itself is DIF_DEFAULTBUTTON
        let has_buttons = self.items.iter().any(|item| matches!(item, Item::Buttons(_)));
        match id {
            Some(id) => self.press(id),
            None if !has_buttons => self.press("ok"),
            None => Outcome::Open,
        }
    }

    fn press(&mut self, id: &'static str) -> Outcome {
        let stays = self.items.iter().any(|item| matches!(item, Item::Buttons(buttons) if buttons.iter().any(|b| b.id == id && b.stay)));
        if stays {
            self.notify(Event::Button(id));
            Outcome::Open
        } else if id == "cancel" {
            Outcome::Closed
        } else {
            self.notify(Event::Closing(id));
            if self.sub.is_some() {
                // the hook asks or tells something first (Far's DN_CLOSE refused)
                return Outcome::Open;
            }
            Outcome::Pressed(self.action.clone(), id, self.values())
        }
    }

    fn cycle(&mut self, step: isize) {
        match &mut self.items[self.focus] {
            Item::Radio { options, selected, .. } => {
                *selected = (*selected as isize + step).rem_euclid(options.len() as isize) as usize;
            }
            Item::Combo { id, options, selected, unchanged, .. } => {
                *selected = (*selected as isize + step).rem_euclid(options.len() as isize) as usize;
                *unchanged = false;
                // Far's DN_EDITCHANGE of a drop-down list
                let id = *id;
                self.notify(Event::Toggled(id));
            }
            _ => {}
        }
    }

    /// Sets the drop-down list `index` to `selected` (Far's DM_LISTSETCURPOS).
    pub(crate) fn set_list_pos(&mut self, index: usize, pos: usize) {
        if let Some(Item::Combo { options, selected, .. }) = self.items.get_mut(index) {
            *selected = pos.min(options.len().saturating_sub(1));
        }
    }

    pub(crate) fn list_pos(&self, index: usize) -> usize {
        match self.items.get(index) {
            Some(Item::Combo { selected, .. }) => *selected,
            _ => 0,
        }
    }

    /// A message over the dialog (Far's Message from a dialog procedure):
    /// the lines, an OK, in the warning colors.
    pub(crate) fn message_over(&mut self, title: &str, lines: &[&str], buttons: &[(&'static str, &'static str)]) {
        let buttons = buttons.iter().enumerate().map(|(i, (label, id))| button(label, id, i == 0)).collect();
        let mut message = Dialog::warning(title, lines.iter().map(|l| l.to_string()).collect(), buttons);
        message.action = Action::Info;
        self.sub = Some(Box::new(message));
    }

    /// A hot key: focuses the control, toggles a check box, picks a radio
    /// button, presses a button.
    fn hot(&mut self, c: char) -> Option<Outcome> {
        let c = c.to_lowercase().next().unwrap_or(c);
        for index in 0..self.items.len() {
            if self.is_disabled(index) {
                continue;
            }
            if let Item::Option { label, .. } = &self.items[index]
                && hot_key(label) == Some(c)
            {
                self.select_option(index);
                self.focus = index;
                self.notify(Event::Toggled(option_id(&self.items[index])));
                return Some(Outcome::Open);
            }
            match &mut self.items[index] {
                Item::Label(label) if hot_key(label) == Some(c) => {
                    self.focus_after(index);
                    return Some(Outcome::Open);
                }
                Item::Check { label, enabled, .. } | Item::Check3 { label, enabled, .. } if *enabled && hot_key(label) == Some(c) => {
                    self.focus = index;
                    self.toggle(index);
                    return Some(Outcome::Open);
                }
                Item::Edit { label: Some(label), enabled: true, .. } if hot_key(label) == Some(c) => {
                    self.focus = index;
                    return Some(Outcome::Open);
                }
                Item::Combo { label, .. } if hot_key(label) == Some(c) => {
                    self.focus = index;
                    return Some(Outcome::Open);
                }
                Item::Radio { label, options, selected, .. } => {
                    if let Some(option) = options.iter().position(|o| hot_key(o) == Some(c)) {
                        *selected = option;
                        self.focus = index;
                        return Some(Outcome::Open);
                    }
                    if hot_key(label) == Some(c) {
                        self.focus = index;
                        return Some(Outcome::Open);
                    }
                }
                Item::Buttons(buttons) => {
                    let id = buttons.iter().find(|b| b.enabled && hot_key(b.label) == Some(c)).map(|b| b.id);
                    if let Some(id) = id {
                        return Some(self.press(id));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// What the dialog over this one made of a key or click: it closed, the
    /// hook hears of it.
    fn after_sub(&mut self, outcome: Outcome) -> Outcome {
        match outcome {
            Outcome::Open => {}
            Outcome::Closed => {
                self.sub = None;
                self.notify(Event::SubClosed(None));
            }
            Outcome::Pressed(_, id, values) => {
                self.sub = None;
                self.notify(Event::SubClosed(Some((id, values))));
            }
        }
        match self.closing.take() {
            Some(id) => Outcome::Pressed(self.action.clone(), id, self.values()),
            None => Outcome::Open,
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        self.localize();
        if let Some(sub) = &mut self.sub {
            let outcome = sub.key(key);
            return self.after_sub(outcome);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if key.code == KeyCode::Esc {
            return Outcome::Closed;
        }
        let on_edit = matches!(self.items[self.focus], Item::Edit { .. } | Item::Masked { .. });
        if let KeyCode::Char(c) = key.code {
            // Alt+letter always, a bare letter where nothing is being typed.
            if (alt || (!on_edit && !ctrl)) && c != ' ' {
                if let Some(outcome) = self.hot(c).or_else(|| crate::keys::latin(c).and_then(|l| self.hot(l))) {
                    return outcome;
                }
            }
        }
        if !matches!(key.code, KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down | KeyCode::Enter) {
            self.edit_key(key.code, ctrl, alt);
            // DN_EDITCHANGE
            if let Some(Item::Edit { id, .. }) = self.items.get(self.focus) {
                let id = *id;
                self.notify(Event::Edited(id));
            }
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.move_focus(true),
            KeyCode::BackTab | KeyCode::Up => self.move_focus(false),
            KeyCode::Enter => {
                if let Item::Buttons(buttons) = &self.items[self.focus] {
                    let id = buttons.get(self.button).filter(|b| b.enabled).map(|b| b.id);
                    return id.map_or(Outcome::Open, |id| self.press(id));
                }
                return self.press_default();
            }
            KeyCode::Left | KeyCode::Right => {
                let step = if key.code == KeyCode::Left { -1 } else { 1 };
                match &mut self.items[self.focus] {
                    Item::Buttons(buttons) => {
                        let count = buttons.len() as isize;
                        let mut at = self.button as isize;
                        for _ in 0..count {
                            at = (at + step).rem_euclid(count);
                            if buttons[at as usize].enabled {
                                self.button = at as usize;
                                break;
                            }
                        }
                    }
                    Item::Radio { .. } | Item::Combo { .. } => self.cycle(step),
                    Item::Option { id, .. } => {
                        // to the group's next (or previous) button, which turns on
                        let group = *id;
                        let members: Vec<usize> = (0..self.items.len())
                            .filter(|&i| matches!(&self.items[i], Item::Option { id, .. } if *id == group) && !self.is_disabled(i))
                            .collect();
                        if let Some(at) = members.iter().position(|&i| i == self.focus) {
                            let next = members[(at as isize + step).rem_euclid(members.len() as isize) as usize];
                            self.select_option(next);
                            self.focus = next;
                            self.notify(Event::Toggled(group));
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Char(' ') => match &self.items[self.focus] {
                Item::Check { .. } | Item::Check3 { .. } => self.toggle(self.focus),
                Item::Option { .. } => {
                    let focus = self.focus;
                    self.select_option(focus);
                    self.notify(Event::Toggled(option_id(&self.items[focus])));
                }
                Item::Radio { .. } | Item::Combo { .. } => self.cycle(1),
                Item::Buttons(buttons) => {
                    let id = buttons.get(self.button).filter(|b| b.enabled).map(|b| b.id);
                    if let Some(id) = id {
                        return self.press(id);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Outcome::Open
    }

    /// What a key does to the focused input line (edit.cpp): the first
    /// character typed into unchanged text replaces it, Del at its end
    /// clears it, and any key makes it changed.
    fn edit_key(&mut self, code: KeyCode, ctrl: bool, alt: bool) {
        if let Item::Masked { mask, text, cursor, unchanged, .. } = &mut self.items[self.focus] {
            masked_key(mask, text, cursor, unchanged, code, ctrl || alt);
            return;
        }
        let Item::Edit { text, cursor, unchanged, .. } = &mut self.items[self.focus] else { return };
        let was_unchanged = std::mem::replace(unchanged, false);
        match code {
            KeyCode::Left => *cursor = cursor.saturating_sub(1),
            KeyCode::Right => *cursor = (*cursor + 1).min(text.len()),
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = text.len(),
            KeyCode::Backspace => {
                if *cursor > 0 {
                    *cursor -= 1;
                    text.remove(*cursor);
                }
            }
            KeyCode::Delete => {
                if was_unchanged && *cursor >= text.len() {
                    text.clear();
                    *cursor = 0;
                } else if *cursor < text.len() {
                    text.remove(*cursor);
                }
            }
            KeyCode::Char('u') if ctrl => {
                text.clear();
                *cursor = 0;
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                if was_unchanged {
                    text.clear();
                    *cursor = 0;
                }
                text.insert(*cursor, c);
                *cursor += 1;
            }
            _ => *unchanged = was_unchanged,
        }
    }

    /// What a click at (`col`, `row`) does, on a screen of `size`.
    pub fn click(&mut self, col: u16, row: u16, size: (u16, u16)) -> Outcome {
        self.localize();
        if let Some(sub) = &mut self.sub {
            let outcome = sub.click(col, row, size);
            return self.after_sub(outcome);
        }
        let frame = self.frame(size);
        // Dialog settings: Mouse click outside a dialog closes it
        let outside = col < frame.x || col >= frame.x + frame.w || row < frame.y || row >= frame.y + frame.h;
        if outside {
            return if crate::settings::bool("Dialog.MouseButton") { Outcome::Closed } else { Outcome::Open };
        }
        for index in 0..self.items.len() {
            if row != self.item_y(&frame, index) {
                continue;
            }
            for (sub, x, w) in self.item_spans(&frame, index) {
                if col < x || col >= x + w {
                    continue;
                }
                if matches!(self.items[index], Item::Option { .. }) && !self.is_disabled(index) {
                    self.select_option(index);
                    self.focus = index;
                    self.notify(Event::Toggled(option_id(&self.items[index])));
                    return Outcome::Open;
                }
                if !self.focusable(index) {
                    return Outcome::Open;
                }
                self.focus = index;
                let mut pressed = None;
                let mut toggled = false;
                match &mut self.items[index] {
                    Item::Edit { text, cursor, unchanged, .. } => {
                        let start = (*cursor + 1).saturating_sub(w as usize);
                        *cursor = (start + (col - x) as usize).min(text.len());
                        *unchanged = false;
                    }
                    Item::Masked { text, cursor, unchanged, .. } => {
                        *cursor = ((col - x) as usize).min(text.len().saturating_sub(1));
                        *unchanged = false;
                    }
                    Item::Check { .. } | Item::Check3 { .. } => toggled = true,
                    Item::Radio { selected, .. } => *selected = sub,
                    Item::Combo { .. } => self.cycle(1),
                    Item::Buttons(buttons) => {
                        self.button = sub;
                        pressed = buttons.get(sub).filter(|b| b.enabled).map(|b| b.id);
                    }
                    _ => {}
                }
                if toggled {
                    self.toggle(index);
                }
                return pressed.map_or(Outcome::Open, |id| self.press(id));
            }
        }
        Outcome::Open
    }
}

/// A key in a masked field (edit.cpp with a mask): a digit fills the slot
/// under the cursor and moves to the next one, the fixed characters are
/// stepped over, Backspace and Del blank a slot; typed into an unchanged
/// field, a digit first blanks it all.
fn masked_key(mask: &[char], text: &mut [char], cursor: &mut usize, unchanged: &mut bool, code: KeyCode, modified: bool) {
    let slots: Vec<usize> = (0..mask.len()).filter(|&i| is_mask_slot(mask[i])).collect();
    let (Some(&first), Some(&last)) = (slots.first(), slots.last()) else { return };
    let next = |from: usize| slots.iter().copied().find(|&i| i > from);
    let previous = |from: usize| slots.iter().rev().copied().find(|&i| i < from);
    let was_unchanged = std::mem::replace(unchanged, false);
    match code {
        KeyCode::Left => *cursor = previous(*cursor).unwrap_or(first),
        KeyCode::Right => *cursor = next(*cursor).unwrap_or(last),
        KeyCode::Home => *cursor = first,
        KeyCode::End => *cursor = last,
        KeyCode::Backspace => {
            if let Some(at) = previous(*cursor) {
                text[at] = ' ';
                *cursor = at;
            }
        }
        KeyCode::Delete => {
            if is_mask_slot(mask[*cursor]) {
                text[*cursor] = ' ';
            }
        }
        KeyCode::Char(c) if !modified && (c.is_ascii_hexdigit() || c == ' ') => {
            let at = if was_unchanged { first } else if is_mask_slot(mask[*cursor]) { *cursor } else { next(*cursor).unwrap_or(last) };
            let Some(c) = mask_fits(mask[at], c) else {
                *unchanged = was_unchanged;
                return;
            };
            if was_unchanged {
                for &i in &slots {
                    text[i] = ' ';
                }
            }
            *cursor = at;
            text[*cursor] = c;
            *cursor = next(*cursor).unwrap_or(last);
        }
        _ => *unchanged = was_unchanged,
    }
}

// ---------------------------------------------------------------- progress

/// Far's delete_progress (delete.cpp, DlgW 76): "Deleting/Wiping the file or
/// folder", the name, for a wipe the file's bar, a separator, the files
/// done (Far's DelShowTotal is off: no total bar).
fn paint_delete_progress(job: &Job, grid: &mut Grid) {
    const DLG_W: u16 = 76;
    let wipe = job.kind == Kind::Wipe;
    let p = Style::Normal.palette();
    let progress = &job.progress;
    // DlgH 10, a row less without the wipe bar, another without the total bar
    let h = if wipe { 9 } else { 8 };
    let frame = Frame::new((grid.cols, grid.rows), DLG_W, h, false);
    frame.draw(grid, job.kind.title(), &p);
    let x = frame.x + 5;
    let width = (DLG_W - 10) as usize;
    let y = |row: u16| frame.y + row;
    grid.put(x, y(2), if wipe { "Wiping the file or folder" } else { "Deleting the file or folder" }, p.fg, p.bg);
    grid.put(x, y(3), &progress.from.chars().take(width).collect::<String>(), p.fg, p.bg);
    let mut row = 4;
    if wipe {
        let percent = if progress.file_size == 0 { 0 } else { progress.file_done * 100 / progress.file_size };
        grid.put(x, y(row), &progressbar(width, percent as usize), p.fg, p.bg);
        row += 1;
    }
    frame.separator(grid, y(row), None, &p);
    grid.put(x, y(row + 1), &counter("Files:", "Bytes:", progress.files_done as u64, width - 5), p.fg, p.bg);
}

/// Far's make_progressbar with the percent: `█` done, `░` to do, ` NN%`.
fn progressbar(size: usize, percent: usize) -> String {
    let percent_text = format!(" {percent:3}%");
    let size = size.saturating_sub(percent_text.len());
    let done = percent.min(100) * size / 100;
    format!("{}{}{percent_text}", "\u{2588}".repeat(done), "\u{2591}".repeat(size - done))
}

/// Far's copy_progress::FormatCounter without a total: the label padded to
/// the longer of the two, the number right-aligned to `max_width`.
fn counter(label: &str, other: &str, value: u64, max_width: usize) -> String {
    let padded = label.len().max(other.len()) + 1;
    let value = group(value);
    format!("{label:<padded$}{value:>width$}", width = max_width.saturating_sub(padded))
}

/// arclite's ArchiveUpdateProgress (update.cpp, c_width 60): the archive,
/// the file with its bytes and bar, the total with the speed and what is
/// left, the total bar.
fn paint_pack_progress(job: &Job, grid: &mut Grid) {
    const WIDTH: usize = 60;
    let p = Style::Normal.palette();
    let progress = &job.progress;
    let frame = Frame::new((grid.cols, grid.rows), WIDTH as u16 + 10, 12, true);
    let percent = |done: u64, total: u64| if total == 0 { 0 } else { (done * 100 / total) as usize };
    frame.draw(grid, &format!("{{{}%}} {}", percent(progress.bytes_done, progress.bytes_total), job.kind.title()), &p);
    let x = frame.x + 5;
    let fit = |text: &str| -> String {
        let count = text.chars().count();
        if count <= WIDTH { text.to_string() } else { format!("\u{2026}{}", text.chars().skip(count - WIDTH + 1).collect::<String>()) }
    };
    let seconds = job.elapsed().as_secs_f64().max(0.001);
    let speed = progress.bytes_done as f64 / seconds;
    let left = if speed > 1.0 { (progress.bytes_total.saturating_sub(progress.bytes_done) as f64 / speed) as u64 } else { 0 };
    let y = frame.y;
    let line = |grid: &mut Grid, row: u16, text: &str| grid.put(x, y + row, text, p.fg, p.bg);
    line(grid, 2, &fit(&progress.to));
    frame.separator(grid, y + 3, None, &p);
    line(grid, 4, &fit(&progress.from));
    line(grid, 5, &format!("{:>7} / {}", data_size(progress.file_done), data_size(progress.file_size)));
    line(grid, 6, &progressbar(WIDTH, percent(progress.file_done, progress.file_size)));
    frame.separator(grid, y + 7, None, &p);
    let speed_text = format!("{}/s", data_size(speed as u64));
    line(grid, 8, &format!("{:>7} / {} @ {speed_text:>9} -{}", data_size(progress.bytes_done), data_size(progress.bytes_total), clock(left)));
    line(grid, 9, &progressbar(WIDTH, percent(progress.bytes_done, progress.bytes_total)));
}

/// arclite's format_data_size: at most four digits and a unit.
fn data_size(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 * 10.0 && unit + 1 < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{} {}", value.round() as u64, units[unit])
}

/// The progress box of a running copy, move, delete, wipe or archive.
pub fn paint_progress(job: &Job, grid: &mut Grid) {
    if matches!(job.kind, Kind::Delete | Kind::Wipe) {
        return paint_delete_progress(job, grid);
    }
    if matches!(job.kind, Kind::Pack | Kind::Unpack | Kind::Update | Kind::Test) {
        return paint_pack_progress(job, grid);
    }
    let p = Style::Normal.palette();
    let progress = &job.progress;
    let copying = job.kind != Kind::Delete;
    let rows: u16 = if copying { 11 } else { 7 };
    let frame = Frame::sized((grid.cols, grid.rows), job.kind.title(), (grid.cols * 3 / 4).clamp(40, 74), rows);
    frame.draw(grid, job.kind.title(), &p);
    let (x, w) = (frame.text_x(), frame.text_w() as usize);
    let fit = |text: &str| -> String {
        let count = text.chars().count();
        if count <= w {
            text.to_string()
        } else {
            format!("\u{2026}{}", text.chars().skip(count - w + 1).collect::<String>())
        }
    };
    let mut row = 0;
    let mut line = |grid: &mut Grid, text: &str| {
        grid.put(x, frame.row_y(row), text, p.fg, p.bg);
        row += 1;
    };
    let verb = match job.kind {
        Kind::Copy => "Copying the file",
        Kind::Move => "Moving the file",
        Kind::Delete | Kind::Wipe => "Deleting the file",
        Kind::Pack | Kind::Unpack | Kind::Update | Kind::Test => "Adding the file",
    };
    line(grid, verb);
    line(grid, &fit(&progress.from));
    if copying {
        line(grid, "to");
        line(grid, &fit(&progress.to));
        bar(grid, x, frame.row_y(row), w, progress.file_done, progress.file_size, &p);
        row += 1;
    }
    frame.separator(grid, frame.row_y(row), Some("Total"), &p);
    row += 1;
    let right = |grid: &mut Grid, y: u16, label: &str, value: &str| {
        grid.put(x, y, label, p.fg, p.bg);
        grid.put(x + (w as u16).saturating_sub(value.chars().count() as u16), y, value, p.fg, p.bg);
    };
    right(grid, frame.row_y(row), "Files:", &format!("{} / {}", progress.files_done, progress.files_total));
    row += 1;
    if copying {
        right(grid, frame.row_y(row), "Bytes:", &format!("{} / {}", group(progress.bytes_done), group(progress.bytes_total)));
        row += 1;
        bar(grid, x, frame.row_y(row), w, progress.bytes_done, progress.bytes_total, &p);
    } else {
        bar(grid, x, frame.row_y(row), w, progress.files_done as u64, progress.files_total as u64, &p);
    }
    row += 1;
    frame.separator(grid, frame.row_y(row), None, &p);
    row += 1;
    let seconds = job.elapsed().as_secs_f64().max(0.001);
    let speed = progress.bytes_done as f64 / seconds;
    let left = if speed > 1.0 { (progress.bytes_total.saturating_sub(progress.bytes_done) as f64 / speed) as u64 } else { 0 };
    let y = frame.row_y(row);
    let time = format!("Time: {}", clock(seconds as u64));
    let remaining = format!("Remaining: {}", clock(left));
    let rate = format!("{} MB/s", (speed / 1_048_576.0) as u64);
    grid.put(x, y, &time, p.fg, p.bg);
    grid.put(x + (w as u16).saturating_sub(remaining.chars().count() as u16) / 2, y, &remaining, p.fg, p.bg);
    grid.put(x + (w as u16).saturating_sub(rate.chars().count() as u16), y, &rate, p.fg, p.bg);
}

/// A bar `width` cells wide with the percentage at its end.
fn bar(grid: &mut Grid, x: u16, y: u16, width: usize, done: u64, total: u64, p: &Palette) {
    let fraction = if total == 0 { 0.0 } else { (done as f64 / total as f64).min(1.0) };
    let percent = format!("{:>3}%", (fraction * 100.0) as u32);
    let cells = width.saturating_sub(percent.len() + 1);
    let filled = (cells as f64 * fraction) as usize;
    let text: String = (0..cells).map(|i| if i < filled { '\u{2588}' } else { '\u{2591}' }).collect();
    grid.put(x, y, &text, p.fg, p.bg);
    grid.put(x + cells as u16 + 1, y, &percent, p.fg, p.bg);
}

fn clock(seconds: u64) -> String {
    format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// 1234567 as `1 234 567`.
pub fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// `03.11.26 00:02`, in the machine's local time.
fn date(time: Option<SystemTime>) -> String {
    time.map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%d.%m.%y %H:%M").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(dialog: &mut Dialog, code: KeyCode) -> Outcome {
        dialog.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(dialog: &mut Dialog, text: &str) {
        for c in text.chars() {
            press(dialog, KeyCode::Char(c));
        }
    }

    fn copy_dialog() -> Dialog {
        Dialog::copy_or_move(Kind::Copy, "Copy file \"a\" to:".into(), "/tmp/x", Action::Copy(vec![]))
    }

    fn pressed(outcome: Outcome) -> (&'static str, Values) {
        match outcome {
            Outcome::Pressed(_, id, values) => (id, values),
            _ => panic!("a button was pressed"),
        }
    }

    #[test]
    fn the_input_line_is_edited_and_enter_presses_the_default_button() {
        let mut dialog = copy_dialog();
        press(&mut dialog, KeyCode::Backspace);
        typed(&mut dialog, "yz");
        press(&mut dialog, KeyCode::Home);
        press(&mut dialog, KeyCode::Delete);
        let (id, values) = pressed(press(&mut dialog, KeyCode::Enter));
        assert_eq!(id, "ok");
        assert_eq!(values.text("dest"), "tmp/yz");
    }

    #[test]
    fn escape_and_the_cancel_button_close_the_dialog() {
        let mut dialog = copy_dialog();
        assert!(matches!(press(&mut dialog, KeyCode::Esc), Outcome::Closed));
        // Alt+N is the hot key of Cancel
        assert!(matches!(dialog.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT)), Outcome::Closed));
    }

    #[test]
    fn the_controls_are_reached_with_tab_and_changed_with_the_keys() {
        let mut dialog = copy_dialog();
        press(&mut dialog, KeyCode::Tab); // access rights
        press(&mut dialog, KeyCode::Right); // Copy
        press(&mut dialog, KeyCode::Tab); // existing files
        press(&mut dialog, KeyCode::Right); // Overwrite
        press(&mut dialog, KeyCode::Tab); // timestamps
        press(&mut dialog, KeyCode::Char(' '));
        let (_, values) = pressed(dialog.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        assert_eq!(values.choice("access"), 1);
        assert_eq!(values.choice("existing"), 1);
        assert!(values.checked("timestamps"));
        assert!(!values.checked("links"));
    }

    #[test]
    fn hot_keys_toggle_check_boxes_and_a_disabled_one_is_skipped() {
        let mut dialog = copy_dialog();
        dialog.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::ALT));
        assert!(dialog.values().checked("multiple"));
        dialog.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::ALT));
        assert!(!dialog.values().checked("filter"), "the disabled filter stays off");
        // Tab goes round without ever stopping on it or on its button
        let mut stops = Vec::new();
        for _ in 0..8 {
            press(&mut dialog, KeyCode::Tab);
            stops.push(dialog.focus);
        }
        assert!(stops.iter().all(|&i| dialog.focusable(i)));
    }

    #[test]
    fn a_question_is_answered_with_y_and_n_without_alt() {
        let mut dialog = Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit);
        // Far's Yes and No have no hot keys: letters do nothing, Enter is Yes
        assert!(matches!(press(&mut dialog, KeyCode::Char('n')), Outcome::Open));
        let (id, _) = pressed(press(&mut dialog, KeyCode::Enter));
        assert_eq!(id, "ok");
        let mut dialog = Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit);
        press(&mut dialog, KeyCode::Right);
        assert!(matches!(press(&mut dialog, KeyCode::Enter), Outcome::Closed), "focus moved to No");
    }

    #[test]
    fn tab_goes_through_each_button_and_skips_disabled_ones() {
        let mut dialog = copy_dialog();
        for _ in 0..6 {
            press(&mut dialog, KeyCode::Tab);
        }
        assert_eq!((dialog.focus, dialog.button), (dialog.items.len() - 1, 0), "on Copy");
        press(&mut dialog, KeyCode::Tab);
        assert_eq!(dialog.button, 2, "Filter is disabled: on Cancel");
        press(&mut dialog, KeyCode::Tab);
        assert_eq!(dialog.focus, 1, "round to the input line");
        press(&mut dialog, KeyCode::BackTab);
        assert_eq!(dialog.button, 2, "back to Cancel");
    }

    #[test]
    fn typing_letters_into_the_input_does_not_trigger_hot_keys() {
        let mut dialog = Dialog::mkdir();
        typed(&mut dialog, "ocm");
        let (_, values) = pressed(press(&mut dialog, KeyCode::Enter));
        assert_eq!(values.text("name"), "ocm");
        assert!(!values.checked("multiple"));
    }

    #[test]
    fn the_conflict_dialog_offers_overwrite_skip_cancel_and_remembers() {
        let conflict = Conflict { dst: "/x/y.txt".into(), new_size: 12_345, new_modified: None, old_size: 7, old_modified: None };
        let mut dialog = Dialog::conflict(&conflict);
        dialog.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT));
        let (id, values) = pressed(dialog.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)));
        assert_eq!(id, "skip");
        assert!(values.checked("remember"));
    }

    #[test]
    fn clicks_press_buttons_toggle_checks_and_place_the_cursor() {
        let size = (100, 30);
        let mut dialog = copy_dialog();
        let frame = dialog.frame(size);
        // the input line is row 1
        let (_, ix, _) = spans(&dialog.items[1], &frame)[0];
        assert!(matches!(dialog.click(ix + 2, frame.row_y(1), size), Outcome::Open));
        typed(&mut dialog, "X");
        // the multiple-destinations check box
        let row = dialog.items.iter().position(|i| matches!(i, Item::Check { id: "multiple", .. })).unwrap();
        let (_, cx, _) = spans(&dialog.items[row], &frame)[0];
        dialog.click(cx + 1, frame.row_y(row), size);
        // the Copy button
        let last = dialog.items.len() - 1;
        let (_, bx, _) = spans(&dialog.items[last], &frame)[0];
        let (id, values) = pressed(dialog.click(bx + 3, frame.row_y(last), size));
        assert_eq!(id, "ok");
        assert_eq!(values.text("dest"), "/tXmp/x");
        assert!(values.checked("multiple"));
        // Dialog settings' "Mouse click outside a dialog closes it" is on by default
        assert!(matches!(dialog.click(0, 0, size), Outcome::Closed), "outside closes it");
    }

    #[test]
    fn dialogs_paint_inside_the_screen_with_a_cursor_in_the_input() {
        let mut grid = Grid::new(100, 30);
        let cursor = copy_dialog().paint(&mut grid).expect("the cursor is in the input");
        assert!(cursor.0 < 100 && cursor.1 < 30);
        Dialog::mkdir().paint(&mut grid);
        Dialog::error("It failed\nfor a reason").paint(&mut grid);
        Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit).paint(&mut grid);
        Dialog::delete("Do you wish to delete the file?".into(), vec!["a".into()], true, Action::Quit).paint(&mut grid);
        // a tiny screen must not panic
        let mut tiny = Grid::new(10, 4);
        copy_dialog().paint(&mut tiny);
        Dialog::mkdir().paint(&mut tiny);
    }

    #[test]
    fn the_frame_has_far_margins_and_a_titled_top() {
        let mut grid = Grid::new(100, 30);
        Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit).paint(&mut grid);
        let frame = Dialog::confirm("Quit", "Do you want to quit farsh?", Action::Quit).frame((100, 30));
        assert_eq!(frame.fx - frame.x, 3, "three columns of grey before the frame");
        assert_eq!(frame.fy - frame.y, 1);
        assert_eq!(grid.cell_mut(frame.fx, frame.fy).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(frame.fx, frame.fy + 2).unwrap().text, "\u{255F}", "the separator joins the frame");
    }

    #[test]
    fn numbers_are_grouped_and_dates_are_read_from_the_epoch() {
        assert_eq!(group(0), "0");
        assert_eq!(group(1_234_567), "1 234 567");
        assert_eq!(group(114_021_279_984), "114 021 279 984");
        let shown = date(Some(SystemTime::now()));
        assert_eq!(shown.len(), 14, "{shown}");
        assert_eq!(date(None), "");
    }

    // ---------------------------------------------------------------- Attributes

    const DMY: attr::Format = attr::Format { order: attr::DateOrder::Dmy, date_separator: '.', time_separator: ':', decimal: ',' };

    fn scratch(name: &str, folder: bool) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("farsh-attrdlg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("claude.exe");
        if folder {
            std::fs::create_dir_all(&path).unwrap();
        } else {
            std::fs::write(&path, "x").unwrap();
        }
        path
    }

    fn attributes_dialog(path: &std::path::Path) -> Dialog {
        let mut setup = attr::Setup::new(vec![path.to_path_buf()]).unwrap();
        // the dates as a Russian Windows writes them
        setup.format = DMY;
        let times = attr::read(path).unwrap().times;
        setup.dates = std::array::from_fn(|i| times[i].map(|t| DMY.to_strings(t)).unwrap_or_default());
        Dialog::attributes(setup)
    }

    /// The screen of an 80 by 25 terminal: the dialog's box is at row 2.
    fn screen(dialog: &Dialog) -> Vec<String> {
        let mut grid = Grid::new(80, 25);
        dialog.draw(&mut grid);
        (0..25).map(|row| grid.row(row).iter().map(|cell| cell.text.clone()).collect()).collect()
    }

    fn at_col(line: &str, col: usize, len: usize) -> String {
        line.chars().skip(col).take(len).collect()
    }

    fn alt(dialog: &mut Dialog, c: char) -> Outcome {
        dialog.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT))
    }

    #[test]
    fn the_attributes_dialog_is_laid_out_cell_for_cell_as_far_lays_it_out() {
        let path = scratch("layout", false);
        let dialog = attributes_dialog(&path);
        let rows = screen(&dialog);
        // dialog row y is screen row y + 2; columns are the box's own
        let row = |y: usize| rows[y + 2].clone();
        assert_eq!(at_col(&row(1), 3, 1), "\u{2554}");
        assert_eq!(at_col(&row(1), 34, 12), " Attributes ");
        assert_eq!(at_col(&row(2), 27, 26), "Change file attributes for");
        assert_eq!(at_col(&row(3), 35, 10), "claude.exe");
        assert_eq!(at_col(&row(4), 3, 1), "\u{255F}");
        assert_eq!(at_col(&row(4), 30, 1), "\u{252C}");
        assert_eq!(at_col(&row(4), 76, 1), "\u{2562}");
        assert_eq!(at_col(&row(5), 5, 13), "[ ] Read only");
        assert_eq!(at_col(&row(14), 5, 11), "[ ] Offline");
        assert_eq!(at_col(&row(16), 5, 8), "[ More ]");
        assert_eq!(at_col(&row(6), 32, 10), "Date/Time:");
        assert_eq!(at_col(&row(6), 47, 11), "DD.MM.YYYYY");
        assert_eq!(at_col(&row(6), 59, 8), "hh:mm:ss");
        assert_eq!(at_col(&row(7), 32, 6), "write:");
        assert_eq!(at_col(&row(8), 32, 9), "creation:");
        assert_eq!(at_col(&row(9), 32, 7), "access:");
        assert_eq!(at_col(&row(10), 32, 7), "change:");
        assert_eq!(at_col(&row(7), 49, 1), ".");
        assert_eq!(at_col(&row(7), 61, 1), ":");
        assert_eq!(at_col(&row(7), 67, 1), ",");
        assert_eq!(at_col(&row(12), 36, 34), "[ Original ] [ Current ] [ Blank ]");
        assert_eq!(at_col(&row(14), 30, 1), "\u{251C}");
        assert_eq!(at_col(&row(14), 31, 1), "\u{2500}");
        assert_eq!(at_col(&row(14), 76, 1), "\u{2562}");
        assert_eq!(at_col(&row(15), 32, 6), "Owner:");
        assert_eq!(at_col(&row(13), 30, 1), "\u{2502}");
        assert_eq!(at_col(&row(17), 30, 1), "\u{2534}");
        assert_eq!(at_col(&row(17), 3, 1), "\u{255F}");
        assert_eq!(at_col(&row(18), 20, 40), "{ Set } [ System properties ] [ Cancel ]");
        assert_eq!(at_col(&row(19), 3, 1), "\u{255A}");
        assert!(!at_col(&row(16), 32, 43).trim().is_empty(), "the owner: {:?}", row(16));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_folder_gets_process_subfolders_and_it_leaves_everything_as_it_is() {
        let path = scratch("folder", true);
        let mut dialog = attributes_dialog(&path);
        // two rows taller (23): the box starts a row higher
        let rows = screen(&dialog);
        assert_eq!(at_col(&rows[1 + 17], 3, 1), "\u{255F}", "the separator before it");
        assert_eq!(at_col(&rows[1 + 17], 30, 1), "\u{2534}", "the line ends on it");
        assert_eq!(at_col(&rows[1 + 18], 5, 22), "[ ] Process subfolders");
        assert_eq!(at_col(&rows[1 + 19], 3, 1), "\u{255F}");
        assert_eq!(at_col(&rows[1 + 20], 20, 7), "{ Set }");
        assert_eq!(at_col(&rows[1 + 21], 3, 1), "\u{255A}");
        let written = dialog.values().text("wdate");
        alt(&mut dialog, 'f');
        let values = dialog.values();
        assert_eq!(values.choice("readonly"), 2, "[?]: leave as it is");
        assert!(!values.text("wdate").chars().any(|c| c.is_ascii_digit()), "the folder's times are blanked");
        assert!(values.text("owner").is_empty());
        assert_eq!(at_col(&screen(&dialog)[1 + 5], 5, 3), "[?]");
        alt(&mut dialog, 'f');
        let values = dialog.values();
        assert_eq!(values.choice("readonly"), 0);
        assert_eq!(values.text("wdate"), written, "back as it was");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn original_current_and_blank_fill_the_times_and_the_dialog_stays() {
        let path = scratch("times", false);
        let mut dialog = attributes_dialog(&path);
        let written = dialog.values().text("wdate");
        assert!(matches!(alt(&mut dialog, 'b'), Outcome::Open));
        let values = dialog.values();
        for (date, time) in TIME_IDS {
            assert_eq!(values.text(date), "  .  .     ");
            assert_eq!(values.text(time), "  :  :  ,       ");
        }
        // Alt+O is Offline's, as in Far (it comes first)
        alt(&mut dialog, 'o');
        assert_eq!(dialog.values().choice("offline"), 1);
        assert!(matches!(dialog.press("original"), Outcome::Open));
        assert_eq!(dialog.values().text("wdate"), written);
        alt(&mut dialog, 'n');
        let today = DMY.to_strings(SystemTime::now()).0;
        assert_eq!(dialog.values().text("cdate"), today);
        // the focus went to the write date: digits typed there replace it
        typed(&mut dialog, "01022020");
        assert_eq!(dialog.values().text("wdate"), "01.02.2020 ");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn compressed_and_encrypted_exclude_each_other_and_more_opens_over_the_dialog() {
        let path = scratch("more", false);
        let mut dialog = attributes_dialog(&path);
        alt(&mut dialog, 'c');
        assert_eq!(dialog.values().choice("compressed"), 1);
        alt(&mut dialog, 'e');
        assert_eq!(dialog.values().choice("encrypted"), 1);
        assert_eq!(dialog.values().choice("compressed"), 0);
        alt(&mut dialog, 'm');
        let rows = screen(&dialog);
        assert!(rows.iter().any(|row| row.contains("[ ] Integrity stream")), "{rows:#?}");
        alt(&mut dialog, 'p'); // Pinned
        press(&mut dialog, KeyCode::Enter);
        let Action::Attributes(setup) = &dialog.action else { panic!("the dialog's action") };
        assert_eq!(setup.current[14], 1, "Pinned, set in More, is kept");
        assert!(!screen(&dialog).iter().any(|row| row.contains("Integrity stream")), "More is closed");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_wipe_progress_box_is_fars_delete_progress() {
        let mut job = crate::fileops::start(Kind::Wipe, vec![], crate::fileops::Options::default());
        job.progress.from = "C:\\x\\big.iso".into();
        job.progress.file_size = 200;
        job.progress.file_done = 100;
        job.progress.files_done = 1234;
        let mut grid = Grid::new(80, 25);
        paint_progress(&job, &mut grid);
        let rows: Vec<String> = (0..25).map(|row| grid.row(row).iter().map(|cell| cell.text.clone()).collect()).collect();
        // 76 by 9, centered: the box at column 2, row 8
        let row = |y: usize| rows[8 + y].clone();
        assert_eq!(at_col(&row(1), 5, 1), "\u{2554}");
        assert_eq!(at_col(&row(1), 37, 6), " Wipe ", "centered on the double frame");
        assert_eq!(at_col(&row(2), 7, 25), "Wiping the file or folder");
        assert_eq!(at_col(&row(3), 7, 12), "C:\\x\\big.iso");
        let bar = at_col(&row(4), 7, 66);
        assert_eq!(bar.chars().filter(|&c| c == '\u{2588}').count(), 30, "Far: 50 * 61 / 100: {bar}");
        assert!(bar.ends_with("  50%"), "{bar}");
        assert_eq!(at_col(&row(5), 5, 1), "\u{255F}");
        assert_eq!(at_col(&row(6), 7, 61), format!("Files: {:>54}", "1 234"));
        assert_eq!(at_col(&row(7), 5, 1), "\u{255A}");
    }

    #[test]
    fn select_get_string_and_link_are_placed_as_far_places_them() {
        // Select: 55 by 7 on 80 by 25 at (12, 9): the frame from (15, 10), the mask from 17, its arrow at 62
        let rows = screen(&Dialog::select_group(true, "*.*"));
        assert_eq!(at_col(&rows[10], 15, 1), "\u{2554}");
        assert!(rows[10].contains(" Select "), "{}", rows[10]);
        assert_eq!(at_col(&rows[11], 17, 4), "*.* ");
        assert_eq!(at_col(&rows[11], 62, 1), "\u{2193}");
        assert!(rows[13].contains("{ OK }") && rows[13].contains("[ Filter ]") && rows[13].contains("[ Cancel ]"), "{}", rows[13]);
        assert!(screen(&Dialog::select_group(false, ""))[10].contains(" Deselect "));
        // GetString with buttons: 76 by 8 at (2, 8): the prompt at (7, 10), the line from 7, its arrow at 73
        let rows = screen(&Dialog::get_string("Apply command", "Enter command to process selected files", "", Action::Info));
        assert!(rows[9].contains(" Apply command "));
        assert_eq!(at_col(&rows[10], 7, 39), "Enter command to process selected files");
        assert_eq!(at_col(&rows[11], 73, 1), "\u{2193}");
        assert!(rows[13].contains("{ OK }") && rows[13].contains("[ Cancel ]"));
        // Link: 76 by 14 at (2, 5): the label at row 7, the type at 10 (its list at 31), multiple at 12, buttons at 16
        let link = Dialog::link("Create link to a.txt &in:".into(), "dest", true, vec![]);
        let rows = screen(&link);
        assert!(rows[6].contains(" Link "));
        assert_eq!(at_col(&rows[7], 7, 23), "Create link to a.txt in");
        assert_eq!(at_col(&rows[10], 7, 10), "Link type:");
        assert_eq!(at_col(&rows[10], 31, 9), "hard link");
        assert_eq!(at_col(&rows[12], 7, 33), "[ ] Process multiple destinations");
        assert!(rows[16].contains("{ Create link }") && rows[16].contains("[ Filter ]") && rows[16].contains("[ Cancel ]"), "{}", rows[16]);
    }
}
