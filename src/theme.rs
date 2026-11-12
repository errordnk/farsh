//! farsh's themes: JSON files that say what Far's palette (`Panel.Text`,
//! `Menu.Highlight.Selected`...; `src/colordata.rs`) looks like in RGB, what
//! the 16 console colors are (the files highlighting and the colors chosen in
//! Options > Colors are console colors), and how the editor and the viewer's
//! text (fresh) look. Built in: `assets/themes/*.json`; more in
//! `~/.config/farsh/themes/*.json` (a file with a built-in's name replaces it).
//! The chosen one is the setting `Theme` (its file name without `.json`;
//! none: `default`, Far's own colors). The format is `docs/themes.md`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crossterm::style::Color;
use serde_json::{Map, Value};

/// The built-in themes: their id (file name) and JSON.
const BUILTIN: &[(&str, &str)] = &[
    ("default", include_str!("../assets/themes/default.json")),
    ("catppuccin", include_str!("../assets/themes/catppuccin.json")),
    ("tokyo-night", include_str!("../assets/themes/tokyo-night.json")),
    ("one-dark", include_str!("../assets/themes/one-dark.json")),
    ("dracula", include_str!("../assets/themes/dracula.json")),
    ("nord", include_str!("../assets/themes/nord.json")),
];

/// Far's names of the 16 console colors (setcolor.cpp's order), as a theme writes them.
pub const CONSOLE_NAMES: [&str; 16] = [
    "black",
    "blue",
    "green",
    "cyan",
    "red",
    "magenta",
    "brown",
    "lightgray",
    "darkgray",
    "lightblue",
    "lightgreen",
    "lightcyan",
    "lightred",
    "lightmagenta",
    "yellow",
    "white",
];

/// A part of an element: its foreground and background, either may be left to the default.
type Pair = (Option<Color>, Option<Color>);

#[derive(Debug)]
pub struct Theme {
    pub id: String,
    pub name: String,
    /// The 16 console colors.
    pub console: [Color; 16],
    /// What Far's default attribute (light gray on black) is: the terminal's own unless said.
    pub default_fg: Color,
    pub default_bg: Color,
    /// Far's palette names, or `Prefix.*` / `*` patterns, and their colors.
    elements: HashMap<String, Pair>,
    /// fresh's theme (its own format), the colors in it already as fresh takes them.
    pub editor: Option<Value>,
}

/// Far's classic console table, as ConPTY gives it to the terminal (`src/palette.rs`).
pub const CLASSIC: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (0, 0, 128),
    (0, 128, 0),
    (0, 128, 128),
    (128, 0, 0),
    (128, 0, 128),
    (128, 128, 0),
    (192, 192, 192),
    (128, 128, 128),
    (0, 0, 255),
    (0, 255, 0),
    (0, 255, 255),
    (255, 0, 0),
    (255, 0, 255),
    (255, 255, 0),
    (255, 255, 255),
];

fn classic() -> [Color; 16] {
    CLASSIC.map(|(r, g, b)| Color::Rgb { r, g, b })
}

impl Theme {
    /// Far's own colors: what farsh had before themes.
    fn far() -> Self {
        Self {
            id: "default".into(),
            name: "Default".into(),
            console: classic(),
            default_fg: Color::Reset,
            default_bg: Color::Reset,
            elements: HashMap::new(),
            editor: None,
        }
    }

    /// The theme's colors of an element (`None` where it leaves a part to Far's
    /// default): the name itself, then `A.B.*`, `A.*`, `*`, each part on its own.
    pub fn element(&self, name: &str) -> Pair {
        let (mut fg, mut bg) = (None, None);
        let mut look = |key: &str| {
            if let Some((f, b)) = self.elements.get(key) {
                fg = fg.or(*f);
                bg = bg.or(*b);
            }
        };
        look(name);
        let mut rest = name;
        while let Some(dot) = rest.rfind('.') {
            rest = &rest[..dot];
            look(&format!("{rest}.*"));
        }
        look("*");
        (fg, bg)
    }

