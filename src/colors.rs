//! Far's palette (palette.cpp): every color by its name (`Panel.Text`,
//! `Menu.Highlight.Selected`...), Far's default unless Options > Colors
//! changed it; the changes are kept in the settings as `Colors` (each a
//! console attribute: the foreground in the low four bits, the background in
//! the high four; a missing part is the terminal's default).

use std::collections::HashMap;
use std::sync::RwLock;

use crossterm::style::Color;
use serde_json::{Map, Value};

use crate::colordata::PALETTE;
use crate::palette;

static CHANGED: RwLock<Option<HashMap<String, (u8, u8)>>> = RwLock::new(None);

fn changed() -> HashMap<String, (u8, u8)> {
    match crate::settings::value("Colors") {
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

/// A color's foreground and background (console indices).
pub fn attr(name: &str) -> (u8, u8) {
    {
        let guard = CHANGED.read().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = guard.as_ref() {
            return map.get(name).copied().unwrap_or_else(|| default_of(name));
        }
    }
    reload();
    attr(name)
}

pub fn fg(name: &str) -> Color {
    palette::fg(attr(name).0)
}

pub fn bg(name: &str) -> Color {
    palette::bg(attr(name).1)
}

/// A color set (Options > Colors > Set); `None` brings Far's default back.
pub fn set(name: &str, value: Option<(u8, u8)>) {
    let mut map: Map<String, Value> = match crate::settings::value("Colors") {
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
    crate::settings::set("Colors", Value::Object(map));
    reload();
}

/// Themes > Default: every color back to Far's.
pub fn reset_all() {
    crate::settings::set("Colors", Value::Object(Map::new()));
    reload();
}
