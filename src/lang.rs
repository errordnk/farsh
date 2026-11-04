//! The interface language (Far's Options > Languages): English, or
//! Russian from Far's own translation (farlang.m4's FarRus and arclite's
//! arclite_rus.lng, `assets/lang/ru.tsv`: Far's message name, the English
//! text, the Russian one). farsh's texts are Far's English ones: a text is
//! looked up whole; one with Far's `{0}` places (`Copy {0} {1}:`) matches
//! its pattern and its pieces are looked up in turn.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

static RUSSIAN: AtomicBool = AtomicBool::new(false);

const TABLE: &str = include_str!("../assets/lang/ru.tsv");

/// The languages Options > Languages offers: (name in the settings, as shown).
pub const LANGUAGES: [(&str, &str); 2] = [("English", "English"), ("Russian", "Русский")];

struct Lang {
    exact: HashMap<&'static str, &'static str>,
    /// The texts with places: the English pieces between them, the place numbers, the Russian text.
    patterns: Vec<(Vec<&'static str>, Vec<usize>, &'static str)>,
}

fn lang() -> &'static Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    LANG.get_or_init(|| {
        let mut exact = HashMap::new();
        let mut patterns = Vec::new();
        for line in TABLE.lines() {
            let mut fields = line.splitn(3, '\t');
            let (Some(_), Some(en), Some(ru)) = (fields.next(), fields.next(), fields.next()) else { continue };
            exact.entry(en).or_insert(ru);
            if en.contains("{0}") {
                let (pieces, places) = split_places(en);
                // a pattern needs some text of its own
                if pieces.iter().map(|p| p.len()).sum::<usize>() >= 3 {
                    patterns.push((pieces, places, ru));
                }
            }
        }
        Lang { exact, patterns }
    })
}

/// `Copy {0} {1}:` -> (["Copy ", " ", ":"], [0, 1]).
fn split_places(text: &str) -> (Vec<&'static str>, Vec<usize>) {
    let chars: Vec<char> = text.chars().collect();
    let mut pieces = Vec::new();
    let mut places = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' && i + 2 < chars.len() && chars[i + 1].is_ascii_digit() && chars[i + 2] == '}' {
            pieces.push(&*Box::leak(std::mem::take(&mut current).into_boxed_str()));
            places.push(chars[i + 1] as usize - '0' as usize);
            i += 3;
        } else {
            current.push(chars[i]);
            i += 1;
        }
    }
    pieces.push(&*Box::leak(current.into_boxed_str()));
    (pieces, places)
}

pub fn set_russian(on: bool) {
    RUSSIAN.store(on, Ordering::Relaxed);
}

pub fn russian() -> bool {
    RUSSIAN.load(Ordering::Relaxed)
}

/// Applies the language of the settings.
pub fn load() {
    set_russian(crate::settings::string("Language.Main") == "Russian");
}

/// A fixed text in the current language.
pub fn tr(text: &'static str) -> &'static str {
    if !russian() {
        return text;
    }
    lang().exact.get(text).copied().unwrap_or(text)
}

/// Any text in the current language: whole, or by a pattern with places.
pub fn trs(text: &str) -> String {
    if !russian() || text.is_empty() {
        return text.to_string();
    }
    to_russian(text)
}

fn to_russian(text: &str) -> String {
    let lang = lang();
    if let Some(ru) = lang.exact.get(text) {
        return ru.to_string();
    }
    for (pieces, places, ru) in &lang.patterns {
        if let Some(args) = matches(text, pieces) {
            let mut out = (*ru).to_string();
            for (place, arg) in places.iter().zip(args) {
                let arg = lang.exact.get(arg.as_str()).map_or(arg.clone(), |t| t.to_string());
                out = out.replace(&format!("{{{place}}}"), &arg);
            }
            return out;
        }
    }
    text.to_string()
}

/// The values in the places, if `text` is `pieces` with something between them.
fn matches(text: &str, pieces: &[&str]) -> Option<Vec<String>> {
    let first = pieces.first()?;
    let mut rest = text.strip_prefix(first)?;
    let mut args = Vec::new();
    for (i, piece) in pieces.iter().enumerate().skip(1) {
        let last = i == pieces.len() - 1;
        let at = if last {
            if !rest.ends_with(piece) {
                return None;
            }
            rest.len() - piece.len()
        } else if piece.is_empty() {
            return None;
        } else {
            rest.find(piece)?
        };
        if at == 0 {
            return None;
        }
        args.push(rest[..at].to_string());
        rest = &rest[at + piece.len()..];
    }
    Some(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_russian_texts() {
        // the global language is not switched: other tests run meanwhile
        assert_eq!(to_russian("S&ystem settings"), "Систе&мные параметры");
        assert_eq!(to_russian("Copy README t&o:"), format!("Копировать README {}:", to_russian("t&o")));
        assert_eq!(to_russian("not Far's text"), "not Far's text");
        assert_eq!(tr("S&ystem settings"), "S&ystem settings");
    }
}
