# farsh

Rust. The user's real shell runs in a PTY (`portable-pty`), its screen is kept by
`vt100`, and everything is drawn through our own cell grid (`src/grid.rs`) with a
diff against what the terminal shows. Far-style panels (`src/panels.rs`) are drawn
over the shell screen.

- The command line is the SHELL's own line (the cursor row of its screen), never a
  buffer of ours. `Shell::command_line_is_empty` compares that row with the last
  bare prompt; Enter/Home/End belong to the panel only while it is empty.
- bash/zsh on Windows (Git Bash, MSYS2, Cygwin) report `$PWD` in their own
  paths (`/c/Users/x`, `/tmp`); the OSC 7 hook sends `cygpath -m` of it
  instead, when `cygpath` is there (`cwd.rs`, `DIR_FN`).
- Windows' own ConPTY (portable-pty creates it with INHERIT_CURSOR) starts with a
  cursor query `ESC[6n` and shows nothing until it is answered: `Shell::pump`
  answers that first query only. Any later DSR is not ours to answer (cmd would
  echo the reply as text).
- farsh runs on Windows' defaults only. portable-pty loads `conpty.dll` by bare
  name, so `main` calls `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)`
  first: DLLs come from System32 and farsh's own folder, never from PATH (a
  Contour Terminal copy on PATH once replaced the system ConPTY unnoticed). Any
  departure from a Windows default must be written here with the reason.
- The panels copy Far Manager 3 cell for cell (its filelist.cpp, panelmix.cpp,
  strmix.cpp, keybar.cpp and the default modes/highlighting in config.cpp and
  hilight.cpp): view modes and column widths, `Folder`/`Up`/`Juncti`, sizes in
  K/M/G, extension alignment, `}` over the divider, the status line, totals,
  the key bar (its labels change while one of Shift/Ctrl/Alt is held, Far's
  MShiftF1.. sets; two or more held leave the plain labels, the user's call: `src/modkeys.rs` takes the modifier-only key events off the
  console queue before crossterm, which drops them; the terminal must send
  them in win32-input-mode, as Windows Terminal and Som do). Colors are `src/palette.rs`: Far's console attributes as
  they reach a terminal through ConPTY — the classic console table in RGB
  (blue 0,0,128...), with light gray text and a black background as the
  terminal's own defaults. Not the terminal's 16-color palette: Som's theme
  maps ANSI blue to a light Nord blue, while Far stays navy (checked live). Compare
  with the real Far by dumping both consoles' buffers (characters and
  attributes) at the same size, not by eye. Sorting starts by extension, as
  the user's Far does. Sort modes (`src/sorting.rs`) are Far's layers and
  Far's string order (`CompareString`, linguistic, case ignored, digits as
  numbers); owner, hard links and streams are read only when a mode needs
  them. The "Sort by" menu's F4 (editing the layers) is not done.
- Dialogs and menus copy Far's layout too (`src/dialog.rs`: message.cpp,
  dialog.cpp, copy.cpp, mkdir.cpp, delete.cpp; `src/menu.rs`, `src/drives.rs`:
  vmenu.cpp, diskmenu.cpp). Known, deliberate differences: "quit farsh?",
  menu items farsh does not do are grey, Copy's "Use filter" stays disabled,
  Make folder offers no link types, Change drive has no plugin rows.
