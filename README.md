# farsh

Your own shell with Far Manager built in: bash on Linux, zsh on macOS,
PowerShell on Windows. Written in Rust.

farsh runs the real shell in a PTY, so your profile, aliases, history and line
editing work as they always did. `Ctrl+O` shows and hides the Far panels over the
shell screen; the command line under the panels is the shell's own prompt line.

## Keys (panels up)

| Key | Action |
|---|---|
| `Tab` | switch panel |
| `Up` `Down` `PgUp` `PgDn` | move the cursor in the panel |
| `Left` `Right` | to the next column of names (one-column views: while the command line is empty) |
| `Ctrl+1` `Ctrl+2` `Ctrl+3` `Ctrl+4` | Far's Brief, Medium, Full and Wide views |
| `Ctrl+F3`..`Ctrl+F11` | sort by name, extension, write time, size, unsorted, creation, access, descriptions, owner (again: reversed) |
| `Ctrl+F12` | the Sort by menu: all 15 Far sort modes and options (`+`/`-` set the direction, `*` flips) |
| `Shift+F11` `Shift+F12` | sort groups / marked files first |
| `Home` `End` `Enter` | panel cursor / open folder, while the command line is empty |
| `Ctrl+E` `Ctrl+X` | shell history up / down (Far's keys; the arrows belong to the panel) |
| anything else | the shell's command line |
| `Enter` over a typed command | runs it; the panels hide so its output shows |
| `Ctrl+O` | show / hide the panels |

`FARSH_SHELL` overrides the shell (default: `$SHELL`, else bash / zsh / pwsh).

## Status

First milestone: shell in a PTY, two panels with navigation, shared command line.
