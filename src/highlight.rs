//! Far's files highlighting and sort groups (hilight.cpp). A group is a
//! filter (`crate::filter::Filter`), a mark, eight colors (the name and the
//! mark, normal / selected / under the cursor / both; none: the panel's own)
//! and "Continue processing". The groups are in four parts: highlighting,
//! upper sort groups, lower sort groups, the lowest priority highlighting;
//! every group colors (the first that matches stops unless it continues),
//! the sort groups also order the files when "Use sort groups" is on.
//! Kept in the settings as `Highlight`; Far's six default groups otherwise.

use std::sync::RwLock;
use std::time::SystemTime;

use serde_json::{json, Value};

use crate::filter::Filter;
use crate::listing::Entry;
use crate::masks::Masks;

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub filter: Filter,
    pub mark: String,
    pub inherit_mark: bool,
    /// The name's colors, then the mark's: normal, selected, cursor, selected under the cursor.
    pub colors: [Option<(u8, u8)>; 8],
    pub continue_processing: bool,
    /// 0 highlighting, 1 upper sort group, 2 lower sort group, 3 lowest priority highlighting.
    pub section: u8,
}

impl Group {
    fn of(mask: &str, attribute: u32, normal: u8, cursor: u8) -> Self {
        let mut filter = Filter { mask_on: !mask.is_empty(), mask: mask.into(), attr_on: attribute != 0, ..Filter::default() };
        filter.attributes = crate::filter::ATTRIBUTES.iter().map(|(a, _)| if *a == attribute { 1 } else { 2 }).collect();
        let mut colors = [None; 8];
        colors[0] = Some((normal, 0));
        colors[2] = Some((cursor, 0));
        Self { filter, mark: String::new(), inherit_mark: true, colors, continue_processing: false, section: 0 }
    }
}

/// SetHighlighting's defaults.
pub fn defaults() -> Vec<Group> {
    use crate::palette::*;
    vec![
        Group::of("", 0x2, CYAN, DARK_GRAY),
        Group::of("", 0x4, CYAN, DARK_GRAY),
        Group::of("", 0x10, WHITE, WHITE),
        Group::of("<exec>", 0, LIGHT_GREEN, LIGHT_GREEN),
        Group::of("<arc>", 0, LIGHT_MAGENTA, LIGHT_MAGENTA),
        Group::of("<temp>", 0, BROWN, BROWN),
    ]
}

fn to_json(group: &Group) -> Value {
    let colors: Vec<Value> = group.colors.iter().map(|c| c.map_or(Value::Null, |(f, b)| Value::from(u64::from(f) | (u64::from(b) << 4)))).collect();
    json!({
        "Filter": group.filter.to_line(),
        "Mark": group.mark,
        "InheritMark": group.inherit_mark,
        "Colors": colors,
        "ContinueProcessing": group.continue_processing,
        "Section": group.section,
    })
}

fn from_json(value: &Value) -> Option<Group> {
    let filter = Filter::from_fields(&crate::profile::fields(value.get("Filter")?.as_str()?))?;
    let mut colors = [None; 8];
    if let Some(Value::Array(list)) = value.get("Colors") {
        for (slot, v) in colors.iter_mut().zip(list) {
            *slot = v.as_u64().map(|a| ((a & 15) as u8, ((a >> 4) & 15) as u8));
        }
    }
    Some(Group {
        filter,
        mark: value.get("Mark").and_then(Value::as_str).unwrap_or("").to_string(),
        inherit_mark: value.get("InheritMark").and_then(Value::as_bool).unwrap_or(true),
        colors,
        continue_processing: value.get("ContinueProcessing").and_then(Value::as_bool).unwrap_or(false),
        section: value.get("Section").and_then(Value::as_u64).unwrap_or(0).min(3) as u8,
    })
}

pub fn load() -> Vec<Group> {
    match crate::settings::value("Highlight") {
        Some(Value::Array(list)) => list.iter().filter_map(from_json).collect(),
        _ => defaults(),
    }
}

