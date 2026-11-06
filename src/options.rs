//! The Options menu's dialogs, as config.cpp builds them (DialogBuilder,
//! `crate::builder`): every item bound to a setting of
//! `~/.config/farsh/settings.json`. Not here: the editor's settings (the
//! editor is external) and the plugins' (they will be Lua).

use crate::builder::{Builder, Settings};
use crate::dialog::Dialog;

/// Options > System settings (SystemSettings).
pub fn system() -> Dialog {
    let mut b = Builder::new("System settings");
    b.check("System.DeleteToRecycleBin", "&Delete to Recycle Bin");
    b.check("System.UseSystemCopy", "Use sys&tem copy routine");
    b.check("System.CopyOpened", "Copy files opened for &writing");
    b.check("System.ScanJunction", "Scan s&ymbolic links");
    b.check("System.SaveHistory", "Save commands &history");
    b.check("System.SaveFoldersHistory", "Save &folders history");
    b.check("System.SaveViewHistory", "Save &view and edit history");
    b.check("System.UseRegisteredTypes", "Use Windows re&gistered types");
    b.check("System.UpdateEnvironment", "Automatic update of &environment variables");
    b.text("Request administrator rights:");
    let modify = b.check_bit("elev_modify", "System.ElevationMode", 1, "For &modification");
    b.indent(modify, 4);
    let read = b.check_bit("elev_read", "System.ElevationMode", 2, "For &reading");
    b.indent(read, 4);
    let privileges = b.check_bit("elev_priv", "System.ElevationMode", 4, "Use additional &privileges");
    b.indent(privileges, 4);
    let collation = b.combo("System.Sort.Collation", 20, &[("ordinal", 0), ("invariant", 1), ("linguistic", 2)]);
    b.text_before(collation, "Sorti&ng collation:");
    let digits = b.check("System.Sort.DigitsAsNumbers", "Treat digits &as numbers");
    b.indent(digits, 4);
    let case = b.check("System.Sort.CaseSensitive", "&Case sensitive");
    b.indent(case, 4);
    b.check("System.AutoSaveSetup", "Auto &save setup");
    b.ok_cancel();
    b.build(Settings::System)
}

/// Options > Panel settings (PanelSettings).
pub fn panel() -> Dialog {
    let mut b = Builder::new("Panel settings");
    b.check("Panel.ShowHidden", "Show &hidden and system files");
    b.check("Panel.Highlight", "Hi&ghlight files");
    b.check("Panel.SelectFolders", "Select &folders");
    b.check("Panel.RightClickSelect", "Right clic&k selects files");
    b.check("Panel.SortFolderExt", "Sort folder names by e&xtension");
    b.check("Panel.ReverseSort", "Allow re&verse sort modes");
    let enabled = b.check("Panel.AutoUpdateEnabled", "&Disable automatic panel update");
    let limit = b.int_edit("Panel.AutoUpdateLimit", 6);
    b.link(enabled, limit);
    let label = b.text_before(limit, "if ob&ject count exceeds");
    b.indent(label, 4);
    b.indent(limit, 4);
    b.check("System.AutoUpdateRemoteDrive", "Network drives autor&efresh");
    b.separator(None);
    b.check("Panel.Layout.ColumnTitles", "Show &column titles");
    b.check("Panel.Layout.StatusLine", "Show &status line");
    b.check("Panel.Layout.DetailedJunction", "Detect volume mount &points");
    b.check("Panel.Layout.TotalInfo", "Show files &total information");
    b.check("Panel.Layout.FreeInfo", "Show f&ree size");
    b.check("Panel.Layout.Scrollbar", "Show scroll&bar");
    b.check("Panel.Layout.ScreensNumber", "Show background screens &number");
    b.check("Panel.Layout.SortMode", "Show sort &mode letter");
    b.check("Interface.ShowDotsInRoot", "Show \"&..\" in root folders");
    b.ok_cancel();
    b.build(Settings::Panel)
}

