//! The archive dialogs, as Far's archive plugin arclite lays them out
//! (plugins/arclite/src/ui.cpp and farutils.cpp of Far 3.0.6446): arclite
//! does not place its items by hand, its `Far::Dialog` puts them left to
//! right (`label`, `edit_box`, `check_box`...), `new_line` starts the next
//! row, and the box grows to what is in it. [`Layout`] is that, giving
//! Far's own coordinates to a `Layout::Placed` dialog.

use std::path::{Path, PathBuf};

use crate::archive::{self, Format};
use crate::dialog::{Action, Button, Dialog, Event, Item, Place};

/// arclite's c_x_frame and c_y_frame: where the client area starts.
const X_FRAME: u16 = 5;
const Y_FRAME: u16 = 2;

fn label_len(text: &str) -> u16 {
    // `&&` is one `&`, a lone `&` marks the hot key
    let mut len = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                len += 1;
            }
            continue;
        }
        len += 1;
    }
    len
}

/// arclite's Far::Dialog layout (farutils.cpp).
struct Layout {
    items: Vec<Item>,
    places: Vec<Place>,
    x: u16,
    y: u16,
    client_xs: u16,
}

impl Layout {
    fn new(title: &str, width: u16) -> Self {
        Self { items: Vec::new(), places: Vec::new(), x: X_FRAME, y: Y_FRAME, client_xs: width.max(title.chars().count() as u16) }
    }

    fn grow(&mut self) {
        if self.x - X_FRAME > self.client_xs {
            self.client_xs = self.x - X_FRAME;
        }
    }

    fn push(&mut self, item: Item, place: Place) -> usize {
        self.items.push(item);
        self.places.push(place);
        self.items.len() - 1
    }

    fn new_line(&mut self) {
        self.x = X_FRAME;
        self.y += 1;
    }

    fn reset_line(&mut self) {
        self.x = X_FRAME;
    }

    fn spacer(&mut self, size: u16) {
        self.x += size;
        self.grow();
    }

    fn separator(&mut self) -> usize {
        let y = self.y;
        self.push(Item::Separator(None), Place { x: -1, y, x2: 0, y2: 0 })
    }

    /// A plain label of `size` cells (arclite's label with a box size, DIF_SHOWAMPERSAND).
    fn text_box(&mut self, text: &str, size: u16) -> usize {
        let (x, y) = (self.x, self.y);
        self.x += size;
        self.grow();
        self.push(Item::Text { text: text.into(), align: crate::dialog::Align::Left, highlight: false }, Place { x: x as i16, y, x2: self.x - 1, y2: 0 })
    }

    fn pad(&mut self, pos: u16) {
        if pos > self.x - X_FRAME {
            self.spacer(pos - (self.x - X_FRAME));
        }
    }

    fn label(&mut self, text: &'static str) -> usize {
        let (x, y) = (self.x, self.y);
        self.x += label_len(text);
        self.grow();
        let item = if text.contains('&') { Item::Label(text.into()) } else { Item::Text { text: text.into(), align: crate::dialog::Align::Left, highlight: false } };
        self.push(item, Place { x: x as i16, y, x2: self.x - 1, y2: 0 })
    }

    /// An input line `size` wide, or (None) to the right edge as it is now.
    fn edit_box(&mut self, id: &'static str, text: &str, size: Option<u16>, history: bool, secret: bool) -> usize {
        let (x, y) = (self.x, self.y);
        self.x = match size {
            Some(size) => self.x + size,
            None => X_FRAME + self.client_xs,
        };
        self.grow();
        let x2 = self.x - 1 - u16::from(history);
        let text: Vec<char> = text.chars().collect();
        let cursor = text.len();
        let unchanged = !text.is_empty();
        self.push(Item::Edit { id, label: None, x, text, cursor, unchanged, enabled: true, history, secret }, Place { x: x as i16, y, x2, y2: 0 })
    }

    fn check_box(&mut self, id: &'static str, label: &'static str, state: u8, three: bool) -> usize {
        let (x, y) = (self.x, self.y);
        self.x += label_len(label) + 4;
        self.grow();
        self.push(Item::Check3 { id, label, state, three, enabled: true }, Place { x: x as i16, y, x2: 0, y2: 0 })
    }

    fn radio(&mut self, group: &'static str, index: usize, label: &'static str, on: bool) -> usize {
        let (x, y) = (self.x, self.y);
        self.x += label_len(label) + 4;
        self.grow();
        self.push(Item::Option { id: group, index, label, on }, Place { x: x as i16, y, x2: 0, y2: 0 })
    }

    /// A drop-down list: as wide as its longest item and the arrow (AUTO_SIZE), or `size`.
    fn combo(&mut self, id: &'static str, options: Vec<&'static str>, selected: usize, size: Option<u16>) -> usize {
        let (x, y) = (self.x, self.y);
        let longest = options.iter().map(|o| o.chars().count() as u16).max().unwrap_or(1).max(1);
        self.x += size.unwrap_or(longest + 5);
        self.grow();
        let x2 = self.x - 2;
        self.push(Item::Combo { id, label: "", x, options, selected, unchanged: false }, Place { x: x as i16, y, x2, y2: 0 })
    }