    /// Reads a theme's JSON; `id` is its file name.
    pub fn parse(id: &str, text: &str) -> Result<Self, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let root = root.as_object().ok_or("a theme is a JSON object")?;
        let mut theme = Self::far();
        theme.id = id.to_string();
        theme.name = root.get("name").and_then(Value::as_str).unwrap_or(id).to_string();
        let vars = match root.get("palette") {
            Some(Value::Object(map)) => map.clone(),
            None => Map::new(),
            Some(_) => return Err("\"palette\" is an object of named colors".into()),
        };
        let resolver = Resolver { vars: &vars };
        // the console colors first: the elements may name them
        match root.get("console") {
            None => {}
            Some(Value::Array(list)) => {
                if list.len() != 16 {
                    return Err("\"console\" as a list has 16 colors".into());
                }
                for (i, v) in list.iter().enumerate() {
                    theme.console[i] = resolver.color(v, &theme.console, 0).map_err(|e| format!("console[{i}]: {e}"))?;
                }
            }
            Some(Value::Object(map)) => {
                for (key, v) in map {
                    let i = console_index(key).ok_or_else(|| format!("console: \"{key}\" is not a console color"))?;
                    let color = resolver.color(v, &theme.console, 0).map_err(|e| format!("console.{key}: {e}"))?;
                    theme.console[i] = color;
                }
            }
            Some(_) => return Err("\"console\" is a list of 16 colors or an object by their names".into()),
        }
        let console = theme.console;
        if let Some(default) = root.get("default") {
            let (f, b) = resolver.pair(default, &console).map_err(|e| format!("default: {e}"))?;
            theme.default_fg = f.unwrap_or(theme.default_fg);
            theme.default_bg = b.unwrap_or(theme.default_bg);
        }
        match root.get("elements") {
            None => {}
            Some(Value::Object(map)) => {
                for (key, v) in map {
                    let pair = resolver.pair(v, &console).map_err(|e| format!("elements.{key}: {e}"))?;
                    theme.elements.insert(key.clone(), pair);
                }
            }
            Some(_) => return Err("\"elements\" is an object by Far's color names".into()),
        }
        match root.get("editor") {
            None => {}
            Some(Value::Object(map)) => {
                let mut editor = Value::Object(map.clone());
                resolver.convert(&mut editor, &console, "");
                theme.editor = Some(editor);
            }
            Some(_) => return Err("\"editor\" is a fresh theme (an object)".into()),
        }
        Ok(theme)
    }
}

fn console_index(name: &str) -> Option<usize> {
    let name = name.to_ascii_lowercase().replace([' ', '_', '-'], "");
    CONSOLE_NAMES.iter().position(|n| *n == name)
}

struct Resolver<'a> {
    vars: &'a Map<String, Value>,
}

impl Resolver<'_> {
    /// A color: `#rrggbb`, `#rgb`, `[r, g, b]`, `terminal` (the terminal's own),
    /// a console color's name, or a name from the theme's `palette`.
    fn color(&self, value: &Value, console: &[Color; 16], depth: u8) -> Result<Color, String> {
        match value {
            Value::Array(list) if list.len() == 3 => {
                let part = |v: &Value| v.as_u64().filter(|n| *n < 256).map(|n| n as u8).ok_or("a part of [r, g, b] is 0..255");
                Ok(Color::Rgb { r: part(&list[0])?, g: part(&list[1])?, b: part(&list[2])? })
            }
            Value::String(text) => self.named(text, console, depth),
            _ => Err(format!("{value} is not a color")),
        }
    }

    fn named(&self, text: &str, console: &[Color; 16], depth: u8) -> Result<Color, String> {
        if let Some(hex) = text.strip_prefix('#') {
            return hex_color(hex).ok_or_else(|| format!("\"{text}\" is not #rrggbb"));
        }
        if text.eq_ignore_ascii_case("terminal") {
            return Ok(Color::Reset);
        }
        if let Some(v) = self.vars.get(text) {
            if depth > 8 {
                return Err(format!("\"{text}\" names itself"));
            }
            return self.color(v, console, depth + 1);
        }
        if let Some(i) = console_index(text) {
            return Ok(console[i]);
        }
        Err(format!("\"{text}\" is not a color"))
    }

    /// An element's colors: `{"fg": .., "bg": ..}` (either may be left out) or `[fg, bg]`.
    fn pair(&self, value: &Value, console: &[Color; 16]) -> Result<Pair, String> {
        let part = |v: Option<&Value>| v.filter(|v| !v.is_null()).map(|v| self.color(v, console, 0)).transpose();
        match value {
            Value::Object(map) => {
                if let Some(key) = map.keys().find(|k| *k != "fg" && *k != "bg") {
                    return Err(format!("\"{key}\": an element has \"fg\" and \"bg\""));
                }
                Ok((part(map.get("fg"))?, part(map.get("bg"))?))
            }
            Value::Array(list) if list.len() == 2 => Ok((part(list.first())?, part(list.get(1))?)),
            _ => Err("an element is {\"fg\": .., \"bg\": ..} or [fg, bg]".into()),
        }
    }

    /// The colors of a fresh theme in farsh's way of writing them, as fresh
    /// takes them (`[r, g, b]`, `"Default"`). Names (`name`, `extends`) and
    /// modifiers stay as they are, and so does any string that is no color.
    fn convert(&self, value: &mut Value, console: &[Color; 16], key: &str) {
        match value {
            Value::Object(map) => {
                for (k, v) in map.iter_mut() {
                    if k != "name" && k != "extends" && k != "modifier" {
                        self.convert(v, console, k);
                    }
                }
            }
            Value::String(text) if !key.is_empty() => {
                if let Ok(color) = self.named(text, console, 0) {
                    *value = fresh_color(color);
                }
            }
            _ => {}
        }
    }
}

