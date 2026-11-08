//! farsh's settings (Far's configuration): `~/.config/farsh/settings.json`
//! on every system. A setting has Far's own name (`Panel.ShowHidden`,
//! `Panel.Layout.ColumnTitles`...) and Far's default; the file keeps the
//! ones that were changed, nested by the dots. A change is written at once.

use std::sync::RwLock;

use serde_json::{Map, Value};

/// A setting's default.
#[derive(Clone, Copy)]
pub enum Def {
    Bool(bool),
    /// Far's 3-state: 0 false, 1 true, 2 other.
    Three(i64),
    Int(i64),
    Str(&'static str),
}

use Def::{Bool, Int, Str, Three};

/// Far's defaults (config.cpp's InitConfigsData) of what farsh shows.
const DEFAULTS: &[(&str, Def)] = &[
    // System settings
    ("System.DeleteToRecycleBin", Bool(true)),
    ("System.UseSystemCopy", Bool(true)),
    ("System.CopyOpened", Bool(true)),
    ("System.ScanJunction", Bool(true)),
    ("System.SaveHistory", Three(1)),
    ("System.SaveFoldersHistory", Three(1)),
    ("System.SaveViewHistory", Three(1)),
    ("System.UseRegisteredTypes", Bool(true)),
    ("System.UpdateEnvironment", Bool(false)),
    ("System.ElevationMode", Int(-1)),
    ("System.Sort.Collation", Int(2)),
    ("System.Sort.DigitsAsNumbers", Bool(true)),
    ("System.Sort.CaseSensitive", Bool(false)),
    ("System.AutoSaveSetup", Bool(false)),
    ("System.FolderInfo", Str("DirInfo,File_Id.diz,Descript.ion,ReadMe.*,Read.Me")),
    ("System.Executor.UseHomeDir", Bool(true)),
    ("System.Executor.~", Str("%FARHOME%")),
    // Panel settings
    ("Panel.ShowHidden", Bool(true)),
    ("Panel.Highlight", Bool(true)),
    ("Panel.SelectFolders", Bool(false)),
    ("Panel.RightClickSelect", Bool(false)),
    ("Panel.SortFolderExt", Bool(false)),
    ("Panel.ReverseSort", Bool(true)),
    ("Panel.AutoUpdateLimit", Int(0)),
    ("System.AutoUpdateRemoteDrive", Bool(true)),
    ("Panel.Layout.ColumnTitles", Bool(true)),
    ("Panel.Layout.StatusLine", Bool(true)),
    ("Panel.Layout.DetailedJunction", Bool(false)),
    ("Panel.Layout.TotalInfo", Bool(true)),
    ("Panel.Layout.FreeInfo", Bool(false)),
    ("Panel.Layout.Scrollbar", Bool(false)),
    ("Panel.Layout.ScreensNumber", Three(1)),
    ("Panel.Layout.SortMode", Bool(true)),
    ("Interface.ShowDotsInRoot", Bool(false)),
    // Interface settings
    ("Screen.Clock", Bool(true)),
    ("Interface.Mouse", Bool(true)),
    ("Screen.KeyBar", Bool(true)),
    ("Viewer.ShowKeyBar", Bool(true)),
    ("Interface.ShowMenuBar", Bool(false)),
    ("Screen.ScreenSaver", Bool(false)),
    ("Screen.ScreenSaverTime", Int(5)),
    ("Interface.CopyShowTotal", Bool(true)),
    ("System.CopyTimeRule", Int(3)),
    ("Interface.DelShowTotal", Bool(false)),
    ("Interface.CtrlPgUp", Three(1)),
    ("Interface.VirtualTerminalRendering", Bool(false)),
    ("Interface.FullWidthAwareRendering", Three(0)),
    ("Interface.ClearType", Three(1)),
    ("Interface.SetIcon", Bool(false)),
    ("Interface.IconIndex", Int(0)),
    ("Interface.SetAdminIcon", Bool(true)),
    ("Interface.TitleAddons", Str("%Ver %Platform %Admin")),
    ("Interface.SetPalette", Bool(true)),
    // Languages
    ("Language.Main", Str("English")),
    ("Language.Help", Str("English")),
    // Dialog settings
    ("Dialog.EditHistory", Three(1)),
    ("Dialog.EditBlock", Bool(false)),
    ("Dialog.DelRemovesBlocks", Bool(true)),
    ("Dialog.AutoComplete", Bool(true)),
    ("Dialog.EULBsClear", Bool(false)),
    ("Dialog.MouseButton", Int(0xFFFF)),
    // Menu settings: Far's VMENUCLICK_IGNORE 0, _CANCEL 1, _APPLY 2
    ("VMenu.LBtnClick", Int(1)),
    ("VMenu.RBtnClick", Int(1)),
    ("VMenu.MBtnClick", Int(2)),
    // Command line settings
    ("Cmdline.EditBlock", Bool(false)),
    ("Cmdline.DelRemovesBlocks", Bool(true)),
    ("Cmdline.AutoComplete", Bool(true)),
    ("Cmdline.UsePromptFormat", Bool(false)),
    ("Cmdline.PromptFormat", Str("$p$g")),
    // AutoComplete settings
    ("Interface.Completion.ShowList", Bool(true)),
    ("Interface.Completion.ModalList", Bool(false)),
    ("Interface.Completion.Append", Bool(false)),
    // InfoPanel settings
    ("Panel.Info.ShowPowerStatus", Bool(false)),
    ("Panel.Info.ShowCDInfo", Bool(true)),
    ("Panel.Info.InfoComputerNameFormat", Int(4)),
    ("Panel.Info.InfoUserNameFormat", Int(0)),
    // Confirmations
    ("Confirmations.Copy", Bool(true)),
    ("Confirmations.Move", Bool(true)),
    ("Confirmations.RO", Bool(true)),
    ("Confirmations.Drag", Bool(true)),
    ("Confirmations.Delete", Bool(true)),
    ("Confirmations.DeleteFolder", Bool(true)),
    ("Confirmations.Esc", Bool(true)),
    ("Confirmations.RemoveConnection", Bool(true)),
    ("Confirmations.RemoveSUBST", Bool(true)),
    ("Confirmations.DetachVHD", Bool(true)),
    ("Confirmations.RemoveHotPlug", Bool(true)),
    ("Confirmations.AllowReedit", Bool(true)),
    ("Confirmations.HistoryClear", Bool(true)),
    ("Confirmations.Exit", Bool(true)),
    // File descriptions
    ("Descriptions.ListNames", Str("Descript.ion,Files.bbs")),
    ("Descriptions.SetHidden", Bool(true)),
    ("Descriptions.ROUpdate", Bool(false)),
    ("Descriptions.StartPos", Int(0)),
    ("Descriptions.UpdateMode", Int(1)),
    ("Descriptions.AnsiByDefault", Bool(false)),
    ("Descriptions.SaveInUTF", Bool(false)),
    // Viewer settings
    ("Viewer.UseExternalViewer", Bool(false)),
    ("Viewer.ExternalViewerName", Str("")),
    ("Viewer.PersistentBlocks", Bool(true)),
    ("Viewer.TabSize", Int(8)),
    ("Editor.TabSize", Int(8)),
    ("Editor.ShowKeyBar", Bool(true)),
    ("Editor.ShowLineNumbers", Bool(true)),
    ("Viewer.ShowLineNumbers", Bool(true)),
    ("Viewer.ShowArrows", Bool(true)),
    ("Viewer.Visible0x00", Bool(false)),
    ("Viewer.ShowScrollbar", Bool(false)),
    ("Viewer.SaveViewerPos", Bool(true)),
    ("Viewer.SaveViewerCodepage", Bool(true)),
    ("Viewer.SaveViewerShortPos", Bool(true)),
    ("Viewer.MaxLineSize", Int(10000)),
    ("Viewer.SaveViewMode", Bool(true)),
    ("Viewer.SaveViewerWrapMode", Bool(false)),
    ("Viewer.DetectDumpMode", Bool(true)),
    ("Viewer.AutoDetectCodePage", Bool(true)),
    ("Viewer.DefaultCodePage", Int(65001)),
    // Editor settings
    ("Editor.UseExternalEditor", Bool(false)),
    ("Editor.ExternalEditorName", Str("")),
];

pub fn path() -> std::path::PathBuf {
    crate::profile::path("settings.json")
}

static SETTINGS: RwLock<Option<Map<String, Value>>> = RwLock::new(None);

fn read_file() -> Map<String, Value> {
    // the tests see Far's defaults, not the user's file
    // (without the line numbers column, which the layout tests do not expect: the default
    // is checked against DEFAULTS in `line_numbers_are_on_by_default`)
    if cfg!(test) {
        let off = |key: &str| (key.to_string(), serde_json::json!({ "ShowLineNumbers": false }));
        return [off("Viewer"), off("Editor")].into_iter().collect();
    }
    std::fs::read_to_string(path()).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

fn with<R>(f: impl FnOnce(&Map<String, Value>) -> R) -> R {
    {
        let guard = SETTINGS.read().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = guard.as_ref() {
            return f(map);
        }
    }
    let mut guard = SETTINGS.write().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(read_file);
    f(map)
}

/// The stored value of a dotted name, if it was changed.
fn stored(key: &str) -> Option<Value> {
    with(|map| {
        let mut parts = key.split('.').peekable();
        let mut node: &Map<String, Value> = map;
        while let Some(part) = parts.next() {
            let value = node.get(part)?;
            if parts.peek().is_none() {
                return Some(value.clone());
            }
            node = value.as_object()?;
        }
        None
    })
}

/// farsh's settings and their defaults (far:config's enabled rows).
#[cfg_attr(not(test), allow(dead_code))]
pub fn defaults() -> &'static [(&'static str, Def)] {
    DEFAULTS
}

