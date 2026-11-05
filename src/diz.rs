//! A folder's file descriptions (Far's DizList, dizlist.cpp): `Descript.ion`
//! or `Files.bbs` (Far's default Descriptions.ListNames), one `name
//! description` per line, a name with spaces in quotes, lines that start
//! with a space or a tab continuing the one before. Written back as Far
//! does: the records in their order, a new one at the end, one space after
//! the name (StartPos 0), CRLF, the file hidden (SetHidden), removed when
//! nothing is left in it.

use std::path::{Path, PathBuf};

const LIST_NAMES: [&str; 2] = ["Descript.ion", "Files.bbs"];

pub struct Diz {
    /// The file read, or where a new one goes.
    path: PathBuf,
    exists: bool,
    /// Name and lines (the first one is what follows the name).
    records: Vec<(String, Vec<String>)>,
    /// The file had a UTF-8 signature.
    bom: bool,
}

impl Diz {
    pub fn read(dir: &Path) -> Self {
        let found = std::fs::read_dir(dir).ok().and_then(|listing| {
            let names: Vec<String> = listing.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            list_names().iter().find_map(|wanted| names.iter().find(|n| n.eq_ignore_ascii_case(wanted)).cloned())
        });
        let first = list_names().into_iter().next().unwrap_or_else(|| LIST_NAMES[0].to_string());
        let mut diz = Self { path: dir.join(found.clone().unwrap_or(first)), exists: found.is_some(), records: Vec::new(), bom: false };
        let Some(bytes) = found.and_then(|name| std::fs::read(dir.join(name)).ok()) else { return diz };
        let text = match bytes.strip_prefix(b"\xEF\xBB\xBF") {
            Some(rest) => {
                diz.bom = true;
                String::from_utf8_lossy(rest).into_owned()
            }
            None => String::from_utf8_lossy(&bytes).into_owned(),
        };
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            if line.starts_with([' ', '\t']) {
                if let Some((_, lines)) = diz.records.last_mut() {
                    lines.push(line.to_string());
                }
                continue;
            }
            let (name, rest) = match line.strip_prefix('"') {
                Some(quoted) => quoted.split_once('"').unwrap_or((quoted, "")),
                None => line.split_once([' ', '\t']).unwrap_or((line, "")),
            };
            let rest = if line.starts_with('"') { rest.to_string() } else { format!(" {rest}") };
            diz.records.push((name.to_string(), vec![rest]));
        }
        diz
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.records.iter().position(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    /// The description of `name`, its first line without the spaces before it.
    pub fn get(&self, name: &str) -> Option<String> {
        self.find(name).map(|i| self.records[i].1.first().map(|l| l.trim_start().to_string()).unwrap_or_default())
    }

    /// All descriptions, by lower-case name.
    pub fn map(&self) -> std::collections::HashMap<String, String> {
        self.records.iter().map(|(n, lines)| (n.to_lowercase(), lines.first().map(|l| l.trim_start().to_string()).unwrap_or_default())).collect()
    }

    /// Far's DizList::Set: one line, the old continuation lines dropped.
    pub fn set(&mut self, name: &str, text: &str) {
        let line = format!(" {text}");
        match self.find(name) {
            Some(i) => self.records[i].1 = vec![line],
            None => self.records.push((name.to_string(), vec![line])),
        }
    }

    pub fn erase(&mut self, name: &str) {
        if let Some(i) = self.find(name) {
            self.records.remove(i);
        }
    }

    /// Far's DizList::Flush.
    pub fn flush(&mut self) -> Result<(), String> {
        let fail = |e: std::io::Error| format!("Cannot update the file descriptions\n{}: {e}", self.path.display());
        if self.exists && std::fs::metadata(&self.path).is_ok_and(|m| m.permissions().readonly()) {
            return Err(format!("{}\nThe file is read only", self.path.display()));
        }
        if self.records.is_empty() {
            if self.exists {
                std::fs::remove_file(&self.path).map_err(fail)?;
                self.exists = false;
            }
            return Ok(());
        }
        let mut text = String::new();
        for (name, lines) in &self.records {
            text.push_str(&if name.contains(' ') { format!("\"{name}\"") } else { name.clone() });
            for line in lines {
                text.push_str(line);
                text.push_str("\r\n");
            }
        }
        let mut bytes = Vec::new();
        if self.bom || !text.is_ascii() {
            bytes.extend_from_slice(b"\xEF\xBB\xBF");
        }
        bytes.extend_from_slice(text.as_bytes());
        // a hidden file cannot be opened for a rewrite on Windows: show it
        // first; a new list is hidden if File descriptions says so
        let hide = if self.exists { is_hidden(&self.path) } else { crate::settings::bool("Descriptions.SetHidden") };
        set_hidden(&self.path, false);
        std::fs::write(&self.path, bytes).map_err(fail)?;
        set_hidden(&self.path, hide);
        self.exists = true;
        Ok(())
    }
}

/// Options > File descriptions: the description list names.
pub fn list_names() -> Vec<String> {
    crate::settings::string("Descriptions.ListNames").split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect()
}

#[cfg(windows)]
fn is_hidden(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
    attributes != INVALID_FILE_ATTRIBUTES && attributes & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(windows))]
fn is_hidden(_: &Path) -> bool {
    false
}

#[cfg(windows)]
fn set_hidden(path: &Path, hidden: bool) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, GetFileAttributesW, INVALID_FILE_ATTRIBUTES, SetFileAttributesW};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is a NUL-terminated path that outlives both calls.
    unsafe {
        let attributes = GetFileAttributesW(wide.as_ptr());
        if attributes == INVALID_FILE_ATTRIBUTES {
            return;
        }
        let wanted = if hidden { attributes | FILE_ATTRIBUTE_HIDDEN } else { attributes & !FILE_ATTRIBUTE_HIDDEN };
        if wanted != attributes {
            SetFileAttributesW(wide.as_ptr(), wanted);
        }
    }
}

#[cfg(not(windows))]
fn set_hidden(_: &Path, _: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sets_and_writes_like_far() {
        let dir = std::env::temp_dir().join(format!("farsh-diz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Descript.ion"), "a.txt First\r\n  more of it\r\n\"b c.txt\" Second\r\n").unwrap();
        let mut diz = Diz::read(&dir);
        assert_eq!(diz.get("A.TXT").as_deref(), Some("First"));
        assert_eq!(diz.get("b c.txt").as_deref(), Some("Second"));
        diz.set("b c.txt", "Changed");
        diz.set("new.rs", "Новый");
        diz.flush().unwrap();
        let bytes = std::fs::read(dir.join("Descript.ion")).unwrap();
        assert_eq!(String::from_utf8_lossy(&bytes[3..]), "a.txt First\r\n  more of it\r\n\"b c.txt\" Changed\r\nnew.rs Новый\r\n");
        let mut diz = Diz::read(&dir);
        assert_eq!(diz.get("new.rs").as_deref(), Some("Новый"), "the signature is not part of a name");
        for name in ["a.txt", "b c.txt", "new.rs"] {
            diz.erase(name);
        }
        diz.flush().unwrap();
        assert!(!dir.join("Descript.ion").exists(), "nothing left: the file goes");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