fn hex_color(hex: &str) -> Option<Color> {
    let digit = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok();
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 => Some(Color::Rgb { r: digit(0)? * 17, g: digit(1)? * 17, b: digit(2)? * 17 }),
        6 => Some(Color::Rgb { r: byte(0)?, g: byte(2)?, b: byte(4)? }),
        _ => None,
    }
}

/// A color as fresh's theme JSON writes it.
pub fn fresh_color(color: Color) -> Value {
    match color {
        Color::Rgb { r, g, b } => serde_json::json!([r, g, b]),
        _ => serde_json::json!("Default"),
    }
}

/// Where the user's themes are.
pub fn user_dir() -> PathBuf {
    crate::profile::path("themes")
}

/// A theme of the list: its id, its name, or why it cannot be used.
pub struct Listed {
    pub id: String,
    pub name: Result<String, String>,
    pub user: bool,
}

fn user_files() -> Vec<(String, PathBuf)> {
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(user_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")))
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
        .collect();
    files.sort_by_key(|(id, _)| id.to_lowercase());
    files
}

/// The text of a theme by its id: the user's file, else the built-in.
fn source(id: &str) -> Option<(String, bool)> {
    if !cfg!(test)
        && let Some((_, path)) = user_files().into_iter().find(|(name, _)| name == id)
    {
        return std::fs::read_to_string(path).ok().map(|text| (text, true));
    }
    BUILTIN.iter().find(|(name, _)| *name == id).map(|(_, text)| ((*text).to_string(), false))
}

/// Every theme: the built-in ones in their order, then the user's (by name).
pub fn list() -> Vec<Listed> {
    let users = if cfg!(test) { Vec::new() } else { user_files() };
    let read = |id: &str, text: &str| Theme::parse(id, text).map(|t| t.name);
    let mut out: Vec<Listed> = BUILTIN
        .iter()
        .map(|(id, text)| match users.iter().find(|(name, _)| name == id) {
            Some((_, path)) => Listed { id: (*id).into(), name: std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| read(id, &t)), user: true },
            None => Listed { id: (*id).into(), name: read(id, text), user: false },
        })
        .collect();
    for (id, path) in users.iter().filter(|(id, _)| !BUILTIN.iter().any(|(b, _)| b == id)) {
        out.push(Listed { id: id.clone(), name: std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| read(id, &t)), user: true });
    }
    out
}

/// A theme by its id (`None`: no such theme, or it does not read).
pub fn load(id: &str) -> Option<Theme> {
    let (text, _) = source(id)?;
    Theme::parse(id, &text).ok()
}

static ACTIVE: RwLock<Option<Arc<Theme>>> = RwLock::new(None);

/// The chosen theme's id (the setting `Theme`).
pub fn chosen() -> String {
    match crate::settings::value("Theme") {
        Some(Value::String(id)) if !id.is_empty() => id,
        _ => "default".into(),
    }
}

/// The theme in use: the chosen one, Far's colors if it is gone or does not read.
pub fn active() -> Arc<Theme> {
    if let Some(theme) = ACTIVE.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return theme.clone();
    }
    let theme = Arc::new(load(&chosen()).unwrap_or_else(Theme::far));
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = Some(theme.clone());
    theme
}