/// A setting's default (`None`: not one of farsh's). Viewer.DefaultCodePage
/// is the system's ANSI code page, as in Far.
pub fn default(key: &str) -> Option<Def> {
    let def = DEFAULTS.iter().find(|(k, _)| *k == key).map(|(_, d)| *d)?;
    #[cfg(windows)]
    if key == "Viewer.DefaultCodePage" {
        return Some(Int(i64::from(unsafe { windows_sys::Win32::Globalization::GetACP() })));
    }
    Some(def)
}

pub fn bool(key: &str) -> bool {
    match stored(key) {
        Some(Value::Bool(b)) => b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        _ => match default(key) {
            Some(Bool(b)) => b,
            Some(Int(i) | Three(i)) => i != 0,
            _ => false,
        },
    }
}

pub fn int(key: &str) -> i64 {
    match stored(key) {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)).unwrap_or(0),
        Some(Value::Bool(b)) => i64::from(b),
        _ => match default(key) {
            Some(Int(i) | Three(i)) => i,
            Some(Bool(b)) => i64::from(b),
            _ => 0,
        },
    }
}

/// Whether a setting has its default value (Far's IsDefault: what it is,
/// not whether it is stored).
pub fn is_default(key: &str) -> bool {
    match default(key) {
        Some(Bool(b)) => bool(key) == b,
        Some(Int(i) | Three(i)) => int(key) == i,
        Some(Str(s)) => string(key) == s,
        None => stored(key).is_none(),
    }
}

