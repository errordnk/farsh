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
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CreateIds {
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
    let multifile = sources.len() > 1 || sources.first().is_some_and(|s| s.is_dir());
    let default_name = match sources.as_slice() {
        [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        _ => base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into()),
    };
    let format = Format::SevenZ;
    let old_ext = format.extension().to_string();
    // the other updatable formats: one file may also go into a single-file format
    let other: Vec<Format> = [Format::Gzip, Format::Tar].into_iter().filter(|f| !multifile || !f.single_file()).collect();
    let mut l = Layout::new("Create archive", 68);
    let mut ids = CreateIds::default();

    l.label("&Profile:");
    ids.profile = l.combo("profile", vec![""], 0, Some(30));
    l.spacer(1);
    l.button("Save", "saveprofile", true);
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

    l.label("Compression &level:");
    ids.level = l.combo("level", LEVELS.iter().map(|(name, _)| *name).collect(), 3, None);
    l.spacer(2);
    l.label("&Method:");
    ids.method = l.combo("method", METHODS.to_vec(), 0, None);
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

    ids.move_files = l.check_box("movefiles", "&Delete files after archiving", 0, false);
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
        Button { label: "Save settings", id: "saveparams", default: false, enabled: true, stay: true },
    ]);
    l.new_line();

    let focus = ids.arc_path;
    let setup = CreateSetup { sources, base: base.to_path_buf(), default_name, multifile, other, format, old_ext, ids, options: None };
    let mut dialog = l.finish("Create archive", focus, Some(create_hook), Action::CreateArchive(Box::new(setup)));
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
        Event::Button("saveparams" | "saveprofile") => {
            dialog.message_over("Create archive", &["Settings saved"], &[("OK", "ok")]);
        }
        Event::Closing("ok") => {
            let options = match archive_options(dialog, &setup) {
                Ok(options) => options,
                Err(message) => {
                    dialog.message_over("Error", &[&message], &[("OK", "ok")]);
                    return;
                }
            };
            let exists = options.path.exists();
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
    let volume_size = if dialog.check_state(ids.enable_volumes) == 1 {
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
}