    /// A button of its own where the row is (not centered).
    fn button(&mut self, label: &'static str, id: &'static str, stay: bool) -> usize {
        let (x, y) = (self.x, self.y);
        let width = label_len(label) + 4;
        self.x += width;
        self.grow();
        let button = Button { label, id, default: false, enabled: true, stay };
        self.push(Item::Buttons(vec![button]), Place { x: x as i16, y, x2: x + width - 1, y2: 0 })
    }

    /// A button that cannot be pressed (farsh keeps no profiles or settings).
    fn button_off(&mut self, label: &'static str, id: &'static str) -> usize {
        let index = self.button(label, id, true);
        if let Item::Buttons(buttons) = &mut self.items[index] {
            buttons[0].enabled = false;
        }
        index
    }

    /// The DIF_CENTERGROUP buttons of the last row, centered as Far centers them.
    fn center_buttons(&mut self, buttons: Vec<Button>) -> usize {
        let y = self.y;
        for b in &buttons {
            self.x += label_len(b.label) + 4;
        }
        self.grow();
        self.push(Item::Buttons(buttons), Place { x: -1, y, x2: 0, y2: 0 })
    }

    /// arclite's show(): the box is the client area and the frames around it.
    fn finish(self, title: &str, focus: usize, hook: Option<fn(&mut Dialog, Event)>, action: Action) -> Dialog {
        let size = (self.client_xs + 2 * X_FRAME, self.y - Y_FRAME + 2 * Y_FRAME);
        Dialog::placed(title.into(), size, self.items, self.places, focus, hook, action)
    }
}

// ---------------------------------------------------------------- Extract files

/// What Extract files works on and where its controls are (arclite's ExtractOptions).
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractSetup {
    pub pick: crate::panels::ArchivePick,
    move_files: usize,
    delete_archive: usize,
    updatable: bool,
}

/// arclite's ExtractDialog (c_client_xs 60): F5/F6 from an archive's panel.
/// "Delete files from archive" is [?] and off for an archive that cannot be
/// changed, else on for F6; "Go to target directory" is for Shift+F2 only (off).
pub fn extract(pick: crate::panels::ArchivePick, dest: &Path, moving: bool) -> Dialog {
    let updatable = archive::updatable(&pick.view.listing);
    let mut l = Layout::new("Extract files", 60);
    l.label("&Extract files to:");
    l.new_line();
    let mut dst = dest.display().to_string();
    if !dst.ends_with(std::path::MAIN_SEPARATOR) {
        dst.push(std::path::MAIN_SEPARATOR);
    }
    let dst_id = l.edit_box("dest", &dst, Some(60), true, false);
    l.new_line();
    l.separator();
    l.new_line();
    l.check_box("ignoreerrors", "&Ignore errors", 0, false);
    l.new_line();
    l.label("When destination file exists:");
    l.new_line();
    l.spacer(2);
    l.radio("overwrite", 0, "As&k", true);
    l.spacer(2);
    l.radio("overwrite", 1, "&Overwrite", false);
    l.spacer(2);
    l.radio("overwrite", 2, "&Skip", false);
    l.new_line();
    l.spacer(2);
    l.radio("overwrite", 3, "&Rename", false);
    l.spacer(2);
    l.radio("overwrite", 4, "&Append", false);
    l.new_line();
    let move_state = if updatable { u8::from(moving) } else { 2 };
    let move_files = l.check_box("movefiles", "&Delete files from archive after extraction", move_state, !updatable);
    l.new_line();
    let delete_archive = l.check_box("deletearchive", "De&lete archive after extraction", 0, false);
    l.new_line();
    l.check_box("separatedir", "Ex&tract archive into separate directory", 2, true);
    l.new_line();
    let open_dir = l.check_box("opendir", "&Go to target directory after extraction", 2, true);
    l.new_line();
    l.label("Pass&word:");
    l.edit_box("password", pick.view.password.as_deref().unwrap_or(""), Some(20), false, true);
    l.new_line();
    l.separator();
    l.new_line();
    l.center_buttons(vec![
        Button { label: "OK", id: "ok", default: true, enabled: true, stay: false },
        Button { label: "Cancel", id: "cancel", default: false, enabled: true, stay: false },
        Button { label: "Sa&ve settings", id: "saveparams", default: false, enabled: false, stay: true },
    ]);
    l.new_line();
    let setup = ExtractSetup { pick, move_files, delete_archive, updatable };
    let mut dialog = l.finish("Extract files", dst_id, Some(extract_hook), Action::Extract(Box::new(setup)));
    // triUndef: an archive farsh cannot change keeps its files; the folder to go to is Shift+F2's
    dialog.enable(move_files, updatable);
    dialog.enable(open_dir, false);
    dialog
}

/// ExtractDialog's dialog_proc: "Delete files from archive" only for an
/// archive that can be changed and stays (`move_files != triUndef && !delete_archive`).
fn extract_hook(dialog: &mut Dialog, event: Event) {
    let Action::Extract(setup) = dialog.action_mut() else { return };
    let (move_files, delete_archive, updatable) = (setup.move_files, setup.delete_archive, setup.updatable);
    if event == Event::Toggled("deletearchive") {
        let deleting = dialog.check_state(delete_archive) == 1;
        dialog.enable(move_files, updatable && !deleting);
    }
}

// ---------------------------------------------------------------- Enter directory name

