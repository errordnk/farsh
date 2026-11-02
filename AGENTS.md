# farsh

Rust. The user's real shell runs in a PTY (`portable-pty`), its screen is kept by
`vt100`, and everything is drawn through our own cell grid (`src/grid.rs`) with a
diff against what the terminal shows. Far-style panels (`src/panels.rs`) are drawn
over the shell screen.

- The command line is the SHELL's own line (the cursor row of its screen), never a
  buffer of ours. `Shell::command_line_is_empty` compares that row with the last
  bare prompt; Enter/Home/End belong to the panel only while it is empty.
- No terminal replies are needed from us for ConPTY (a DSR answer gets echoed by
  cmd as text).
- Tests: `cargo test` (the shell test starts cmd.exe on Windows, /bin/sh elsewhere).
- Code, comments and logs in English; talk to the user in Russian.

Next: sync the shell's cwd with the active panel (OSC 7 / shell integration), F-keys
(view, edit, copy, move, delete), panel selection, mouse, macOS/Linux checks.