/// Options > Interface settings (InterfaceSettings).
pub fn interface() -> Dialog {
    let mut b = Builder::new("Interface settings");
    b.check("Screen.Clock", "&Clock");
    b.check("Interface.Mouse", "M&ouse");
    b.check("Screen.KeyBar", "Show &key bar");
    b.check("Interface.ShowMenuBar", "Always show the &menu bar");
    let saver = b.check("Screen.ScreenSaver", "&Screen saver");
    let minutes = b.int_edit("Screen.ScreenSaverTime", 3);
    b.indent(minutes, 5);
    b.text_after(minutes, "mi&nutes");
    b.link(saver, minutes);
    b.check("Interface.CopyShowTotal", "Show &total copy progress indicator");
    b.check("System.CopyTimeRule", "Show cop&ying time information");
    b.check("Interface.DelShowTotal", "Show total &delete progress indicator");
    b.check("Interface.CtrlPgUp", "Use Ctrl+Pg&Up to change drive");
    b.check("Interface.VirtualTerminalRendering", "Use &Virtual Terminal for rendering");
    b.check("Interface.FullWidthAwareRendering", "Fullwidth-aware rendering");
    b.check("Interface.ClearType", "Cl&earType-friendly redraw (can be slow)");
    b.start_columns();
    let icon = b.check("Interface.SetIcon", "Console &icon:");
    b.column_break();
    let index = b.combo("Interface.IconIndex", 0, &[("0", 0), ("1", 1), ("2", 2), ("3", 3), ("4", 4), ("5", 5), ("6", 6), ("7", 7)]);
    b.end_columns();
    b.link(icon, index);
    let admin = b.check("Interface.SetAdminIcon", "Alternate for &Administrator");
    b.indent(admin, 4);
    b.link(icon, admin);
    b.text("Far &window title addons:");
    b.edit("Interface.TitleAddons", 47);
    b.ok_cancel();
    b.build(Settings::Interface)
}

/// Options > Dialog settings.
pub fn dialogs() -> Dialog {
    let mut b = Builder::new("Dialog settings");
    b.check("Dialog.EditHistory", "&History in dialog edit controls");
    b.check("Dialog.EditBlock", "&Persistent blocks in edit controls");
    b.check("Dialog.DelRemovesBlocks", "&Del removes blocks in edit controls");
    b.check("Dialog.AutoComplete", "&AutoComplete in edit controls");
    b.check("Dialog.EULBsClear", "&Backspace deletes unchanged text");
    b.check("Dialog.MouseButton", "Mouse click &outside a dialog closes it");
    b.ok_cancel();
    b.build(Settings::Dialog)
}

/// Options > Menu settings (VMenuSettings).
pub fn menus() -> Dialog {
    // Far's VMENUCLICK_CANCEL, _APPLY, _IGNORE
    let clicks = [("Cancel menu", 1), ("Execute selected item", 2), ("Do nothing", 0)];
    let mut b = Builder::new("Menu settings");
    for (key, label) in [
        ("VMenu.LBtnClick", "&Left mouse click outside a menu"),
        ("VMenu.RBtnClick", "&Right mouse click outside a menu"),
        ("VMenu.MBtnClick", "&Middle mouse click outside a menu"),
    ] {
        b.text(label);
        b.combo(key, 40, &clicks);
    }
    b.ok_cancel();
    b.build(Settings::Menu)
}

/// Options > Command line settings (CmdlineSettings).
pub fn cmdline() -> Dialog {
    let mut b = Builder::new("Command line settings");
    b.check("Cmdline.EditBlock", "&Persistent blocks");
    b.check("Cmdline.DelRemovesBlocks", "&Del removes blocks");
    b.check("Cmdline.AutoComplete", "&AutoComplete");
    let prompt = b.check("Cmdline.UsePromptFormat", "Set command line prompt &format");
    let format = b.edit("Cmdline.PromptFormat", 33);
    b.indent(format, 4);
    b.link(prompt, format);
    let home = b.check("System.Executor.UseHomeDir", "Use &home dir");
    let dir = b.edit("System.Executor.~", 33);
    b.indent(dir, 4);
    b.link(home, dir);
    b.ok_cancel();
    b.build(Settings::Cmdline)
}