- F4 runs the editor in the shell as a typed command (`src/editor.rs`). When
  the shell shows its next prompt (`Shell::prompts`, counted from the OSC 7
  hook) the panels come back, re-read, with the cursor on the edited file,
  as in Far. A shell without the hook (cmd) gives no signal: Ctrl+O there.
  F-keys work whatever is typed: F4 empties the command line first
  (`Shell::clear_line`: Esc for PSReadLine/cmd on Windows, Ctrl+E Ctrl+U for
  readline, zle and PSReadLine's Emacs mode). Every OSC 7 re-learns the bare
  prompt, since keys typed into a running program stop `write` waiting for it.
- Ctrl+A is Far's Attributes dialog (setattr.cpp of Far 3.0.6446, the
  user's build): `Layout::Placed` puts every item at Far's own X1/Y1/X2
  (80 by 21, C1 = 5, C2 = 32, the `┬│┴` and `├─╢` user separators, a row
  more for a link, two for "Process subfolders"), with 3-state check boxes,
  masked date/time fields in the locale's order and separators, buttons
  that keep the dialog open (Original/Current/Blank, More over it) and the
  dialog procedure as a hook. `src/attributes.rs` reads and sets the
  attributes (compression, encryption, sparse and reparse removal by their
  own calls), the four times (FILE_BASIC_INFO), the owner. Known
  differences: a link's target is shown but not editable, several hard
  links are not listed. On Linux/macOS only Read only (the write bits),
  the modification/access times and the owner can be set.
- F8 and Alt+Del (Wipe) go as delete.cpp goes: the same confirmation (Wipe
  has its own title and button), then every folder with something in it is
  asked about ("The following folder will be deleted/wiped:", Delete/All/
  Skip/Cancel) until All, a read-only file too (Warning, with Skip all),
  and for a wipe a file with other hard links (Error). Wipe is Far's
  EraseFile: zeros (WipeSymbol 0) written through to the end, the file cut
  to nothing, renamed to `F3T*.tmp` and removed; a folder is renamed, then
  removed. The progress box is Far's delete_progress (76 wide, the wipe
  bar, "Files:", no total bar: DelShowTotal is off by default).
