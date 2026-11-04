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
    Int(i64),
    Str(&'static str),
}

use Def::{Bool, Int, Str};

/// Far's defaults (config.cpp's InitConfigsData) of what farsh shows.
const DEFAULTS: &[(&str, Def)] = &[
    // System settings
    ("System.DeleteToRecycleBin", Bool(true)),
    ("System.UseSystemCopy", Bool(true)),
    ("System.CopyOpened", Bool(true)),
    ("System.ScanJunction", Bool(true)),
    ("System.SaveHistory", Bool(true)),
    ("System.SaveFoldersHistory", Bool(true)),
    ("System.SaveViewHistory", Bool(true)),
    ("System.UseRegisteredTypes", Bool(true)),
    ("System.UpdateEnvironment", Bool(false)),
    ("System.ElevationMode", Int(-1)),
    ("System.Sort.Collation", Int(2)),
    ("System.Sort.DigitsAsNumbers", Bool(true)),
    ("System.Sort.CaseSensitive", Bool(false)),
    ("System.AutoSaveSetup", Bool(false)),
    ("System.FolderInfo", Str("DirInfo,File_Id.diz,Descript.ion,ReadMe.*,Read.Me")),
    ("System.Executor.UseHomeDir", Bool(true)),
    ("System.Executor.HomeDir", Str("%FARHOME%")),
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
    ("Panel.Layout.ScreensNumber", Bool(true)),
    ("Panel.Layout.SortMode", Bool(true)),
    ("Interface.ShowDotsInRoot", Bool(false)),
    // Interface settings
    ("Screen.Clock", Bool(true)),
    ("Interface.Mouse", Bool(true)),
    ("Screen.KeyBar", Bool(true)),
    ("Interface.ShowMenuBar", Bool(false)),
    ("Screen.ScreenSaver", Bool(false)),
    ("Screen.ScreenSaverTime", Int(5)),
    ("Interface.CopyShowTotal", Bool(true)),
    ("System.CopyTimeRule", Int(3)),
    ("Interface.DelShowTotal", Bool(false)),
    ("Interface.CtrlPgUp", Bool(true)),
    ("Interface.VirtualTerminalRendering", Bool(false)),
    ("Interface.FullWidthAwareRendering", Bool(false)),
    ("Interface.ClearType", Bool(true)),
    ("Interface.SetIcon", Bool(false)),
    ("Interface.IconIndex", Int(0)),
    ("Interface.SetAdminIcon", Bool(true)),
    ("Interface.TitleAddons", Str("%Ver %Platform %Admin")),
    ("Interface.SetPalette", Bool(true)),
    // Languages
    ("Language.Main", Str("English")),
    ("Language.Help", Str("English")),
    // Dialog settings
    ("Dialog.EditHistory", Bool(true)),
    ("Dialog.EditBlock", Bool(false)),
    ("Dialog.DelRemovesBlocks", Bool(true)),
    ("Dialog.AutoComplete", Bool(true)),
    ("Dialog.EULBsClear", Bool(false)),
    ("Dialog.MouseButton", Bool(true)),
    // Menu settings: 0 cancel, 1 execute, 2 nothing
    ("VMenu.LBtnClick", Int(0)),
    ("VMenu.RBtnClick", Int(0)),
    ("VMenu.MBtnClick", Int(1)),
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
];

pub fn path() -> std::path::PathBuf {
    crate::profile::path("settings.json")
}

static SETTINGS: RwLock<Option<Map<String, Value>>> = RwLock::new(None);

fn read_file() -> Map<String, Value> {
    // the tests see Far's defaults, not the user's file
    if cfg!(test) {
        return Map::new();
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

fn default(key: &str) -> Option<Def> {
    DEFAULTS.iter().find(|(k, _)| *k == key).map(|(_, d)| *d)
}

pub fn bool(key: &str) -> bool {
    match stored(key) {
        Some(Value::Bool(b)) => b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        _ => match default(key) {
            Some(Bool(b)) => b,
            Some(Int(i)) => i != 0,
            _ => false,
        },
    }
}

pub fn int(key: &str) -> i64 {
    match stored(key) {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::Bool(b)) => i64::from(b),
        _ => match default(key) {
            Some(Int(i)) => i,
            Some(Bool(b)) => i64::from(b),
            _ => 0,
        },
    }
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
    if cfg!(test) {
        return;
    }
    let file = path();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, text + "\n");
}

pub fn set_bool(key: &str, on: bool) {
    if bool(key) != on || stored(key).is_some() {
        set(key, Value::Bool(on));
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
