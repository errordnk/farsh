//! Options > Code pages (codepage_selection.cpp's SelectCodePage): System
//! (ANSI, OEM), Unicode (UTF-8, UTF-16 LE/BE), Favorites, Other, each item
//! `1251 │ name`. Ins adds a code page to the favorites, Del takes it out,
//! Ctrl+H hides the others (CPMenuMode), F4 renames one; the favorites and
//! names are kept in the settings (`CodePages.Favorites`, `CodePages.Names`).

use std::sync::OnceLock;

use serde_json::Value;

use crate::settings;
use crate::vmenu::{Entry, VMenu};

#[derive(Clone, Debug)]
pub struct CodePage {
    pub cp: u32,
    pub name: String,
}

/// The system's ANSI and OEM code pages, then the installed ones (Windows).
#[cfg(windows)]
fn system() -> (u32, u32, Vec<CodePage>) {
    use windows_sys::Win32::Globalization::{EnumSystemCodePagesW, GetACP, GetCPInfoExW, GetOEMCP, CPINFOEXW, CP_INSTALLED};
    static FOUND: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());
    unsafe extern "system" fn each(name: windows_sys::core::PCWSTR) -> i32 {
        let text = unsafe {
            let mut len = 0;
            while *name.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(name, len))
        };
        if let Ok(cp) = text.parse() {
            FOUND.lock().unwrap_or_else(|e| e.into_inner()).push(cp);
        }
        1
    }
    let name_of = |cp: u32| -> Option<(String, u32)> {
        let mut info: CPINFOEXW = unsafe { std::mem::zeroed() };
        if unsafe { GetCPInfoExW(cp, 0, &mut info) } == 0 {
            return None;
        }
        let len = info.CodePageName.iter().position(|&c| c == 0).unwrap_or(info.CodePageName.len());
        let name = String::from_utf16_lossy(&info.CodePageName[..len]);
        // "1251  (ANSI - Кириллица)" -> "ANSI - Кириллица"
        let name = name.split_once('(').map(|(_, rest)| rest.trim_end_matches(')').to_string()).unwrap_or(name);
        Some((name, info.MaxCharSize))
    };
    FOUND.lock().unwrap_or_else(|e| e.into_inner()).clear();
    unsafe { EnumSystemCodePagesW(Some(each), CP_INSTALLED) };
    let mut cps = FOUND.lock().unwrap_or_else(|e| e.into_inner()).clone();
    cps.sort_unstable();
    let installed = cps
        .into_iter()
        .filter_map(|cp| name_of(cp).filter(|(_, max)| *max > 0 && *max <= 2).map(|(name, _)| CodePage { cp, name }))
        .collect();
    unsafe { (GetACP(), GetOEMCP(), installed) }
}

#[cfg(not(windows))]
fn system() -> (u32, u32, Vec<CodePage>) {
    let known = [
        (437, "OEM United States"),
        (850, "OEM Multilingual Latin 1"),
        (866, "OEM Russian"),
        (1250, "ANSI Central European"),
        (1251, "ANSI Cyrillic"),
        (1252, "ANSI Latin 1"),
        (20866, "Russian (KOI8-R)"),
        (21866, "Ukrainian (KOI8-U)"),
        (28591, "ISO 8859-1 Latin 1"),
        (28595, "ISO 8859-5 Cyrillic"),
    ];
    (65001, 65001, known.iter().map(|(cp, name)| CodePage { cp: *cp, name: name.to_string() }).collect())
}

fn installed() -> &'static (u32, u32, Vec<CodePage>) {
    static ALL: OnceLock<(u32, u32, Vec<CodePage>)> = OnceLock::new();
    ALL.get_or_init(system)
}

fn unicode() -> [CodePage; 3] {
    [
        CodePage { cp: 65001, name: "UTF-8".into() },
        CodePage { cp: 1200, name: "UTF-16 (Little endian)".into() },
        CodePage { cp: 1201, name: "UTF-16 (Big endian)".into() },
    ]
}

pub fn favorites() -> Vec<u32> {
    match settings::value("CodePages.Favorites") {
        Some(Value::Array(list)) => list.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect(),
        _ => vec![],
    }
}

fn custom_name(cp: u32) -> Option<String> {
    match settings::value(&format!("CodePages.Names.{cp}")) {
        Some(Value::String(name)) => Some(name),
        _ => None,
    }
}