- Archives are arclite's (Far's archive plugin, plugins/arclite of Far
  3.0.6446), for 7z, zip, tar and gzip only (`src/archive.rs`: sevenz-rust2,
  zip, tar, flate2, all pure Rust). arclite's dialogs are laid out by its
  own Far::Dialog (items left to right, `new_line`, the box grows to fit):
  `src/arcdialog.rs` ports it (`Layout`) and the dialogs line by line, with
  their dialog procedures as hooks (set_control_state, change_extension,
  the checks on OK as a message over the dialog). Shift+F1 is Create
  archive. Enter on an archive opens it in the panel as arclite's plugin
  panel (`ArchiveView`: title `ArcLite:zip:a.zip:\dir`, `..` even at the
  top, leaving it lands on the archive); F5/F6 there and Shift+F2 on an
  archive open Extract files, a file in the way asks arclite's Overwrite
  file (Overwrite/Skip/Rename/Append, Remember choice), F3 views a file
  from inside. A zip can be changed (arclite's updatable): F5/F6 into a zip
  the other panel shows open Add files to archive (UpdateDialog without the
  new-archive part, with "When destination file exists"), F8 in it asks
  "Do you want to delete selected files?", F6 out of it also deletes from
  it, F7 asks "Enter directory name:" (Far's InputBox, 76 by 6, Enter
  accepts; `a/b` makes both); the zip is written again into a temporary file, the old entries
  copied as they are (not recompressed), which then takes its place. Any
  other archive says arclite's "Archive modification not supported".
  Known differences: new files in an encrypted zip are
  not encrypted unless a password is typed, no SFX (7-Zip's modules), no
  profiles or saved settings (those buttons are disabled), zip cannot be
  written with LZMA or Deflate64 nor 7z with Deflate64 (an error says so),
  Shift+F2 cannot "Go to target directory".
- The Left/Right menus are Far's (config.cpp's LeftMenu): all ten view
  modes (Ctrl+1..9, 0: config.cpp's InitialModes, see `ViewMode`), Info
  panel (Ctrl+L, `src/infopanel.rs`: infolist.cpp's sections), Quick view
  (Ctrl+Q, `src/qview.rs`: qview.cpp, a folder counted as GetDirInfo does,
  in the background), Show long names (Ctrl+N: the 8.3 names from
  FILE_ID_BOTH_DIR_INFO), Panel On/Off (Ctrl+F1/F2, with Far's rules for
  which panel becomes active). Ctrl+L/Q turn the passive panel (the active
  one if it is that kind already), again a file panel. No Tree panel: Far's
  Tree.TurnOffCompletely is on by default, its menu item is hidden and
  Ctrl+T does nothing. Known differences: the info panel has no computer
  or user description lines, no plugin (archive) section, and the folder
  description is shown as text, not in a scrollable viewer.
- The Files menu is whole. Far's Gray +, Gray - and Gray * are Alt++
  (Alt+= too), Alt+- and Alt+* here, so that keyboards without a keypad
  have them (the user's decision): Select/Deselect group (SelectFiles: 55
  by 7, the last mask kept, `src/masks.rs` is filemasks.cpp + CmpName,
  folders only unmarked as with SelectFolders off), Invert (a marked folder
  unmarked). Ctrl+M restores the marks saved by the last group operation,
  Copy/Move, Link, Apply command or Describe (SaveSelection). Alt+F6 is
  copy.cpp's Link dialog (76 by 14, hard link / junction / symbolic link,
  `src/links.rs`; a junction is a mount point reparse point). Ctrl+G is
  GetString with buttons (76 by 8), then each file's line (`src/subst.rs`:
  fnparce.cpp's metasymbols; not the list files `!@!`/`!$!` nor the input
  `!?..!`) is run in the shell; the panels come back after the last
  prompt. Ctrl+Z asks "Describe file" for each name in turn and writes
  `Descript.ion` as DizList::Flush does (`src/diz.rs`: order kept, CRLF,
  hidden, removed when empty; UTF-8 with a signature when not ASCII, where
  Far writes OEM). Shift+F3 on an archive (or in one) is arclite's
  ProcessHostFile menu (`src/vmenu.rs`), "Test archive files": everything
  read through into a temporary folder, then "Archive test complete".
  Known differences: no Filter buttons (disabled), no Shift+Gray +/-
  (select/unselect all), the read-only Descript.ion question is an error.
- A left click on the top row (the panels' frame) opens the menu bar, on
  the title under the mouse, as in Far with the menu bar hidden.
- The Commands menu is whole. Its lists are Far's VMenu (`src/vmenu.rs`:
  centered, MaxHeight, scroll bar, `&` and AssignHighlights hot keys, a
  bottom title, keys it does not use go back to the caller); a dialog a
  list opens is drawn over it and the list comes back (`App::under`,
  `src/commands.rs`). What Far keeps in its profile is in farsh's
  (`src/profile.rs`: `~/.config/farsh` on every system; an old
  `%APPDATA%\farsh` is moved there once).
  - Alt+F8 / Alt+F11 / Alt+F12 (`src/history.rs`, history.cpp): the
    commands Enter ran from the prompt (`Shell::typed`), the files F3/F4
    opened, the folders the active panel visited; a date separator a day,
    Ins locks, Shift+Del deletes one, Del clears (asked), Ctrl+R forgets
    what is gone, Ctrl+Enter puts it on the command line.
  - Alt+F9 (Video mode) is Far's maximize/restore as xterm's `CSI 9;1t` /
    `CSI 9;0t`: only a terminal that does window operations follows it.
  - Compare folders is FileList::CompareDir. Edit user menu and F2 are
    usermenu.cpp with Far's own `FarMenu.ini` format (`src/usermenu.rs`);
    there is no global menu in the program folder, the main one is the
    profile's ("Main menu (User)"). File associations are filetype.cpp
    (`src/assoc.rs`): Enter, Ctrl+PgDn, F3, Alt+F3, F4, Alt+F4 on a file
    run their command in the shell (several: "Select association").
  - Folder shortcuts (`src/shortcuts.rs`): only through Commands > Folder
    shortcuts (Ins sets, Enter or the digit goes); a shortcut has one folder,
    not a list.
  - No keys with two modifiers (the user's rule): Far's Ctrl+Shift+Enter,
    RightCtrl/Ctrl+Shift+0..9, Ctrl+Shift+F3/F4, Alt+Shift+F9 are left out,
    and the key bar changes only for Shift, Ctrl or Alt alone. Alt++ / Alt+-
    still take Shift along, as the main keyboard's `+` needs it.
  - Ctrl+I (the filters, `src/filter.rs`: filefilter.cpp, the dialog of
    filefilterparams.cpp without the highlighting part) is often sent as
    Tab by terminals: then the menu is the way. Each panel keeps its marks
    across Ctrl+U (Far swaps them). Ctrl+W (task list, `src/tasklist.rs`,
    plist.cpp) is readline's while something is typed. Hotplug devices
    (`src/hotplug.rs`) is Windows only; the confirmation does not list the
    drive letters.
  - F11 is the plugins menu with arclite only (Open, Create, Extract,
    Test; Convert into SFX says it cannot), F12 the screens: the panels
    and the viewers sent to the background by F12.
  - Alt+F7 (`src/findfile.rs`): FindAskDlg and the results window. Not
    done: hex search, code pages other than UTF-8/UTF-16LE, fuzzy search,
    archives, alternate streams, Advanced, the Panel button (Far's
    temporary panel); those are disabled. Filter opens the Find file marks.
- Enter on a link: a junction or symlink is entered by its own path; one
  that denies listing itself (Windows' compatibility junctions `My Documents`,
  `Application Data`, `Start Menu`...) is followed to its target, as Far does.
  A folder that cannot be read is an error dialog; the panel stays.
- Every shortcut works in the Russian layout too (Far's Xlat): Ctrl/Alt
  with a Russian letter is the Latin letter of its key (`keys::normalize`,
  in `App::on_key`), and the hot keys of menus and dialogs match either
  layout (`keys::same`, `keys::latin`). Plain letters stay as typed.
- The Options menu is whole but for the editor's settings (the editor is
  external) and the plugins' (they will be Lua): grey. Every setting is
  Far's own, with Far's name and default, kept in
  `~/.config/farsh/settings.json` (the user's rule) as soon as it changes
  (`src/settings.rs`; the tests see only the defaults). Most dialogs are
  config.cpp's DialogBuilder ones: `src/builder.rs` ports its layout
  (TextBefore/After, Indent, two columns, LinkFlags) and binds each item to
  a setting; `src/options.rs` is the dialogs, `src/optmenu.rs` the rest.
  - What acts in farsh: the Recycle Bin for F8 (SHFileOperation), the
    three histories' saving, the sorting collation / digits / case, folder
    extensions, reverse sorts, hidden files, highlighting, select folders,
    column titles, status line, totals, free size, scroll bar, sort letter,
    `..` in the root, clock, mouse, key bar, the menu bar always shown, the
    confirmations of copy, move, delete, folders, read-only files, history
    clearing, hotplug removal and exit, the dialogs' history arrows and a
    click outside, the menus' clicks outside, the description list names
    and their hidden attribute, the folder description files, the viewer's
    tab size and external viewer (F3 / Alt+F3), Save setup (Shift+F9: the
    panels' folders, modes and sorting, restored at start; on F10 with
    "Auto save setup"). The rest is kept and shown as Far has it, with
    nothing in farsh for it yet (system copy, elevation, the command line's
    and AutoComplete's settings: the command line is the shell's).
  - Languages: English, or Russian from Far's own translation
    (`assets/lang/ru.tsv`, made from farlang.m4 and arclite_rus.lng;
    `src/lang.rs`: a whole text, or a pattern with Far's `{0}` places).
    Dialogs translate when shown; the builder before laying out.
  - File panel modes: the modes are Far's column strings (`N,S,D,T` /
    `0,6,0,5`, `NM`, `SC`, `DM`, `DE`...) with the PVS_* flags, Far's
    defaults unless changed (`ViewMode::text`). Ins/Del (more modes) not done.
  - Colors: Far's 146 palette colors by name (`src/colordata.rs` is made
    from palette.cpp and setcolor.cpp), the menus at Far's places, the color
    dialog (`src/colorpick.rs`) with the 16 console colors and Default; the
    code, 256-color and RGB fields and the styles are disabled. Panels,
    menus, dialogs, viewer, key bar and clock take them.
  - Files highlighting and sort groups (`src/highlight.rs`): Far's six
    default groups, the four parts, the filter dialog with its colors part
    (a color button opens the color dialog); the sort groups order the files
    with "Use sort groups". The color example and the marks in the panel
    are not drawn. Groups of file masks (`<arc>`, `<exec>`, `%PATHEXT%`) work
    in every mask.
- Tests: `cargo test` (the shell test starts cmd.exe on Windows, /bin/sh elsewhere).
- Code, comments and logs in English; talk to the user in Russian.

Next: F-keys
(view, edit, copy, move, delete), panel selection, mouse, macOS/Linux checks.

- A release build is always installed into `~/.local/bin` (Windows: `~/.local/bin/farsh.exe`) and nowhere else.
