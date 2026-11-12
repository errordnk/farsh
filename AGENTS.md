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
  them in win32-input-mode, as Windows Terminal and Som do). The key bar's
  layout is NOT Far's (the user's call, 2026-11-07): F1-F6 stand under the
  left panel, F7-F12 under the right, each panel's six shared out over its
  columns — two under each of Brief's three, three under each of Medium's two,
  six under a one-column mode, evenly under a panel that is not a file list
  (`Panels::key_spans`); a full-screen panel and the help keep one even row of
  twelve (`panels::paint_labels`, at least 8 columns a key, fewer keys when
  narrower). Widths are even everywhere, the leftover columns one each to the
  first keys: Far's (ScrX-1)/12 left F12 up to twice as long. The reference is Far 3.0.6446, the user's Far:
  fetch its sources at the `builds/3.0.6446` tag, not master. Colors are `src/palette.rs`: Far's console attributes as
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
- F4 is the internal editor: fresh over the whole screen (`Viewer::open_edit`,
  `fresh_view::editor_options`), Far's status bar on top, the key bar below
  (F2 save, F6 viewer, F10/Esc quit and ask about changes, Shift+F10 save and
  quit). F3's viewer is the same screen read-only; F6 switches. The paragraph
  below is the EXTERNAL editor (`Editor.UseExternalEditor`, Alt+F4): it runs
  in the shell as a typed command (`src/editor.rs`). When
  the shell shows its next prompt (`Shell::prompts`, counted from the OSC 7
  hook) the panels come back, re-read, with the cursor on the edited file,
  as in Far. A shell without the hook (cmd) gives no signal: Ctrl+O there.
  F-keys work whatever is typed: F4 empties the command line first
  (`Shell::clear_line`: Esc for PSReadLine/cmd on Windows, Ctrl+E Ctrl+U for
  readline, zle and PSReadLine's Emacs mode). Every OSC 7 re-learns the bare
  prompt, since keys typed into a running program stop `write` waiting for it.
- Alt and a character over a file panel is Far's quick search (`src/fastfind.rs`,
  fastfind.cpp and `FileList::FindPartName` of 3.0.6446): a 22 by 3 "Search"
  box over the panel's bottom border, the command line and the key bar, from the
  panel's ninth column; the first Alt character is lower-cased, what is typed
  after it narrows the search to names that begin with it (case ignored, `*` and
  `?` are wildcards, brackets plain, a closing `\` or `/` asks for folders; a
  character no name continues with is not taken); Backspace shortens, Ctrl+Y
  clears, Ctrl+Enter goes to the next such name, Esc or F10 or a click closes it,
  any other key closes it and is the panel's own (Enter enters, the arrows move).
  Alt+Plus, Alt+Minus and Alt+* stay the group marks (`Panels::fast_key`). Not
  done: Ctrl+Shift+Enter (previous; two modifiers), paste into the box, Alt+Shift
  as the first key (the one-modifier rule drops it).
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
- "Access denied" during a delete is Far's elevation (`src/elevation.rs`,
  elevation.cpp): the 64x12 dialog (OK with the UAC shield `▀▄`, Skip; "Do
  this for all current objects" on, "Do not ask again in the current
  session"; the focus is on OK, the user's choice: Far focuses "Do not ask
  again"), then on OK a copy of farsh started through UAC (`farsh
  --elevation <pipe> <pid>`, kept for the session) deletes, removes folders,
  clears read-only, lists, reads attributes and recycles over a named pipe
  only administrators may open, and serves only the pid that started it.
  System.ElevationMode says for what (1 change, 2 read). Esc/Skip leaves the
  item and the folders holding it. Windows only; copy and move are not
  elevated yet.
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
  in the background; NOT Far's status bar under the view - its separator,
  file type and name line - the user's call, 2026-11-07: the inside of the
  frame is the file's to the bottom border, in either layout), Show long names (Ctrl+N: the 8.3 names from
  FILE_ID_BOTH_DIR_INFO), Panel On/Off (Ctrl+F1/F2, with Far's rules for
  which panel becomes active). Ctrl+L/Q turn the passive panel (the active
  one if it is that kind already), again a file panel. While a quick view
  stands beside a file list the layout is NOT Far's (the user's call,
  2026-11-07; `Panels::quick_layout`): the file panel is one column of names
  (Brief's, extensions aligned) in a sixth of the screen on its own side, the
  viewer takes the other five sixths; both keep their frames and the folder
  or "Quick view" title in the border (`Panel::paint_compact`: no column
  titles, status line or totals; the view mode is not changed, so leaving
  Ctrl+Q shows the panel as it was). The command line is there and the key bar
  is all twelve keys in one even row across the screen (the narrow panel has
  no room for six of its own). (A first version without frames and with only
  F3/F4 was tried and taken back the same day.) A binary file
  (`qview::is_binary`: a NUL or over a tenth control
  bytes in the first 8 KiB; a UTF-16/32 BOM is text) that is not a placement
  shows a line saying so instead of its bytes. The text of any other file in
  the quick view (framed or not) is fresh, our fork of the editor (GPL-3.0, as
  farsh is), built in as a library: read-only, highlighted, no line numbers,
  wrapped, no animations (`fresh_view::viewer_options`; docs/fresh-plan.md).
  ONE `fresh::embed::Session` lives as long as a quick view is shown
  (`Panels::qeditor`) and is moved from file to file with `open_other` (which
  closes the file before it: a session is one file) after the cursor has stayed
  150 ms on a file (`qview::State::load`; a file not yet read keeps showing the
  one before). `Panels::tick` runs its housekeeping in the main loop, `busy`
  keeps the screen redrawing only while it asks for frames (`next_frame`); the
  wheel over the quick view and the scroll keys of an active one go to it
  (`EditorView::event`, mouse positions taken from the rectangle's origin).
  `fresh_view::GridBackend` is a ratatui backend that keeps fresh's frame as
  farsh `Cell`s (which gained italic, dim and crossed-out). The embedded
  editor uses the user's own fresh folders (`~/.config/fresh`...); tests use a
  temporary tree per thread. No Tree panel: Far's
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
- Without panels (Ctrl+O hides them) farsh is the shell itself, nothing
  else (plan: `docs/shell-passthrough-plan.md`). The shell owns the terminal's
  MAIN screen: what it writes goes to the terminal exactly as it wrote it
  (`App::sync_screen`), so the terminal's own history, reflow, selection,
  wheel, OSC (cwd, title, links) and mouse modes work as without farsh. farsh
  only keeps a copy of the shell's screen (`vt100`) for the panels. The
  panels are on the ALTERNATE screen: Ctrl+O enters it, what the shell
  writes meanwhile waits (`App::pending`, up to 16 MB, then the screen is
  redrawn from the copy) and goes to the main screen when the panels leave.
  The shell's ConPTY starts with a cursor query: it is answered with the
  terminal's real cursor (`shell::set_start_cursor`), so what was on the
  screen stays.
- Input without panels (Windows, `src/rawkeys.rs`): every console input
  record goes to the shell's ConPTY as it came, in win32-input-mode
  (releases, modifier keys alone, scan codes, AltGr), the mouse as the
  reports the shell's program asked for, a resize as a resize. Only Ctrl+O
  is farsh's, and only at a prompt (`Shell::ctrl_o_is_ours`: the prompt hook
  reported it and no command runs, no program of the shell is on its
  alternate screen; a shell without the hook always gives Ctrl+O to farsh).
  On Linux/macOS (`src/rawinput.rs`) the terminal's own bytes go to the
  shell's PTY as they came (escape sequences, kitty keyboard, mouse reports,
  bracketed paste: the modes the shell asked for in its output reach the
  terminal as they are); only Ctrl+O is looked for (byte 0x0F or kitty's
  `CSI 111;5u`), bytes that came with it wait until the shell's screen is
  back, and a resize is noticed by polling `terminal::size()` (crossterm,
  which hears SIGWINCH, is not reading). `keys::encode` remains for what the
  panels type themselves.
  Windows' limit: the shell sits behind two conhosts (farsh's console and
  the shell's ConPTY), whatever the system's ConPTY drops on the way is
  dropped (Windows defaults only).
- farsh can be the user's login shell on Linux/macOS (`src/login.rs`: put its
  path in `/etc/shells`, `chsh -s`). The real shell is `$FARSH_SHELL`, the
  profile's `Shell.Program`, or `$SHELL`, the first that is not farsh (with
  farsh as the login shell `$SHELL` is farsh), else bash / zsh; farsh is never
  its own shell. Only an interactive session on a terminal is farsh's: `-c
  cmd` (ssh, scp, rsync, `make`, vim's `:!`), a script, `-s`, any argument it
  does not know, or stdin/stdout that is not a terminal make farsh `exec` the
  real shell with the same arguments, printing nothing. `-farsh` as argv[0],
  `-l` and `--login` make the session a login one: the shell under farsh
  reads the login files in a login shell's order (zsh gets stubs for
  `.zprofile`/`.zlogin`; bash gets an rcfile that reads /etc/profile and the
  first of ~/.bash_profile, ~/.bash_login, ~/.profile, then the hook: a real
  `bash -l` ignores `--rcfile`, and a prompt tool that replaces
  `PROMPT_COMMAND` would take the hook away; differences: `shopt login_shell`
  is off, `.bash_logout` runs from a trap). The shell under farsh gets
  `SHELL` = the real shell (vim's `:sh`, tmux windows are not farsh). A login
  session never ends because farsh broke: if farsh cannot start its session
  or panics, the terminal is put right and the real shell takes over (`exec`
  as `-bash`). A debug build panics on `FARSH_DEBUG_PANIC` to show that.
- F3 and the quick view (Ctrl+Q) show what Som shows through SRP, its
  rich-content protocol, when farsh runs in Som (`SOM_WINDOW_ID`; `FARSH_SRP=1`
  forces it): `src/srp/`. A placement is placeholder cells farsh draws
  (`Grid::place`: U+10EEEE with a row and a column mark, foreground = session
  id, underline color SGR 58 = file id, so `Cell` has `ul`; both colors go
  out as raw SGR, since crossterm drops colors under `NO_COLOR` and the ids
  with them) and the file's
  bytes sent to the `somsrv` daemon on this machine (JSON `PutChunk`s over
  its pipe/socket). Som fits the picture into the rectangle itself, so farsh
  never asks the terminal for the cell size. The protocol is Som's own
  `srp_client` crate (`som/crates/srp_client`: the message types `somsrv`
  uses, the pipe/socket framing, the placeholder marks, `SrvChannel`, the
  `.cue` parser), a git dependency pinned to a rev of `errordnk/som`: bump
  the rev when Som's protocol changes. What Som could still change is in
  `docs/srp-findings.md`. A placement in the quick view takes no keys (the
  user's call): its player is used with the mouse, and Som leaves space and
  the arrows to farsh for a placement narrower than the terminal. farsh only connects to a running daemon: it never
  starts or replaces one (a new `somsrv` ends every session, farsh lives in
  one). The transfer runs in a thread (the file is read and sent while the
  placement is already drawn: grid first, then bytes); dropping the viewer
  cancels it; the quick view waits 150 ms on a file first. A picture also gets a
  range responder until the placement goes: Som may see the cells in the middle
  of the transfer and asks for the whole file again from the start, and the
  transfer's own connection is only written to (an 8 MB GIF showed nothing
  without it, in somsrp too). Done: pictures
  (png, jpg, gif) and audio (mp3, flac, ogg, oga). Audio is played by Som
  (decoding and speakers are there): farsh reads the shape, length and tags
  from the headers (symphonia, `srp/audio.rs`), announces the track
  (`AnnounceTrack`, `len: 0` = no playlist), sends the first 256 KiB, registers
  a second connection as the range responder and answers Som's
  `RequestByteRange` from the open file until `EndPlayback` or until the viewer
  is dropped (then `StopPlayback`; the responder's blocked read is cut by
  `Connection::close`). The whole viewer rectangle is the placement: Som
  centers its 40x1 widget in it. Video (mp4, mkv, avi) goes the same way,
  without a track announce and with the file's last 4 MiB sent along with its
  front; its size, rate and codec are not probed (zeros, as `somsrp` off
  Windows: Som reads the real ones and letterboxes). `.m3u`/`.m3u8` and `.cue`
  (`srp/play.rs`, `srp/cue.rs`, ported from somsrp): each track is a placement
  of its own (`Shared.ids` changes, `Media::busy` makes the viewer redraw), the
  list runner moves on at `EndPlayback` or at the widget's previous/next
  (`PlaylistStep`) or `n`/`p` in F3; http(s) entries are fetched by the daemon
  (`FetchResource`); a cue sheet's track list is announced (`AnnounceCue`). Not
  yet: HLS. A playlist line naming Yandex (`yandex:wave`) is the service's radio
  (`srp/radio.rs`, ported from somsrp: My Wave in batches fetched through the
  daemon's linked account, feedback and likes reported, the widget's like and
  wave icons via `RadioControl`, `n`/`p` and the history kept in farsh); it
  needs the account linked in `somsrv`, and a test daemon made from the user's
  build uses that account too, so radio is not covered by daemon tests. Markdown
  (`.md`, `run_markdown`): the file is sent with its folder, a second connection
  registered with `RegisterMarkdownGrower` hears `GrowMarkdownRows` and
  `Shared.rows` grows; F3 shows it over the whole screen with no bars (Som paints
  a document over everything below its first cell, not only over its cells) and
  scrolls by drawing a window of the placement's rows (`Grid::place_from`); the
  last rows of a long document are reached with PgDn/arrows (the window may go
  down to one row: Som paints more than the rows it asks for), not with End; see
  `docs/srp-findings.md` item 14. In F3 `a` goes to a video's next audio stream and
  `s` to its next subtitle stream, then to none (the video restarts with it; the
  status line names it). The streams are listed with FFmpeg on Windows only,
  as somsrp probes video (the user's call): `src/srp/streams.rs`, avformat,
  avcodec and avutil delay-loaded, put into farsh.exe (with swresample, which
  avcodec imports itself: without it the first `a` ended farsh with 0xC06D007E,
  since `main` limits the DLL search and `cargo test` had hidden it by putting
  the vcpkg tree on PATH; the stream test limits the search as `main` does) by `build.rs` from the
  vcpkg tree it links against (`.cargo/config.toml`: VCPKG_ROOT, the same tree
  as Som; `ffmpeg-sys-next` stays at 9.0.0, as in Som: 9.0.1's build script
  looks for headers in /usr/include) and written to `%TEMP%\farsh-ffmpeg-<hash>`
  on the first `a`/`s`. Som's FFmpeg has no `file` protocol, so the file is read
  through `input_from_stream`. Elsewhere `a`/`s` and a digit choose by number.
- The mouse is farsh's only while the panels or something over them are
  shown, or a program in the shell asked for it (`App::sync_mouse`; on
  Windows just ENABLE_MOUSE_INPUT, with QuickEdit when it is off). Without
  that the terminal selects, copies and pastes with a right click itself;
  over the panels it selects with Shift (Som, Windows Terminal).
  Interface.Mouse off leaves the mouse to the terminal.
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
  - Every key of farsh has no modifier or one (the user's rule):
    `keys::chord` stops any key with two or more before panels, menus and
    dialogs see it (over the panels it goes to the shell). So Far's
    Ctrl+Shift+Enter, RightCtrl/Ctrl+Shift+0..9, Ctrl+Shift+F3/F4,
    Alt+Shift+F9 are left out, Alt++ is the keypad's (the main keyboard's
    `+` takes Shift), and the key bar changes only for Shift, Ctrl or Alt
    alone: held by itself for 50 ms, and after two held together not until
    all are let go (`src/modkeys.rs`), so a chord never flickers it. AltGr's characters (Ctrl+Alt + a non-Latin key) are typed text.
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
  - F1 (`src/help.rs`) is 6446's help.cpp over Far's own help files
    (`assets/help/FarEng.hlf`, `FarRus.hlf`, Far's BSD license beside
    them), in Language.Help's language: ReadHelp's layout, OutString's
    markup and colors, MoveToReference, the keys and the mouse, F7's search
    (fuzzy and regex grey: not there yet), Shift+F2's plugins list (empty:
    no plugin has help yet). Checked against Far's console dump: the same
    cells. `<!Macro:…>` lists are skipped (farsh has no macros).
  - `far:config` (typed, Enter; `src/farconfig.rs`) is Far 3.0.6446's
    AdvancedConfig cell for cell (checked against Far's console dump): all
    297 settings from `src/farconfig_data.rs`, generated from the real Far's
    far:config with a fresh profile (`Far.exe -s <roaming> <local>`), so
    their names, types and defaults are Far's. A setting `settings` does not
    know is shown with Far's default and disabled (the editor's, plugins',
    macros'...); a farsh setting must have Far's name, type and default (a
    test checks the types). 3-state settings are `Def::Three` (0/1/2), and
    VMenu clicks are Far's VMENUCLICK values (0 nothing, 1 cancel, 2 apply).
    Shift+F1 opens the setting's help topic, else its group's `…Settings`. Masked
    fields take a value as Far does (`dialog::fit_mask`): a character equal
    to the mask's fixed one there is used up by it, so the hex field shows
    `0x` and 15 digits, as Far's does.
  - What acts in farsh: the Recycle Bin for F8 (SHFileOperation), the
    three histories' saving, the sorting collation / digits / case, folder
    extensions, reverse sorts, hidden files, highlighting, select folders,
    column titles, status line, totals, free size, scroll bar, sort letter,
    `..` in the root, clock, mouse, key bar, the menu bar always shown, the
    confirmations of copy, move, delete, folders, read-only files, history
    clearing, hotplug removal and exit, the dialogs' history arrows and a
    click outside, the menus' clicks outside, the description list names
    and their hidden attribute, the folder description files, the viewer's
    tab size and external viewer (F3 / Alt+F3), the external editor (Options >
    Editor settings: "Use external editor for F4 instead of Alt+F4" and its
    command, `Editor.UseExternalEditor` / `Editor.ExternalEditorName`; the
    other key opens the internal editor, which until fresh is built in is the
    default editor run in the shell; Far's internal-editor settings stay grey,
    fresh keeps its own), Save setup (Shift+F9: the
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
  - Themes (Options > Themes, after Colors; farsh's own, not Far's: the
    user's request of 2026-11-12) are JSON (`src/theme.rs`, the format in
    `docs/themes.md`): RGB for Far's palette names (`Prefix.*` patterns), the
    16 console colors, the default attribute and fresh's theme for the
    editor/viewer text. Built in (`assets/themes/`): default (Far's colors,
    exactly as before), catppuccin, tokyo-night, one-dark, dracula, nord;
    the user's in `~/.config/farsh/themes/` (a built-in's name replaces it).
    The dialog (`src/themepick.rs`, the user's design) is the list and an
    example of farsh in the theme under the cursor: a file panel, the editor
    (a read-only fresh session over `%TEMP%\farsh-theme-example\hello.c`,
    opened again for each theme) and the key bar, painted under
    `theme::previewing` (a thread-local theme), so the theme in use changes
    only on Enter. The setting `Theme` (not Far's, so not in `settings::DEFAULTS`). Options >
    Colors' changes are kept per theme (`Colors` for default,
    `Themes.<id>.Colors`). `palette::fg/bg` are the theme's console colors;
    SRP widgets follow it through the placeholder cells' background. The
    built-in JSONs are generated from one element table (a script, not kept):
    edit them alike.
  - Files highlighting and sort groups (`src/highlight.rs`): Far's six
    default groups, the four parts, the filter dialog with its colors part
    (a color button opens the color dialog); the sort groups order the files
    with "Use sort groups". The color example and the marks in the panel
    are not drawn. Groups of file masks (`<arc>`, `<exec>`, `%PATHEXT%`) work
    in every mask.
- The site (`https://farsh.sarnet.ru/`, Russian; README.md is its English
  counterpart, keep the two in step) is `site/` (index.html, img/). It is
  served by nginx on the `vpn` host (146.103.40.167, user dnk; the
  `farsh.sarnet.ru` server in `/etc/nginx/sites-available/sarnet.ru`) from
  `/home/dnk/farsh/site`: a plain copy, NOT a git clone; `git push` does not
  publish it. Publish only when asked: `scp` index.html and the changed images,
  then compare `sha256sum` on both sides. `site/img/themes-*.webp` are drawn
  from the Themes dialog's real cells (a dump of the grid rendered with Cascadia
  Mono), not mock-ups; `farsh-*.webp` are the user's screenshots.
- Tests: `cargo test` (the shell test starts cmd.exe on Windows, /bin/sh elsewhere).
- Code, comments and logs in English; talk to the user in Russian.

Next: F-keys
(view, edit, copy, move, delete), panel selection, mouse, macOS/Linux checks.

- The F3 viewer and the Ctrl+Q quick view are not to become a copy of
  Far's viewer: they get Som's SRP built in, everything `somsrp` does
  (`som/crates/somsrp`: images, GIF, audio, video with its streams and
  subtitles, .cue; the placeholder grid on the PTY, the bytes over
  somsrv's side channel), the user's decision of 2026-11-05.

- A release build is always installed into `~/.local/bin` (Windows: `~/.local/bin/farsh.exe`) and nowhere else.