/// The rows of the menu: a code page, or a separator's title.
pub fn rows(hide_others: bool) -> Vec<Result<CodePage, &'static str>> {
    let (ansi, oem, others) = installed();
    let named = |cp: u32, prefix: &str| -> CodePage {
        let name = others.iter().find(|c| c.cp == cp).map(|c| c.name.clone()).unwrap_or_else(|| cp.to_string());
        let name = if name.starts_with(prefix) { name } else { format!("{prefix} - {name}") };
        CodePage { cp, name }
    };
    let mut rows = Vec::new();
    if *ansi != 65001 {
        rows.push(Err("System"));
        rows.push(Ok(named(*ansi, "ANSI")));
        if oem != ansi && *oem != 65001 {
            rows.push(Ok(named(*oem, "OEM")));
        }
    }
    rows.push(Err("Unicode"));
    rows.extend(unicode().into_iter().map(Ok));
    let favorites = favorites();
    let standard = [*ansi, *oem, 65001, 1200, 1201];
    let rest: Vec<&CodePage> = others.iter().filter(|c| !standard.contains(&c.cp)).collect();
    let with_name = |c: &CodePage| CodePage { cp: c.cp, name: custom_name(c.cp).unwrap_or_else(|| c.name.clone()) };
    let favored: Vec<CodePage> = rest.iter().filter(|c| favorites.contains(&c.cp)).map(|c| with_name(c)).collect();
    if !favored.is_empty() {
        rows.push(Err("Favorites"));
        rows.extend(favored.into_iter().map(Ok));
    }
    if !hide_others {
        let other: Vec<CodePage> = rest.iter().filter(|c| !favorites.contains(&c.cp)).map(|c| with_name(c)).collect();
        if !other.is_empty() {
            rows.push(Err("Other"));
            rows.extend(other.into_iter().map(Ok));
        }
    }
    rows
}

/// FormatCodePageString: `1251 │ name`.
fn label(cp: &CodePage) -> String {
    format!("{:<5}\u{2502} {}", cp.cp, cp.name)
}

pub struct CodePagesMenu {
    pub menu: VMenu,
    rows: Vec<Result<CodePage, &'static str>>,
}

impl CodePagesMenu {
    pub fn new() -> Self {
        let mut menu = Self { menu: VMenu::with("Code pages", vec![]), rows: vec![] };
        menu.menu.max_height_margin = Some(4);
        menu.menu.auto_highlight = true;
        menu.fill(None);
        menu
    }

    fn fill(&mut self, select: Option<u32>) {
        let hide = settings::bool("CodePages.CPMenuMode");
        self.menu.bottom = if hide { "Ctrl+H, Del, F4" } else { "Ctrl+H, Ins, Del, F4" }.into();
        self.rows = rows(hide);
        let entries = self
            .rows
            .iter()
            .map(|r| match r {
                Ok(cp) => Entry::new(crate::dialog::escape_ampersands(&label(cp))),
                Err(title) => Entry::separator(crate::lang::tr(title)),
            })
            .collect();
        let at = select.and_then(|cp| self.rows.iter().position(|r| r.as_ref().is_ok_and(|c| c.cp == cp))).unwrap_or(0);
        self.menu.set_entries(entries, at);
    }

    pub fn current(&self) -> Option<&CodePage> {
        self.rows.get(self.menu.selected()).and_then(|r| r.as_ref().ok())
    }

    /// Ctrl+H, Ins, Del; F4 is the caller's (a dialog).
    pub fn key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        let current = self.current().map(|c| c.cp);
        match key.code {
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                settings::set_bool("CodePages.CPMenuMode", !settings::bool("CodePages.CPMenuMode"));
            }
            KeyCode::Insert | KeyCode::Delete => {
                let Some(cp) = current else { return true };
                if [65001, 1200, 1201, installed().0, installed().1].contains(&cp) {
                    return true;
                }
                let mut list = favorites();
                list.retain(|c| *c != cp);
                if key.code == KeyCode::Insert {
                    list.push(cp);
                }
                settings::set("CodePages.Favorites", Value::from(list));
            }
            _ => return false,
        }
        self.fill(current);
        true
    }

    pub fn rename(&mut self, cp: u32, name: &str) {
        let key = format!("CodePages.Names.{cp}");
        settings::set(&key, if name.is_empty() { Value::Null } else { Value::from(name) });
        self.fill(Some(cp));
    }
}

/// The viewer's "Default code page" list.
pub fn combo_items() -> Vec<(&'static str, i64)> {
    static ITEMS: OnceLock<Vec<(&'static str, i64)>> = OnceLock::new();
    ITEMS
        .get_or_init(|| {
            rows(true)
                .into_iter()
                .filter_map(Result::ok)
                .map(|cp| (&*Box::leak(label(&cp).into_boxed_str()), i64::from(cp.cp)))
                .collect()
        })
        .clone()
}
