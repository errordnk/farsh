//! Far's file masks (filemasks.cpp and CmpName in processname.cpp): masks
//! separated by `,` or `;` (a quoted one may hold either), `|` before the
//! masks to leave out, `*`, `?` and `[a-z]` sets, case ignored. `*.*` is
//! `*` (every name, with a dot or not); `*.` is a name without an extension.
//! Regular expressions (`/.../`) are not done.

pub struct Masks {
    include: Vec<Vec<char>>,
    exclude: Vec<Vec<char>>,
}

/// `<arc>` is the group of masks of that name (Options > Groups of file
/// masks), `%PATHEXT%` the executable extensions as masks (`*.com,*.exe...`).
fn expand_groups(text: &str, depth: u8) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else { break };
        let name = &rest[start + 1..start + end];
        out.push_str(&rest[..start]);
        match crate::optmenu::mask_groups().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some((_, value)) if depth < 4 => out.push_str(&expand_groups(&value, depth + 1)),
            _ => out.push_str(&rest[start..=start + end]),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    if out.to_uppercase().contains("%PATHEXT%") {
        let exts = std::env::var("PATHEXT").unwrap_or_default();
        let masks: Vec<String> = exts.split(';').filter(|e| !e.is_empty()).map(|e| format!("*{}", e.to_lowercase())).collect();
        let at = out.to_uppercase().find("%PATHEXT%").unwrap_or(0);
        out.replace_range(at..at + 9, &masks.join(","));
    }
    out
}

/// Splits on `,` and `;` outside quotes and brackets, trims, drops the quotes.
fn tokens(text: &str) -> Vec<Vec<char>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut bracket) = (false, false);
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '[' if !quoted => {
                bracket = true;
                current.push(c);
            }
            ']' if !quoted => {
                bracket = false;
                current.push(c);
            }
            ',' | ';' if !quoted && !bracket => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out.into_iter()
        .map(|mask| mask.trim().to_string())
        .filter(|mask| !mask.is_empty())
        .map(|mask| {
            let mask = if mask == "*.*" { "*".to_string() } else { mask };
            // `**` is `*`
            let mut chars: Vec<char> = Vec::new();
            for c in mask.chars() {
                if !(c == '*' && chars.last() == Some(&'*')) {
                    chars.push(c);
                }
            }
            chars
        })
        .collect()
}

impl Masks {
    /// None when there is nothing to match with (Far's "incorrect mask").
    pub fn new(text: &str) -> Option<Self> {
        let expanded = expand_groups(text.trim(), 0);
        let text = expanded.trim();
        if text.starts_with('/') {
            return None;
        }
        let (include, exclude) = match text.split_once('|') {
            Some((include, exclude)) => (include, exclude),
            None => (text, ""),
        };
        let mut include = tokens(include);
        let exclude = tokens(exclude);
        if include.is_empty() {
            if exclude.is_empty() {
                return None;
            }
            // "|*.bak": everything but those
            include.push(vec!['*']);
        }
        Some(Self { include, exclude })
    }

    pub fn check(&self, name: &str) -> bool {
        let name: Vec<char> = name.chars().collect();
        !self.exclude.iter().any(|m| cmp_name(m, &name)) && self.include.iter().any(|m| cmp_name(m, &name))
    }
}

fn upper(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}

/// Far's CmpName (not the legacy mode).
fn cmp_name(pattern: &[char], name: &[char]) -> bool {
    if pattern == ['*'] {
        return true;
    }
    if pattern.is_empty() || name.is_empty() {
        return false;
    }
    matches(pattern, name)
}

fn matches(mut pattern: &[char], mut s: &[char]) -> bool {
    loop {
        let Some((&pc, rest)) = pattern.split_first() else { return s.is_empty() };
        let sc = s.first().map(|&c| upper(c));
        pattern = rest;
        match pc {
            '?' => {
                if s.is_empty() {
                    return false;
                }
            }
            '*' => {
                if pattern.is_empty() {
                    return true;
                }
                if pattern[0] == '.' {
                    if pattern.len() == 2 && pattern[1] == '*' {
                        return true;
                    }
                    if !pattern.iter().any(|c| matches!(c, '*' | '?' | '[')) {
                        let dot = s.iter().rposition(|&c| c == '.');
                        if pattern.len() == 1 {
                            return dot.is_none_or(|d| d + 1 == s.len());
                        }
                        let pattern_has_dot = pattern[1..].contains(&'.');
                        if pattern_has_dot && dot.is_none() {
                            return false;
                        }
                        if !pattern_has_dot && let Some(d) = dot {
                            let ext = &s[d + 1..];
                            return ext.len() == pattern.len() - 1 && ext.iter().zip(&pattern[1..]).all(|(&a, &b)| upper(a) == upper(b));
                        }
                    }
                }
                loop {
                    if matches(pattern, s) {
                        return true;
                    }
                    if s.is_empty() {
                        return false;
                    }
                    s = &s[1..];
                }
            }
            '[' => {
                if !pattern.contains(&']') {
                    if Some('[') != sc {
                        return false;
                    }
                } else if pattern.len() > 1 && pattern[1] == ']' {
                    if s.first() != Some(&pattern[0]) {
                        return false;
                    }
                    pattern = &pattern[2..];
                } else {
                    let Some(sc) = sc else { return false };
                    let mut matched = false;
                    let mut i = 0;
                    loop {
                        let Some(&rc) = pattern.get(i) else { return false };
                        let rc = upper(rc);
                        i += 1;
                        if rc == ']' {
                            if matched {
                                break;
                            }
                            return false;
                        }
                        if matched {
                            continue;
                        }
                        // `a-z`: the character before the dash (`[` when the dash is first) and after it
                        let before = if i >= 2 { pattern[i - 2] } else { '[' };
                        if rc == '-' && before != '[' && pattern.get(i).is_some_and(|&c| c != ']') {
                            matched = upper(before) <= sc && sc <= upper(pattern[i]);
                            i += 1;
                        } else {
                            matched = sc == rc;
                        }
                    }
                    pattern = &pattern[i..];
                }
            }
            _ => {
                if Some(upper(pc)) != sc {
                    return false;
                }
            }
        }
        s = &s[1..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(mask: &str, name: &str) -> bool {
        Masks::new(mask).unwrap().check(name)
    }

    #[test]
    fn far_masks() {
        assert!(check("*.*", "readme"), "*.* is everything");
        assert!(check("*.txt", "A.TXT"));
        assert!(!check("*.txt", "a.txt.bak"));
        assert!(check("*.", "makefile") && !check("*.", "a.c"), "*. is no extension");
        assert!(check("a?c", "abc") && !check("a?c", "ac"));
        assert!(check("[a-c]*", "bob") && !check("[a-c]*", "dave"));
        assert!(check("*.rs;*.toml", "Cargo.toml"));
        assert!(check("\"a,b\"", "a,b"), "a quoted mask keeps its comma");
        assert!(check("*|*.bak", "x.rs") && !check("*|*.bak", "x.bak"));
        assert!(check("|*.bak", "x.rs"), "only exclusions: everything else");
        assert!(Masks::new("").is_none() && Masks::new(" ; ").is_none());
    }
}
