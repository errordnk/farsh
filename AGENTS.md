# farsh

Rust. The user's real shell runs in a PTY (`portable-pty`), its screen is kept by
`vt100`, and everything is drawn through our own cell grid (`src/grid.rs`) with a
diff against what the terminal shows. Far-style panels (`src/panels.rs`) are drawn
over the shell screen.

- The command line is the SHELL's own line (the cursor row of its screen), never a
  buffer of ours. `Shell::command_line_is_empty` compares that row with the last
  bare prompt; Enter/Home/End belong to the panel only while it is empty.
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
  the key bar. Colors are the terminal's 16 colors (what Far's console
  attributes become through ConPTY), so a theme changes both alike. Compare
  with the real Far by dumping both consoles' buffers (characters and
  attributes) at the same size, not by eye. Sorting starts by extension, as
  the user's Far does. Sort modes (`src/sorting.rs`) are Far's layers and
  Far's string order (`CompareString`, linguistic, case ignored, digits as
  numbers); owner, hard links and streams are read only when a mode needs
  them. The "Sort by" menu's F4 (editing the layers) is not done.
- Tests: `cargo test` (the shell test starts cmd.exe on Windows, /bin/sh elsewhere).
- Code, comments and logs in English; talk to the user in Russian.

Next: F-keys
(view, edit, copy, move, delete), panel selection, mouse, macOS/Linux checks.

- A release build is always installed into `~/.local/bin` (Windows: `~/.local/bin/farsh.exe`) and nowhere else.