/// Back to the default: the stored value is dropped.
pub fn reset(key: &str) {
    if stored(key).is_none() {
        return;
    }
    let mut guard = SETTINGS.write().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(read_file);
    let parts: Vec<&str> = key.split('.').collect();
    remove(map, &parts);
    let text = serde_json::to_string_pretty(map).unwrap_or_default();
    drop(guard);
    write_file(text);
}

/// Removes `parts` from the nested map, and the objects it leaves empty.
fn remove(map: &mut Map<String, Value>, parts: &[&str]) {
    match parts {
        [] => {}
        [last] => {
            map.remove(*last);
        }
        [first, rest @ ..] => {
            if let Some(Value::Object(inner)) = map.get_mut(*first) {
                remove(inner, rest);
                if inner.is_empty() {
                    map.remove(*first);
                }
            }
        }
    }
}

fn write_file(text: String) {
    if cfg!(test) {
        return;
    }
    let file = path();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, text + "\n");
}

pub fn string(key: &str) -> String {
    match stored(key) {
        Some(Value::String(s)) => s,
        _ => match default(key) {
            Some(Str(s)) => s.to_string(),
            _ => String::new(),
        },
    }
}

/// Any value under a name (lists and objects: the masks groups, colors...).
pub fn value(key: &str) -> Option<Value> {
    stored(key)
}

/// Sets a value and writes the file.
pub fn set(key: &str, value: Value) {
    let mut guard = SETTINGS.write().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(read_file);
    let parts: Vec<&str> = key.split('.').collect();
    let mut node = map;
    for part in &parts[..parts.len() - 1] {
        let entry = node.entry(part.to_string()).or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        node = entry.as_object_mut().expect("an object");
    }
    node.insert(parts[parts.len() - 1].to_string(), value);
    let text = guard.as_ref().map(|map| serde_json::to_string_pretty(map).unwrap_or_default()).unwrap_or_default();
    drop(guard);
    write_file(text);
}

/// A checkbox's value. A 3-state is 0 or 1; an integer Far keeps as a flag
/// word (Dialog.MouseButton: 0xFFFF) is its default or 0.
pub fn set_bool(key: &str, on: bool) {
    match default(key) {
        Some(Three(_)) => set_int(key, i64::from(on)),
        Some(Int(d)) if d != 0 => set_int(key, if on { d } else { 0 }),
        _ => {
            if bool(key) != on || stored(key).is_some() {
                set(key, Value::Bool(on));
            }
        }
    }
}

pub fn set_int(key: &str, n: i64) {
    if int(key) != n || stored(key).is_some() {
        set(key, Value::from(n));
    }
}

pub fn set_string(key: &str, text: &str) {
    if string(key) != text || stored(key).is_some() {
        set(key, Value::from(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_are_on_by_default_in_the_viewer_and_the_editor() {
        for key in ["Viewer.ShowLineNumbers", "Editor.ShowLineNumbers"] {
            assert!(matches!(default(key), Some(Bool(true))), "{key}");
        }
    }
}
