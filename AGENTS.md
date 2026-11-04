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
  the key bar. Colors are `src/palette.rs`: Far's console attributes as
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
  it; the zip is written again into a temporary file, the old entries
  copied as they are (not recompressed), which then takes its place. Any
  other archive says arclite's "Archive modification not supported".
  Known differences: no F7 inside a zip, new files in an encrypted zip are
  not encrypted unless a password is typed, no SFX (7-Zip's modules), no
  profiles or saved settings (those buttons are disabled), zip cannot be
  written with LZMA or Deflate64 nor 7z with Deflate64 (an error says so),
  Shift+F2 cannot "Go to target directory".
- Enter on a link: a junction or symlink is entered by its own path; one
  that denies listing itself (Windows' compatibility junctions `My Documents`,
  `Application Data`, `Start Menu`...) is followed to its target, as Far does.
  A folder that cannot be read is an error dialog; the panel stays.
- Tests: `cargo test` (the shell test starts cmd.exe on Windows, /bin/sh elsewhere).
- Code, comments and logs in English; talk to the user in Russian.

Next: F-keys
(view, edit, copy, move, delete), panel selection, mouse, macOS/Linux checks.

- A release build is always installed into `~/.local/bin` (Windows: `~/.local/bin/farsh.exe`) and nowhere else.