/// Options > AutoComplete settings.
pub fn autocomplete() -> Dialog {
    let mut b = Builder::new("AutoComplete settings");
    let list = b.check("Interface.Completion.ShowList", "&Show a list");
    let modal = b.check("Interface.Completion.ModalList", "&Modal mode");
    b.indent(modal, 4);
    b.check("Interface.Completion.Append", "&Append the first matched item");
    b.link(list, modal);
    b.ok_cancel();
    b.build(Settings::AutoComplete)
}

/// Options > InfoPanel settings.
pub fn infopanel() -> Dialog {
    let computer = [
        ("NetBIOS", 0),
        ("DNS hostname", 1),
        ("DNS domain", 2),
        ("DNS fully-qualified (FQDN)", 3),
        ("Physical NetBIOS", 4),
        ("Physical DNS hostname", 5),
        ("Physical DNS domain", 6),
        ("Physical DNS fully-qualified (FQDN)", 7),
    ];
    let user = [
        ("Logon name", 0),
        ("Fully Qualified Domain Name (FQDN)", 1),
        ("Sam Compatible", 2),
        ("Display Name", 3),
        ("Unique Id", 6),
        ("Canonical Name", 7),
        ("User Principial Name", 8),
        ("Service Principal", 10),
        ("DNS domain", 12),
        ("Given name", 13),
        ("Surname", 14),
    ];
    let mut b = Builder::new("InfoPanel settings");
    b.check("Panel.Info.ShowPowerStatus", "Show &power status");
    b.check("Panel.Info.ShowCDInfo", "Show &CD drive parameters");
    b.text("Computer &name format");
    b.combo("Panel.Info.InfoComputerNameFormat", 50, &computer);
    b.text("&User name format");
    b.combo("Panel.Info.InfoUserNameFormat", 50, &user);
    b.ok_cancel();
    b.build(Settings::InfoPanel)
}

/// Options > Confirmations (SetConfirmations).
pub fn confirmations() -> Dialog {
    let mut b = Builder::new("Confirmations");
    for (key, label) in [
        ("Confirmations.Copy", "&Copy"),
        ("Confirmations.Move", "&Move"),
        ("Confirmations.RO", "&Overwrite and delete R/O files"),
        ("Confirmations.Drag", "&Drag and drop"),
        ("Confirmations.Delete", "De&lete"),
        ("Confirmations.DeleteFolder", "Delete non-empty &folders"),
        ("Confirmations.Esc", "&Interrupt operation"),
        ("Confirmations.RemoveConnection", "Disconnect &network drive"),
        ("Confirmations.RemoveSUBST", "Delete &SUBST-disk"),
        ("Confirmations.DetachVHD", "Detach &virtual disk"),
        ("Confirmations.RemoveHotPlug", "Hot&Plug-device removal"),
        ("Confirmations.AllowReedit", "&Reload edited file"),
        ("Confirmations.HistoryClear", "Clear &history list"),
        ("Confirmations.Exit", "E&xit"),
    ] {
        b.check(key, label);
    }
    b.ok_cancel();
    b.build(Settings::Confirmations)
}

/// Options > File descriptions (SetDizConfig).
pub fn descriptions() -> Dialog {
    let mut b = Builder::new("File descriptions");
    b.text("Description &list names delimited with commas:");
    b.edit("Descriptions.ListNames", 65);
    b.separator(None);
    b.check("Descriptions.SetHidden", "Set \"&Hidden\" attribute to new description lists");
    b.check("Descriptions.ROUpdate", "Update &read only description file");
    let start = b.int_edit("Descriptions.StartPos", 2);
    b.text_after(start, "&Position of new descriptions in the string");
    b.separator(None);
    b.radio("Descriptions.UpdateMode", &["Do &not update descriptions", "Update if &displayed", "&Always update"]);
    b.separator(None);
    b.check("Descriptions.AnsiByDefault", "Use ANS&I code page by default");
    b.check("Descriptions.SaveInUTF", "Save in UTF-&8");
    b.ok_cancel();
    b.build(Settings::Descriptions)
}