/// F7 in a zip: arclite's create_dir asks with Far's InputBox (GetString in
/// stddlg.cpp, no buttons): 76 by 6, the prompt at (5, 2), the line from 5
/// to 70 on row 3; Enter is OK.
pub fn create_folder(view: crate::panels::ArchiveView) -> Dialog {
    let items = vec![
        Item::Text { text: "Enter directory name:".into(), align: crate::dialog::Align::Left, highlight: false },
        Item::Edit { id: "name", label: None, x: 5, text: vec![], cursor: 0, unchanged: false, enabled: true, history: false, secret: false },
    ];
    let places = vec![Place { x: 5, y: 2, x2: 0, y2: 0 }, Place { x: 5, y: 3, x2: 70, y2: 0 }];
    Dialog::placed("ArcLite".into(), (76, 6), items, places, 1, None, Action::CreateDirInArchive(Box::new(view)))
}

// ---------------------------------------------------------------- Overwrite file

/// arclite's OverwriteDialog (c_client_xs 60, FDLG_WARNING) for extracting:
/// the file, the question, source and destination with size and time (the
/// newer one marked), Remember choice, Overwrite/Skip/Rename/Append/Cancel.
pub fn overwrite(path: &Path, src: (u64, Option<std::time::SystemTime>), dst: (u64, Option<std::time::SystemTime>), update: bool) -> Dialog {
    let mut l = Layout::new("Overwrite file", 60);
    let shown = fit_str(&path.display().to_string(), 60);
    l.text_box(&shown, 60);
    l.new_line();
    l.label("Destination file already exists. Overwrite?");
    l.new_line();
    l.separator();
    l.new_line();
    let (src_label, dst_label) = ("Source:", "Destination:");
    let label_pad = src_label.len().max(dst_label.len()) as u16 + 1;
    let (src_size, dst_size) = (src.0.to_string(), dst.0.to_string());
    let size_pad = label_pad + src_size.len().max(dst_size.len()) as u16 + 1;
    let newer = |a: Option<std::time::SystemTime>, b: Option<std::time::SystemTime>| matches!((a, b), (Some(a), Some(b)) if a > b);
    for (label, (size, time), other) in [(src_label, (&src_size, src.1), dst.1), (dst_label, (&dst_size, dst.1), src.1)] {
        l.text_box(label, label.len() as u16);
        l.pad(label_pad);
        l.text_box(size, size.len() as u16);
        l.pad(size_pad);
        let when = file_time(time);
        l.text_box(&when, when.chars().count() as u16);
        if newer(time, other) {
            l.spacer(1);
            l.label("(newer)");
        }
        l.new_line();
    }
    l.separator();
    l.new_line();
    l.check_box("remember", "&Remember choice", 0, false);
    l.new_line();
    l.separator();
    l.new_line();
    let button = |label, id, default| Button { label, id, default, enabled: true, stay: false };
    // odkExtract has Rename and Append; odkUpdate (adding to an archive) has not
    let mut buttons = vec![button("&Overwrite", "overwrite", true), button("&Skip", "skip", false)];
    if !update {
        buttons.push(button("R&ename", "rename", false));
        buttons.push(button("A&ppend", "append", false));
    }
    buttons.push(button("&Cancel", "cancel", false));
    // the spacer(1) arclite puts between them is in the group's own spacing
    let focus = l.center_buttons(buttons);
    l.new_line();
    let mut dialog = l.finish("Overwrite file", focus, None, Action::Conflict);
    dialog.set_warning();
    dialog
}

/// arclite's fit_str: the middle replaced with `...`.
fn fit_str(text: &str, size: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= size {
        return text.to_string();
    }
    if size <= 3 {
        return chars[..size].iter().collect();
    }
    let keep = size - 3;
    let mut out: String = chars[..keep / 2].iter().collect();
    out.push_str("...");
    out.extend(&chars[chars.len() - (keep - keep / 2)..]);
    out
}

/// arclite's format_file_time: the locale's short date and time.
fn file_time(time: Option<std::time::SystemTime>) -> String {
    let Some(time) = time else { return String::new() };
    let format = crate::attributes::Format::system();
    let (date, clock) = format.to_strings(time);
    let clock: String = clock.chars().take(8).collect();
    format!("{} {clock}", date.trim())
}

// ---------------------------------------------------------------- Create archive

/// arclite's c_levels: the names and 7-Zip's levels.
pub const LEVELS: [(&str, u32); 6] = [("Store", 0), ("Fastest", 1), ("Fast", 3), ("Normal", 5), ("Maximum", 7), ("Ultra", 9)];
/// arclite's c_methods (7z.dll's own codecs are not offered: there is no 7z.dll here).
pub const METHODS: [&str; 5] = ["LZMA", "LZMA2", "PPMD", "Deflate", "Deflate64"];

/// Where each control of Create archive is (arclite's *_ctrl_id), and what
/// the dialog procedure keeps between events.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateSetup {
    pub sources: Vec<PathBuf>,
    /// The folder the archive path is relative to (the active panel's).
    pub base: PathBuf,
    /// The name the archive gets when the path is a folder.
    pub default_name: String,
    pub multifile: bool,
    pub other: Vec<Format>,
    pub format: Format,
    old_ext: String,
    ids: CreateIds,
    /// What OK asked for, once its checks passed.
    pub options: Option<archive::Options>,
    /// Add files to archive: the archive (shown in the other panel) and
    /// the folder inside it; `None` creates a new one.
    pub target: Option<crate::panels::ArchiveView>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CreateIds {
    overwrite: usize,
    profile: usize,
    delete_profile: usize,
    arc_path: usize,
    append_ext: usize,
    first_type: usize,
    other_type: usize,
    level: usize,
    method: usize,
    solid: usize,
    advanced: usize,
    encrypt: usize,
    encrypt_header: usize,
    show_password: usize,
    password: usize,
    verify: usize,
    visible: usize,
    create_sfx: usize,
    sfx_options: usize,
    enable_volumes: usize,
    volume_size: usize,
    move_files: usize,
    enable_filter: usize,
}

