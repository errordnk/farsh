//! Far's palette (palette.cpp): every color by its name (`Panel.Text`,
//! `Menu.Highlight.Selected`...), Far's default unless Options > Colors
//! changed it; the changes are kept in the settings as `Colors` (each a
//! console attribute: the foreground in the low four bits, the background in
//! the high four; a missing part is the terminal's default).
//! A theme (`src/theme.rs`) gives a color its own RGB; what Options > Colors
//! changes is kept per theme (`Colors` for the default one, Far's, else
//! `Themes.<id>.Colors`) and goes over the theme's, in its console colors.

use std::collections::HashMap;
use std::sync::RwLock;

use crossterm::style::Color;
use serde_json::{Map, Value};

use crate::colordata::PALETTE;
use crate::palette;

static CHANGED: RwLock<Option<HashMap<String, (u8, u8)>>> = RwLock::new(None);

/// Where the changes of the theme in use are kept.
fn key() -> String {
    let id = crate::theme::with(|t| t.id.clone());
    if id == "default" { "Colors".into() } else { format!("Themes.{}.Colors", id.replace('.', "_")) }
}

fn changed() -> HashMap<String, (u8, u8)> {
    match crate::settings::value(&key()) {
        Some(Value::Object(map)) => map.iter().filter_map(|(k, v)| v.as_u64().map(|a| (k.clone(), ((a & 15) as u8, ((a >> 4) & 15) as u8)))).collect(),
        _ => HashMap::new(),
    }
}

/// The colors again from the settings (after Options > Colors).
pub fn reload() {
    *CHANGED.write().unwrap_or_else(|e| e.into_inner()) = Some(changed());
}

/// Far's default of a color.
pub fn default_of(name: &str) -> (u8, u8) {
    PALETTE.iter().find(|(n, _, _)| *n == name).map_or((palette::LIGHT_GRAY, palette::BLACK), |(_, f, b)| (*f, *b))
}

/// What Options > Colors set a color to, if it did (in the theme in use).
fn user(name: &str) -> Option<(u8, u8)> {
    {
        let guard = CHANGED.read().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = guard.as_ref() {
            return map.get(name).copied();
        }
    }
    reload();
    user(name)
}

/// A color's foreground and background (console indices): what the color
/// dialog starts with.
pub fn attr(name: &str) -> (u8, u8) {
    user(name).unwrap_or_else(|| default_of(name))
}

/// A color as the terminal gets it: Options > Colors' change, else the theme's, else Far's default.
pub fn pair(name: &str) -> (Color, Color) {
    if let Some((f, b)) = user(name) {
        return (palette::fg(f), palette::bg(b));
    }
    let (f, b) = crate::theme::with(|t| t.element(name));
    let (df, db) = default_of(name);
    (f.unwrap_or_else(|| palette::fg(df)), b.unwrap_or_else(|| palette::bg(db)))
}

pub fn fg(name: &str) -> Color {
    pair(name).0
}

pub fn bg(name: &str) -> Color {
    pair(name).1
}

/// A color set (Options > Colors > Set); `None` brings Far's default back.
pub fn set(name: &str, value: Option<(u8, u8)>) {
    let key = key();
    let mut map: Map<String, Value> = match crate::settings::value(&key) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    match value {
        Some((f, b)) => {
            map.insert(name.to_string(), Value::from(u64::from(f) | (u64::from(b) << 4)));
        }
        None => {
            map.remove(name);
        }
    }
    crate::settings::set(&key, Value::Object(map));
    reload();
}

/// Colors > Themes > Default: every color back to the theme's.
pub fn reset_all() {
    crate::settings::set(&key(), Value::Object(Map::new()));
    reload();
}