/// Runs `f` with the theme in use, without taking a reference of it.
pub fn with<R>(f: impl FnOnce(&Theme) -> R) -> R {
    if let Some(theme) = ACTIVE.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return f(theme);
    }
    f(&active())
}

/// Options > Themes: `id` becomes the theme (read again, so an edited file shows its changes).
pub fn choose(id: &str) -> Result<(), String> {
    let (text, _) = source(id).ok_or_else(|| format!("no theme \"{id}\""))?;
    let theme = Theme::parse(id, &text)?;
    if id == "default" {
        crate::settings::reset("Theme");
    } else {
        crate::settings::set("Theme", Value::from(id));
    }
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(theme));
    crate::colors::reload();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_theme_reads() {
        for (id, text) in BUILTIN {
            let theme = Theme::parse(id, text).unwrap_or_else(|e| panic!("{id}: {e}"));
            if *id != "default" {
                // every color of Far's palette is the theme's own, none is left to Far's blue
                for (name, _, _) in crate::colordata::PALETTE {
                    let (f, b) = theme.element(name);
                    let default = crate::colors::default_of(name);
                    assert!(f.is_some() || default.0 == crate::palette::LIGHT_GRAY, "{id}: {name} has no foreground");
                    assert!(b.is_some() || default.1 == crate::palette::BLACK, "{id}: {name} has no background");
                }
                assert!(theme.editor.is_some(), "{id}: the editor's colors");
            }
        }
    }

    #[test]
    fn the_default_theme_is_fars_own_colors() {
        let theme = Theme::parse("default", BUILTIN[0].1).unwrap();
        assert_eq!(theme.console, classic());
        assert_eq!((theme.default_fg, theme.default_bg), (Color::Reset, Color::Reset));
        assert_eq!(theme.element("Panel.Text"), (None, None));
    }

    #[test]
    fn colors_patterns_and_parts() {
        let text = r##"{
            "name": "Test",
            "palette": { "base": "#102030", "ink": "fg", "fg": [1, 2, 3] },
            "console": { "blue": "base", "light cyan": "#abc" },
            "default": { "fg": "terminal", "bg": "base" },
            "elements": {
                "Dialog.*": { "bg": "lightcyan" },
                "Dialog.Edit": ["ink", null],
                "*": { "fg": "white" }
            },
            "editor": { "extends": "builtin://dark", "editor": { "bg": "base" }, "syntax": { "keyword": { "color": "blue", "modifier": ["bold"] } } }
        }"##;
        let theme = Theme::parse("t", text).unwrap();
        let base = Color::Rgb { r: 0x10, g: 0x20, b: 0x30 };
        assert_eq!(theme.name, "Test");
        assert_eq!(theme.console[1], base);
        assert_eq!(theme.console[11], Color::Rgb { r: 0xaa, g: 0xbb, b: 0xcc });
        assert_eq!((theme.default_fg, theme.default_bg), (Color::Reset, base));
        let cyan = Color::Rgb { r: 0xaa, g: 0xbb, b: 0xcc };
        assert_eq!(theme.element("Dialog.Edit"), (Some(Color::Rgb { r: 1, g: 2, b: 3 }), Some(cyan)));
        assert_eq!(theme.element("Dialog.List.Text"), (Some(Color::Rgb { r: 255, g: 255, b: 255 }), Some(cyan)));
        assert_eq!(theme.element("Panel.Text"), (Some(Color::Rgb { r: 255, g: 255, b: 255 }), None));
        let editor = theme.editor.unwrap();
        assert_eq!(editor["extends"], "builtin://dark");
        assert_eq!(editor["editor"]["bg"], serde_json::json!([0x10, 0x20, 0x30]));
        assert_eq!(editor["syntax"]["keyword"]["color"], serde_json::json!([0x10, 0x20, 0x30]));
        assert_eq!(editor["syntax"]["keyword"]["modifier"], serde_json::json!(["bold"]));
    }

    #[test]
    fn mistakes_are_said() {
        assert!(Theme::parse("t", r#"{"elements": {"Panel.Text": {"fg": "nope"}}}"#).unwrap_err().contains("nope"));
        assert!(Theme::parse("t", r##"{"console": ["#000"]}"##).is_err());
        assert!(Theme::parse("t", r#"{"palette": {"a": "b", "b": "a"}, "default": {"fg": "a"}}"#).is_err());
        assert!(Theme::parse("t", r#"{"elements": {"Panel.Text": {"color": "red"}}}"#).is_err());
    }
}