/// Shift+F1: UpdateDialog::show with new_arc (c_client_xs 68).
pub fn create_archive(sources: Vec<PathBuf>, base: &Path) -> Dialog {
    update_dialog(sources, base, None, false)
}

/// F5/F6 into an archive's panel: UpdateDialog without new_arc ("Add files
/// to archive": the archive's own format, "When destination file exists").
pub fn add_to_archive(sources: Vec<PathBuf>, base: &Path, target: crate::panels::ArchiveView, moving: bool) -> Dialog {
    update_dialog(sources, base, Some(target), moving)
}

fn update_dialog(sources: Vec<PathBuf>, base: &Path, target: Option<crate::panels::ArchiveView>, moving: bool) -> Dialog {
    let new_arc = target.is_none();
    let multifile = sources.len() > 1 || sources.first().is_some_and(|s| s.is_dir());
    let default_name = match sources.as_slice() {
        [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        _ => base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into()),
    };
    let format = if new_arc { Format::SevenZ } else { Format::Zip };
    let old_ext = format.extension().to_string();
    // the other updatable formats: one file may also go into a single-file format
    let other: Vec<Format> = [Format::Gzip, Format::Tar].into_iter().filter(|f| !multifile || !f.single_file()).collect();
    let title = if new_arc { "Create archive" } else { "Add files to archive" };
    let mut l = Layout::new(title, 68);
    let mut ids = CreateIds::default();

    if new_arc {
    l.label("&Profile:");
    ids.profile = l.combo("profile", vec![""], 0, Some(30));
    l.spacer(1);
    l.button_off("Save", "saveprofile");
    l.spacer(1);
    ids.delete_profile = l.button("Delete", "deleteprofile", true);
    l.new_line();
    l.separator();
    l.new_line();

    let path_label = "Arc&hive path:";
    l.spacer(label_len(path_label) + 1);
    l.button("Eval&uate", "evaluate", true);
    l.spacer(1);
    ids.append_ext = l.check_box("appendext", "&Append extension", 0, false);
    l.reset_line();
    l.label(path_label);
    l.new_line();
    ids.arc_path = l.edit_box("arcpath", &format!("{default_name}{old_ext}"), Some(68), true, false);
    l.new_line();
    l.separator();
    l.new_line();

    l.label("Select archiver:");
    l.spacer(1);
    ids.first_type = l.radio("type", 0, "&7z", true);
    l.spacer(1);
    l.radio("type", 1, "&zip", false);
    if !other.is_empty() {
        l.spacer(1);
        ids.other_type = l.radio("type", 2, "O&ther:", false);
        l.combo("otherformat", other.iter().map(|f| f.name()).collect(), 0, None);
    }
    l.new_line();
    }

    l.label("Compression &level:");
    ids.level = l.combo("level", LEVELS.iter().map(|(name, _)| *name).collect(), 3, None);
    l.spacer(2);
    l.label("&Method:");
    // an archive keeps its method (a zip's is Deflate); a new 7z starts with LZMA
    ids.method = l.combo("method", METHODS.to_vec(), if new_arc { 0 } else { 3 }, None);
    l.spacer(2);
    ids.solid = l.check_box("solid", "S&olid", 1, false);
    l.new_line();

    l.label("Advan&ced parameters:");
    ids.advanced = l.edit_box("advanced", "", None, true, false);
    l.new_line();
    l.separator();
    l.new_line();

    ids.encrypt = l.check_box("encrypt", "&Encrypt archive", 0, false);
    l.spacer(2);
    ids.encrypt_header = l.check_box("encryptheader", "Encrypt header", 2, true);
    l.spacer(2);
    ids.show_password = l.check_box("showpassword", "Show password", 0, false);
    l.new_line();
    l.label("Pass&word:");
    ids.password = l.edit_box("password", "", Some(20), false, true);
    l.spacer(2);
    l.label("Repeat:");
    ids.verify = l.edit_box("verify", "", Some(20), false, true);
    l.reset_line();
    l.label("Pass&word:");
    ids.visible = l.edit_box("visible", "", Some(20), false, false);
    l.new_line();
    l.separator();
    l.new_line();

    if new_arc {
    ids.create_sfx = l.check_box("sfx", "Create self-e&xtracting archive", 0, false);
    l.spacer(2);
    ids.sfx_options = l.button("SFX options", "sfxoptions", true);
    l.new_line();
    l.separator();
    l.new_line();

    ids.enable_volumes = l.check_box("volumes", "Split into &volumes", 0, false);
    l.spacer(2);
    l.label("Volume size:");
    ids.volume_size = l.edit_box("volumesize", "", Some(20), true, false);
    l.new_line();
    l.separator();
    l.new_line();
    } else {
    l.label("When destination file exists:");
    l.new_line();
    l.spacer(2);
    ids.overwrite = l.radio("oa", 0, "As&k", true);
    l.spacer(2);
    l.radio("oa", 1, "O&verwrite", false);
    l.spacer(2);
    l.radio("oa", 2, "Ski&p", false);
    l.new_line();
    l.separator();
    l.new_line();
    }

    ids.move_files = l.check_box("movefiles", "&Delete files after archiving", u8::from(moving), false);
    l.spacer(2);
    l.check_box("ignoreerrors", "&Ignore errors", 0, false);
    l.new_line();
    l.check_box("openshared", "Archive files open for w&riting", 1, false);
    l.spacer(2);
    ids.enable_filter = l.check_box("filter", "Use &filter", 0, false);
    l.new_line();

    l.separator();
    l.new_line();
    l.center_buttons(vec![
        Button { label: "OK", id: "ok", default: true, enabled: true, stay: false },
        Button { label: "Cancel", id: "cancel", default: false, enabled: true, stay: false },
        Button { label: "Save settings", id: "saveparams", default: false, enabled: false, stay: true },
    ]);
    l.new_line();

    // arclite's DN_INITDIALOG focuses the path; with no path, the first control
    let focus = if new_arc { ids.arc_path } else { ids.level };
    let setup = CreateSetup { sources, base: base.to_path_buf(), default_name, multifile, other, format, old_ext, ids, options: None, target };
    let mut dialog = l.finish(title, focus, Some(create_hook), Action::CreateArchive(Box::new(setup)));
    set_control_state(&mut dialog);
    dialog
}

