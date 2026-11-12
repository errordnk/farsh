# farsh themes

Options > Themes (right after Colors) is a dialog: the themes on the left,
and on the right an example of farsh in the theme under the cursor - a file
panel (Far's default highlighting, a marked file, the cursor, the totals), the
editor with "hello, world" in C (fresh, with the theme's syntax colors) and the
key bar. The arrows, PgUp/PgDn, Home/End, a letter (the next name starting
with it) or the mouse move the cursor; Enter or OK makes it the theme, Esc
keeps the one there was. Built in: Default
(Far's own colors), Catppuccin Mocha, Tokyo Night, One Dark, Dracula, Nord
(`assets/themes/*.json`). More themes are JSON files in
`~/.config/farsh/themes/` (Windows: `%USERPROFILE%\.config\farsh\themes\`);
the file name without `.json` is the theme's id, and a file named like a
built-in one (`nord.json`) takes its place. The choice is the setting `Theme`
in `settings.json` (no setting: `default`). Choosing a theme reads its file
again, so a theme being written is seen by opening the dialog once more. A
file that does not read is listed as "(cannot be read)", and the example's
place says why.

A theme colors the whole of farsh: the panels, menus, dialogs, key bar, help,
the F3/F4 screens, the editor's and viewer's text (fresh) and the background of
SRP placements in Som (pictures' margins, the audio/video and markdown
widgets: Som paints a widget on the background of the placement's cells).

## The file

```json
{
  "name": "My theme",
  "palette": { "base": "#1e1e2e", "text": "#cdd6f4", "accent": "#89b4fa" },
  "console": { "blue": "#1e66f5", "lightgreen": "#a6e3a1" },
  "default": { "fg": "terminal", "bg": "terminal" },
  "elements": {
    "Panel.*": { "fg": "text", "bg": "base" },
    "Panel.Cursor": { "fg": "base", "bg": "accent" },
    "Dialog.Edit": ["text", "#181825"]
  },
  "editor": {
    "extends": "builtin://dark",
    "syntax": { "keyword": "accent", "string": "lightgreen" }
  }
}
```

Every key may be left out.

- `name`: shown in the menu (else the file name).
- `palette`: names for colors, used anywhere below (a name may name another).
- `console`: the 16 console colors, either a list of 16 or by name: `black`,
  `blue`, `green`, `cyan`, `red`, `magenta`, `brown`, `lightgray`, `darkgray`,
  `lightblue`, `lightgreen`, `lightcyan`, `lightred`, `lightmagenta`,
  `yellow`, `white` (case, spaces, `-` and `_` do not matter). The files
  highlighting groups and the colors chosen in Options > Colors are console
  colors, so they take these. Missing ones are Far's classic table
  (blue `#000080`, cyan `#008080`...).
- `default`: what Far's default attribute (light gray on black) becomes,
  `{"fg": .., "bg": ..}`. Far's own: the terminal's colors (`terminal`). The
  command line row and every element left to `lightgray`/`black` use it.
- `elements`: Far's palette names (Options > Colors, `src/colordata.rs`:
  `Panel.Text`, `Menu.Highlight.Selected`, `WarnDialog.Button.Selected`...)
  and their colors, `{"fg": .., "bg": ..}` (either part may be left out) or
  `[fg, bg]` (`null` for none). A name may end in `.*`, and `*` alone is
  everything: for `Dialog.List.Text` farsh looks at `Dialog.List.Text`, then
  `Dialog.List.*`, `Dialog.*`, `*`, each part (fg, bg) on its own, so the
  more exact name wins. A part no entry gives is Far's default for that
  element, in the theme's `console`.
- `editor`: a [fresh](https://github.com/errordnk/fresh) theme (its own
  format: `editor`, `ui`, `syntax`, `search`, `diagnostic`; see fresh's
  `themes/*.json`) for F4, F3 and the quick view's text. It goes over fresh's
  `builtin://dark` unless it `extends` another (`builtin://dracula`,
  `builtin://nord`, `builtin://tokyo-night`, `builtin://gruvbox`,
  `builtin://solarized-dark`...). Colors here may be written as anywhere in
  the theme (`#rrggbb`, palette names, console names), or as fresh writes
  them (`[r, g, b]`, `{"color": .., "modifier": ["bold"]}`). The text area's
  `bg`, `after_eof_bg`, `fg`, `selection_bg`, `selection_fg` and
  `line_number_bg` always come from farsh's elements (`Editor.Text`,
  `Editor.Text.Selected`; the viewer's `Viewer.Text`, the quick view's
  `Panel.Text`), so Options > Colors changes them too.

A color is `#rrggbb`, `#rgb`, `[r, g, b]`, `terminal` (the terminal's own
foreground or background), a console color's name or a `palette` name.

## Options > Colors and themes

What Options > Colors changes (a console color for an element) goes over the
theme, and is kept per theme: `Colors` in `settings.json` for the default
theme (as before themes), `Themes.<id>.Colors` for another. Colors > Themes >
Default brings the theme in use back to its own colors.

## Not done

- A viewer sent to the background with F12 keeps the colors it was opened in.
- Som's SRP widgets take only their background from farsh (the cells'); their
  glyphs, progress bar and markdown links are Som's own colors.
