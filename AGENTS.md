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
- Enter on a link: a junction or symlink is entered by its own path; one
  that denies listing itself (Windows' compatibility junctions `My Documents`,
  `Application Data`, `Start Menu`...) is followed to its target, as Far does.
  A folder that cannot be read is an error dialog; the panel stays.
- Tests: `cargo test` (the shell test starts cmd.exe on Windows, /bin/sh elsewhere).
- Code, comments and logs in English; talk to the user in Russian.

Next: F-keys
(view, edit, copy, move, delete), panel selection, mouse, macOS/Linux checks.

- A release build is always installed into `~/.local/bin` (Windows: `~/.local/bin/farsh.exe`) and nowhere else.