/// Options > Viewer settings (ViewerConfig, not Local).
pub fn viewer() -> Dialog {
    let mut b = Builder::new("Viewer");
    b.check("Viewer.UseExternalViewer", "Use external viewer for F&3 instead of Alt+F3");
    b.text("&Viewer command:");
    b.edit("Viewer.ExternalViewerName", 64);
    b.separator(Some("Internal viewer"));
    b.start_columns();
    b.check("Viewer.PersistentBlocks", "&Persistent selection");
    let tab = b.int_edit("Viewer.TabSize", 3);
    b.text_after(tab, "&Tab size");
    b.column_break();
    b.check("Viewer.ShowArrows", "Show scrolling arro&ws");
    b.check("Viewer.Visible0x00", "Visible '\\&0'");
    b.check("Viewer.ShowScrollbar", "Show scro&llbar");
    b.end_columns();
    b.separator(None);
    b.start_columns();
    b.check("Viewer.SaveViewerPos", "&Save file position");
    b.check("Viewer.SaveViewerCodepage", "Save file &code page");
    b.check("Viewer.SaveViewerShortPos", "Save &bookmarks");
    let max = b.int_edit("Viewer.MaxLineSize", 6);
    b.text_after(max, "&Maximum line width");
    b.column_break();
    b.check("Viewer.SaveViewMode", "Save vi&ew mode");
    b.check("Viewer.SaveViewerWrapMode", "Save w&rap mode");
    b.check("Viewer.DetectDumpMode", "Detect d&ump view mode");
    b.check("Viewer.AutoDetectCodePage", "&Autodetect code page");
    b.end_columns();
    b.text("&Default code page:");
    b.combo("Viewer.DefaultCodePage", 64, &crate::codepages::combo_items());
    b.ok_cancel();
    b.build(Settings::Viewer)
}

/// Options > Editor settings (EditorConfig): the external editor. Far's own
/// internal-editor options are not here: farsh's internal editor is to be
/// fresh, which keeps its own settings (docs/fresh-plan.md).
pub fn editor() -> Dialog {
    let mut b = Builder::new("Editor");
    b.separator(Some("External editor"));
    b.check("Editor.UseExternalEditor", "Use external editor for &F4 instead of Alt+F4");
    b.text("Editor &command:");
    b.edit("Editor.ExternalEditorName", 64);
    b.ok_cancel();
    b.build(Settings::Editor)
}

/// Options > Folder description files (SetFolderInfoFiles: GetString with buttons).
pub fn folder_info() -> Dialog {
    let value = crate::settings::string("System.FolderInfo");
    Dialog::get_string("Folder description files", "Enter file names delimited with commas (wildcards are allowed)", &value, crate::dialog::Action::FolderInfo)
}

/// Options > Save setup (Shift+F9): Far asks first.
pub fn save_setup() -> Dialog {
    let lines = [crate::lang::tr("Do you wish to save").to_string(), crate::lang::tr("current setup?").to_string()];
    Dialog::message("Save setup", &lines, &[("Save", "ok"), ("Cancel", "cancel")], false, crate::dialog::Action::SaveSetup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::Binding;
    use crate::dialog::Action;

    #[test]
    fn the_editor_dialog_sets_the_external_editor() {
        let mut dialog = editor();
        let Action::Settings(kind, bindings) = dialog.action_mut() else { panic!("an Options dialog") };
        assert_eq!(*kind, Settings::Editor);
        assert!(bindings.iter().any(|b| matches!(b, Binding::Bool("Editor.UseExternalEditor"))));
        assert!(bindings.iter().any(|b| matches!(b, Binding::Str("Editor.ExternalEditorName"))));
    }
}