fn setup_of(dialog: &mut Dialog) -> Option<&mut CreateSetup> {
    match dialog.action_mut() {
        Action::CreateArchive(setup) => Some(setup),
        _ => None,
    }
}

/// The format the radio buttons and the "Other" list say.
fn chosen_format(dialog: &Dialog, setup: &CreateSetup) -> Format {
    let checked = |index: usize| dialog.check_state(index) == 1;
    if checked(setup.ids.first_type) {
        Format::SevenZ
    } else if checked(setup.ids.first_type + 1) {
        Format::Zip
    } else {
        setup.other.get(dialog.list_pos(setup.ids.other_type + 1)).copied().unwrap_or(Format::SevenZ)
    }
}

/// arclite's get_default_ext: the format's, `.001` for volumes.
fn default_ext(dialog: &Dialog, setup: &CreateSetup) -> String {
    let mut ext = setup.format.extension().to_string();
    if dialog.check_state(setup.ids.enable_volumes) == 1 {
        ext.push_str(".001");
    }
    ext
}

/// arclite's change_extension: a path ending in the old format's extension
/// gets the new one.
fn change_extension(dialog: &mut Dialog) {
    let Some(setup) = setup_of(dialog).cloned() else { return };
    let new_ext = default_ext(dialog, &setup);
    if setup.old_ext.is_empty() || new_ext.is_empty() {
        return;
    }
    let path = dialog.text_of(setup.ids.arc_path);
    let trimmed = path.trim().trim_matches('"').trim();
    let file_name = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed);
    let Some(mut pos) = file_name.rfind('.').filter(|&p| p > 0) else { return };
    let mut ext = &file_name[pos..];
    if ext.eq_ignore_ascii_case(".exe") || ext.eq_ignore_ascii_case(".001") {
        if let Some(before) = file_name[..pos].rfind('.').filter(|&p| p > 0) {
            pos = before;
            ext = &file_name[pos..];
        }
    }
    if !ext.eq_ignore_ascii_case(&setup.old_ext) {
        return;
    }
    let Some(at) = path.to_lowercase().rfind(&ext.to_lowercase()) else { return };
    let changed = format!("{}{new_ext}{}", &path[..at], &path[at + ext.len()..]);
    dialog.set_text_at(setup.ids.arc_path, &changed);
    if let Some(setup) = setup_of(dialog) {
        setup.old_ext = new_ext;
    }
}

/// arclite's set_control_state: what can be used with the format and the
/// check boxes as they are.
fn set_control_state(dialog: &mut Dialog) {
    let Some(setup) = setup_of(dialog).cloned() else { return };
    let ids = &setup.ids;
    let format = setup.format;
    let (is_7z, is_zip) = (format == Format::SevenZ, format == Format::Zip);
    // update_level_list: tar only stores
    if format == Format::Tar {
        dialog.set_list_pos(ids.level, 0);
    }
    let compressed = dialog.list_pos(ids.level) != 0;
    for i in ids.method - 1..=ids.method {
        dialog.enable(i, (is_7z || is_zip) && compressed);
    }
    dialog.enable(ids.level, format != Format::Tar);
    dialog.enable(ids.solid, is_7z && compressed);
    dialog.enable(ids.encrypt, is_7z || is_zip);
    let encrypt = dialog.check_state(ids.encrypt) == 1;
    for i in ids.encrypt + 1..=ids.visible {
        dialog.enable(i, encrypt && (is_7z || is_zip));
    }
    dialog.enable(ids.encrypt_header, is_7z && encrypt);
    let show_password = dialog.check_state(ids.show_password) == 1;
    for i in ids.password - 1..=ids.verify {
        dialog.show(i, !show_password);
    }
    for i in ids.visible - 1..=ids.visible {
        dialog.show(i, show_password);
    }
    if setup.target.is_some() {
        dialog.enable(ids.move_files, dialog.check_state(ids.enable_filter) != 1);
        return;
    }
    change_extension(dialog);
    // self-extracting archives need 7-Zip's SFX modules, which farsh has not got
    dialog.enable(ids.create_sfx, false);
    for i in ids.create_sfx + 1..=ids.sfx_options {
        dialog.enable(i, false);
    }
    let volumes = dialog.check_state(ids.enable_volumes) == 1;
    dialog.enable(ids.enable_volumes, true);
    for i in ids.enable_volumes + 1..=ids.volume_size {
        dialog.enable(i, volumes);
    }
    if ids.other_type != 0 {
        let other = dialog.check_state(ids.other_type) == 1;
        dialog.enable(ids.other_type + 1, other);
    }
    // no saved profiles: nothing to delete
    dialog.enable(ids.delete_profile, false);
    dialog.enable(ids.move_files, dialog.check_state(ids.enable_filter) != 1);
}