pub fn save(groups: &[Group]) {
    crate::settings::set("Highlight", Value::Array(groups.iter().map(to_json).collect()));
    reload();
}

/// The groups with their masks compiled, for drawing.
struct Compiled {
    group: Group,
    /// The filter without its mask, and the mask.
    rest: Filter,
    masks: Option<Masks>,
}

static CACHE: RwLock<Option<Vec<Compiled>>> = RwLock::new(None);

pub fn reload() {
    let compiled = load()
        .into_iter()
        .map(|group| {
            let masks = group.filter.mask_on.then(|| Masks::new(&group.filter.mask)).flatten();
            let rest = Filter { mask_on: false, ..group.filter.clone() };
            Compiled { group, rest, masks }
        })
        .collect();
    *CACHE.write().unwrap_or_else(|e| e.into_inner()) = Some(compiled);
}

fn with<R>(f: impl FnOnce(&[Compiled]) -> R) -> R {
    if CACHE.read().unwrap_or_else(|e| e.into_inner()).is_none() {
        reload();
    }
    let guard = CACHE.read().unwrap_or_else(|e| e.into_inner());
    f(guard.as_deref().unwrap_or(&[]))
}

fn matches(c: &Compiled, entry: &Entry, now: SystemTime) -> bool {
    (!c.group.filter.mask_on || c.masks.as_ref().is_some_and(|m| m.check(&entry.name))) && c.rest.matches(entry, now)
}

/// GetHiColor: the name's four colors (none: the panel's) and the mark.
pub fn colors(entry: &Entry) -> ([Option<(u8, u8)>; 4], String) {
    let now = SystemTime::now();
    with(|groups| {
        let mut colors = [None; 4];
        let mut mark = String::new();
        for c in groups {
            if !matches(c, entry, now) {
                continue;
            }
            for (slot, color) in colors.iter_mut().zip(&c.group.colors[..4]) {
                if color.is_some() {
                    *slot = *color;
                }
            }
            if !c.group.inherit_mark {
                mark = c.group.mark.clone();
            }
            if !c.group.continue_processing {
                break;
            }
        }
        (colors, mark)
    })
}

/// GetGroup: the sort group (upper groups first, the files of none in the
/// middle, the lower groups after).
pub fn sort_group(entry: &Entry) -> usize {
    const DEFAULT: usize = 1000;
    let now = SystemTime::now();
    with(|groups| {
        let (mut upper, mut lower) = (0, 0);
        for c in groups {
            match c.group.section {
                1 => {
                    if matches(c, entry, now) {
                        return upper;
                    }
                    upper += 1;
                }
                2 => {
                    if matches(c, entry, now) {
                        return DEFAULT + 1 + lower;
                    }
                    lower += 1;
                }
                _ => {}
            }
        }
        DEFAULT
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_default_groups() {
        let entry = |name: &str, attributes: u32, is_dir: bool| Entry { name: name.into(), attributes, is_dir, ..Entry::default() };
        let groups = defaults();
        assert_eq!(groups.len(), 6);
        let back: Vec<Group> = groups.iter().filter_map(|g| from_json(&to_json(g))).collect();
        assert_eq!(back, groups, "kept and read back");
        assert_eq!(colors(&entry("a.zip", 0x20, false)).0[0], Some((crate::palette::LIGHT_MAGENTA, 0)));
        assert_eq!(colors(&entry("src", 0x10, true)).0[0], Some((crate::palette::WHITE, 0)));
        assert_eq!(colors(&entry(".git", 0x12, true)).0[2], Some((crate::palette::DARK_GRAY, 0)), "hidden first");
        assert_eq!(colors(&entry("a.txt", 0x20, false)).0, [None; 4]);
        assert_eq!(sort_group(&entry("a.txt", 0x20, false)), 1000);
    }
}