/// UpdateDialog's dialog_proc.
fn create_hook(dialog: &mut Dialog, event: Event) {
    let Some(setup) = setup_of(dialog).cloned() else { return };
    match event {
        Event::Toggled("type" | "otherformat") => {
            let previous = setup.format;
            let format = chosen_format(dialog, &setup);
            if let Some(s) = setup_of(dialog) {
                s.format = format;
            }
            if format != previous {
                // Deflate -> LZMA for 7z, LZMA -> Deflate for zip
                let method = dialog.list_pos(setup.ids.method);
                if method == 3 && format == Format::SevenZ {
                    dialog.set_list_pos(setup.ids.method, 0);
                } else if method == 0 && format == Format::Zip {
                    dialog.set_list_pos(setup.ids.method, 3);
                }
            }
            set_control_state(dialog);
        }
        Event::Toggled(_) => set_control_state(dialog),
        Event::Button("evaluate") => {
            let path = eval_arc_path(dialog, &setup).display().to_string();
            dialog.set_text_at(setup.ids.arc_path, &path);
        }
        Event::Closing("ok") => {
            let options = match archive_options(dialog, &setup) {
                Ok(options) => options,
                Err(message) => {
                    dialog.message_over("Error", &[&message], &[("OK", "ok")]);
                    return;
                }
            };
            let exists = setup.target.is_none() && options.path.exists();
            if let Some(s) = setup_of(dialog) {
                s.options = Some(options);
            }
            if exists {
                dialog.message_over("Create archive", &["Target file already exists. Do you want to overwrite it?"], &[("&Yes", "yes"), ("&No", "no")]);
            }
        }
        Event::SubClosed(Some(("yes", _))) => dialog.close_with("ok"),
        _ => {}
    }
}

/// arclite's eval_arc_path: the typed path, the default name for a folder,
/// the extension appended if asked, made absolute against the panel's folder.
pub fn eval_arc_path(dialog: &Dialog, setup: &CreateSetup) -> PathBuf {
    if let Some(target) = &setup.target {
        return target.file.clone();
    }
    let typed = dialog.text_of(setup.ids.arc_path);
    let mut path = typed.trim().replace('"', "");
    if path.is_empty() || path.ends_with(['\\', '/']) {
        path.push_str(&format!("{}{}", setup.default_name, default_ext(dialog, setup)));
    }
    if dialog.check_state(setup.ids.append_ext) == 1 {
        let ext = default_ext(dialog, setup);
        if !path.to_lowercase().ends_with(&ext.to_lowercase()) {
            path.push_str(&ext);
        }
    }
    setup.base.join(path)
}

/// arclite's read_controls with its checks: what to create, or what is wrong.
pub fn archive_options(dialog: &Dialog, setup: &CreateSetup) -> Result<archive::Options, String> {
    let ids = &setup.ids;
    let format = setup.format;
    let (is_7z, is_zip) = (format == Format::SevenZ, format == Format::Zip);
    let level = LEVELS[dialog.list_pos(ids.level)].1;
    let method = if level == 0 { "Copy" } else if is_7z || is_zip { METHODS[dialog.list_pos(ids.method)] } else { "" };
    let encrypt = (is_7z || is_zip) && dialog.check_state(ids.encrypt) == 1;
    let mut password = String::new();
    if encrypt {
        if dialog.check_state(ids.show_password) == 1 {
            password = dialog.text_of(ids.visible);
        } else {
            password = dialog.text_of(ids.password);
            if password != dialog.text_of(ids.verify) {
                return Err("Passwords don't match".into());
            }
        }
        if password.is_empty() {
            return Err("Password is empty".into());
        }
    }
    let volume_size = if setup.target.is_none() && dialog.check_state(ids.enable_volumes) == 1 {
        let size = archive::parse_size(&dialog.text_of(ids.volume_size)).filter(|&s| s >= archive::MIN_VOLUME_SIZE);
        Some(size.ok_or_else(|| "Invalid volume size".to_string())?)
    } else {
        None
    };
    Ok(archive::Options {
        path: eval_arc_path(dialog, setup),
        format,
        level,
        method: method.into(),
        solid: is_7z && level != 0 && dialog.check_state(ids.solid) == 1,
        password: encrypt.then_some(password),
        encrypt_header: is_7z && encrypt && dialog.check_state(ids.encrypt_header) != 0,
        volume_size,
        move_files: dialog.check_state(ids.enable_filter) != 1 && dialog.check_state(ids.move_files) == 1,
        ignore_errors: dialog.check_state(ids.move_files + 1) == 1,
        overwrite: match (setup.target.is_some(), dialog.check_state(ids.overwrite + 1), dialog.check_state(ids.overwrite + 2)) {
            (true, 1, _) => archive::Overwrite::Overwrite,
            (true, _, 1) => archive::Overwrite::Skip,
            _ => archive::Overwrite::Ask,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;

    fn screen(dialog: &Dialog, cols: u16, rows: u16) -> Vec<String> {
        let mut grid = Grid::new(cols, rows);
        dialog.paint(&mut grid);
        (0..rows).map(|row| grid.row(row).iter().map(|cell| cell.text.clone()).collect()).collect()
    }

    fn at(line: &str, col: usize, len: usize) -> String {
        line.chars().skip(col).take(len).collect()
    }

    /// The columns are those of the user's screenshot of Far's own Create archive.
    #[test]
    fn create_archive_is_laid_out_as_arclite_lays_it_out() {
        let dir = std::env::temp_dir().join(format!("farsh-arcdlg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("claude.exe"), "x").unwrap();
        let dialog = create_archive(vec![dir.join("claude.exe")], &dir);
        // 78 by 25 (client 68 by 21): on a 78 by 25 screen the box is the screen
        let rows = screen(&dialog, 78, 25);
        let row = |y: usize| rows[y].clone();
        assert_eq!(at(&row(1), 3, 1), "\u{2554}");
        assert_eq!(at(&row(1), 31, 16), " Create archive ");
        assert_eq!(at(&row(2), 5, 8), "Profile:");
        assert_eq!(at(&row(2), 42, 1), "\u{2193}");
        assert_eq!(at(&row(2), 44, 19), "[ Save ] [ Delete ]");
        assert_eq!(at(&row(3), 3, 1), "\u{255F}");
        assert_eq!(at(&row(4), 5, 13), "Archive path:");
        assert_eq!(at(&row(4), 19, 12), "[ Evaluate ]");
        assert_eq!(at(&row(4), 32, 20), "[ ] Append extension");
        assert_eq!(at(&row(5), 5, 13), "claude.exe.7z");
        assert_eq!(at(&row(5), 72, 1), "\u{2193}");
        assert_eq!(at(&row(7), 5, 16), "Select archiver:");
        assert_eq!(at(&row(7), 22, 25), "(\u{2022}) 7z ( ) zip ( ) Other:");
        assert_eq!(at(&row(7), 47, 4), "gzip");
        assert_eq!(at(&row(8), 5, 18), "Compression level:");
        assert_eq!(at(&row(8), 23, 6), "Normal");
        assert_eq!(at(&row(8), 37, 7), "Method:");
        assert_eq!(at(&row(8), 44, 4), "LZMA");
        assert_eq!(at(&row(8), 60, 9), "[x] Solid");
        assert_eq!(at(&row(9), 5, 20), "Advanced parameters:");
        assert_eq!(at(&row(11), 5, 19), "[ ] Encrypt archive");
        assert_eq!(at(&row(11), 26, 18), "[?] Encrypt header");
        assert_eq!(at(&row(11), 46, 17), "[ ] Show password");
        assert_eq!(at(&row(12), 5, 9), "Password:");
        assert_eq!(at(&row(12), 36, 7), "Repeat:");
        assert_eq!(at(&row(14), 5, 34), "[ ] Create self-extracting archive");
        assert_eq!(at(&row(16), 5, 22), "[ ] Split into volumes");
        assert_eq!(at(&row(16), 29, 12), "Volume size:");
        assert_eq!(at(&row(16), 60, 1), "\u{2193}");
        assert_eq!(at(&row(18), 5, 32), "[ ] Delete files after archiving");
        assert_eq!(at(&row(18), 39, 17), "[ ] Ignore errors");
        assert_eq!(at(&row(19), 5, 34), "[x] Archive files open for writing");
        assert_eq!(at(&row(19), 41, 14), "[ ] Use filter");
        assert_eq!(at(&row(21), 21, 35), "{ OK } [ Cancel ] [ Save settings ]");
        assert_eq!(at(&row(22), 3, 1), "\u{255A}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn zip_changes_the_extension_and_the_method_and_encryption_opens_the_password() {
        let dir = std::env::temp_dir().join(format!("farsh-arcdlg2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let mut dialog = create_archive(vec![dir.join("a.txt")], &dir);
        let alt = |dialog: &mut Dialog, c: char| dialog.key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::ALT));
        alt(&mut dialog, 'z');
        let rows = screen(&dialog, 78, 25);
        assert_eq!(at(&rows[5], 5, 9), "a.txt.zip");
        assert_eq!(at(&rows[8], 44, 7), "Deflate", "LZMA -> Deflate for zip");
        alt(&mut dialog, 'e');
        alt(&mut dialog, 'w');
        for c in "pw".chars() {
            dialog.key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE));
        }
        // OK with a password typed once: arclite's check
        assert!(matches!(dialog.key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Enter, crossterm::event::KeyModifiers::NONE)), crate::dialog::Outcome::Open));
        assert!(screen(&dialog, 78, 25).iter().any(|row| row.contains("Passwords don't match")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn extract_files_and_overwrite_file_are_laid_out_as_arclite_lays_them_out() {
        let listing = crate::archive::Listing { chain: vec![Format::Zip], items: vec![] };
        let view = crate::panels::ArchiveView { file: PathBuf::from("a.zip"), listing, dir: String::new(), password: None };
        let pick = crate::panels::ArchivePick { view, items: vec!["x".into()] };
        let dest = std::env::temp_dir();
        let dialog = extract(pick, &dest, false);
        // 70 by 18 (client 60 by 14)
        let rows = screen(&dialog, 70, 18);
        let row = |y: usize| rows[y].clone();
        assert_eq!(at(&row(1), 27, 15), " Extract files ");
        assert_eq!(at(&row(2), 5, 17), "Extract files to:");
        assert_eq!(at(&row(3), 64, 1), "\u{2193}");
        assert_eq!(at(&row(5), 5, 17), "[ ] Ignore errors");
        assert_eq!(at(&row(6), 5, 29), "When destination file exists:");
        assert_eq!(at(&row(7), 7, 7), "(\u{2022}) Ask");
        assert_eq!(at(&row(7), 16, 13), "( ) Overwrite");
        assert_eq!(at(&row(7), 31, 8), "( ) Skip");
        assert_eq!(at(&row(8), 7, 10), "( ) Rename");
        assert_eq!(at(&row(8), 19, 10), "( ) Append");
        assert_eq!(at(&row(9), 5, 46), "[ ] Delete files from archive after extraction", "a zip can lose files: F5 leaves it off");
        assert_eq!(at(&row(10), 5, 35), "[ ] Delete archive after extraction");
        assert_eq!(at(&row(11), 5, 43), "[?] Extract archive into separate directory");
        assert_eq!(at(&row(12), 5, 43), "[?] Go to target directory after extraction");
        assert_eq!(at(&row(13), 5, 9), "Password:");
        assert_eq!(at(&row(15), 17, 35), "{ OK } [ Cancel ] [ Save settings ]");
        assert_eq!(at(&row(16), 3, 1), "\u{255A}");

        let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000);
        let older = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let dialog = overwrite(Path::new("C:/x/a.txt"), (17, Some(when)), (3, Some(older)), false);
        let rows = screen(&dialog, 70, 14);
        assert_eq!(at(&rows[1], 27, 16), " Overwrite file ");
        assert_eq!(at(&rows[2], 5, 10), "C:/x/a.txt");
        assert_eq!(at(&rows[3], 5, 43), "Destination file already exists. Overwrite?");
        assert_eq!(at(&rows[5], 5, 7), "Source:");
        assert_eq!(at(&rows[5], 18, 2), "17");
        assert!(rows[5].contains("(newer)") && !rows[6].contains("(newer)"));
        assert_eq!(at(&rows[6], 5, 12), "Destination:");
        assert_eq!(at(&rows[6], 18, 1), "3");
        assert_eq!(at(&rows[8], 5, 19), "[ ] Remember choice");
        assert!(rows[10].contains("{ Overwrite } [ Skip ] [ Rename ] [ Append ] [ Cancel ]"), "{:?}", rows[10]);
    }

    #[test]
    fn add_files_to_archive_is_update_dialog_without_the_new_archive_part() {
        let listing = crate::archive::Listing { chain: vec![Format::Zip], items: vec![] };
        let view = crate::panels::ArchiveView { file: PathBuf::from("a.zip"), listing, dir: String::new(), password: None };
        let dialog = add_to_archive(vec![PathBuf::from("x.txt")], Path::new("."), view, true);
        // 78 wide; rows: level, advanced, sep, encrypt, password, sep, oa (3), sep, move/ignore, shared/filter, sep, buttons
        let rows = screen(&dialog, 78, 25);
        let top = rows.iter().position(|r| r.contains(" Add files to archive ")).unwrap();
        let row = |y: usize| rows[top + y - 1].clone();
        assert_eq!(at(&row(2), 5, 18), "Compression level:");
        assert_eq!(at(&row(2), 44, 7), "Deflate", "a zip's method");
        assert_eq!(at(&row(3), 5, 20), "Advanced parameters:");
        assert_eq!(at(&row(5), 5, 19), "[ ] Encrypt archive");
        assert_eq!(at(&row(8), 5, 29), "When destination file exists:");
        assert_eq!(at(&row(9), 7, 7), "(\u{2022}) Ask");
        assert_eq!(at(&row(9), 16, 13), "( ) Overwrite");
        assert_eq!(at(&row(9), 31, 8), "( ) Skip");
        assert_eq!(at(&row(11), 5, 32), "[x] Delete files after archiving", "F6 moves");
        assert!(!rows.iter().any(|r| r.contains("Profile") || r.contains("Select archiver") || r.contains("volumes")));
    }

    #[test]
    fn enter_directory_name_is_fars_input_box_and_enter_accepts_it() {
        let listing = crate::archive::Listing { chain: vec![Format::Zip], items: vec![] };
        let view = crate::panels::ArchiveView { file: PathBuf::from("a.zip"), listing, dir: String::new(), password: None };
        let mut dialog = create_folder(view);
        let rows = screen(&dialog, 76, 6);
        assert_eq!(at(&rows[1], 3, 1), "\u{2554}");
        assert_eq!(at(&rows[1], 33, 9), " ArcLite ");
        assert_eq!(at(&rows[2], 5, 21), "Enter directory name:");
        assert_eq!(at(&rows[4], 3, 1), "\u{255A}");
        for c in "docs".chars() {
            dialog.key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE));
        }
        let crate::dialog::Outcome::Pressed(Action::CreateDirInArchive(_), "ok", values) =
            dialog.key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Enter, crossterm::event::KeyModifiers::NONE))
        else {
            panic!("Enter accepts")
        };
        assert_eq!(values.text("name"), "docs");
    }
}
