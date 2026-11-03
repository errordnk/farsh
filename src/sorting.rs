//! Far Manager 3's sort modes (filelist.cpp `SortModes`, `list_less`): each
//! mode is a list of layers tried in turn, the first one following the
//! panel's direction, the others their own; `..` stays on top, folders first
//! and marked first when asked.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::Path;

use crate::listing::{Entry, split_ext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortMode {
    Unsorted,
    Name,
    Extension,
    Write,
    Creation,
    Access,
    Size,
    Descriptions,
    Owner,
    Allocated,
    Links,
    Streams,
    StreamsSize,
    NameOnly,
    Change,
}

use SortMode::*;

impl SortMode {
    /// The order of Far's "Sort by" menu.
    pub const MENU: [SortMode; 15] = [
        Name, NameOnly, Extension, Write, Size, Unsorted, Creation, Access, Change, Descriptions, Owner, Allocated, Links,
        Streams, StreamsSize,
    ];

    /// The menu label, `&` before the hotkey (Far's English strings).
    pub fn label(self) -> &'static str {
        match self {
            Name => "&Name",
            NameOnly => "Name only",
            Extension => "E&xtension",
            Write => "&Write time",
            Size => "&Size",
            Unsorted => "&Unsorted",
            Creation => "&Creation time",
            Access => "&Access time",
            Change => "Chan&ge time",
            Descriptions => "&Descriptions",
            Owner => "&Owner",
            Allocated => "Allocated size",
            Links => "Number of &hard links",
            Streams => "Number of st&reams",
            StreamsSize => "Si&ze of streams",
        }
    }

    /// Its Ctrl+F key.
    pub fn key(self) -> Option<u8> {
        match self {
            Name => Some(3),
            Extension => Some(4),
            Write => Some(5),
            Size => Some(6),
            Unsorted => Some(7),
            Creation => Some(8),
            Access => Some(9),
            Descriptions => Some(10),
            Owner => Some(11),
            _ => None,
        }
    }

    pub fn by_key(n: u8) -> Option<SortMode> {
        Self::MENU.into_iter().find(|m| m.key() == Some(n))
    }

    /// Far's default layers: (criterion, descending).
    fn layers(self) -> &'static [(SortMode, bool)] {
        match self {
            Unsorted => &[(Unsorted, false)],
            Name => &[(Name, false), (Unsorted, false)],
            Extension => &[(Extension, false), (NameOnly, false), (Unsorted, false)],
            NameOnly => &[(NameOnly, false), (Extension, false), (Unsorted, false)],
            Descriptions => &[(Descriptions, false), (Name, false), (Unsorted, false)],
            Owner => &[(Owner, false), (Name, false), (Unsorted, false)],
            Write => &[(Write, true), (Name, false), (Unsorted, false)],
            Creation => &[(Creation, true), (Name, false), (Unsorted, false)],
            Access => &[(Access, true), (Name, false), (Unsorted, false)],
            Change => &[(Change, true), (Name, false), (Unsorted, false)],
            Size => &[(Size, true), (Name, false), (Unsorted, false)],
            Allocated => &[(Allocated, true), (Name, false), (Unsorted, false)],
            Links => &[(Links, true), (Name, false), (Unsorted, false)],
            Streams => &[(Streams, true), (Name, false), (Unsorted, false)],
            StreamsSize => &[(StreamsSize, true), (Name, false), (Unsorted, false)],
        }
    }

    /// Whether choosing it starts descending (times and sizes).
    pub fn descending_by_default(self) -> bool {
        self.layers()[0].1
    }

    /// The letter in the panel's corner: the label's hotkey (or first
    /// letter), upper case when reversed.
    pub fn indicator(self, reverse: bool) -> char {
        let label = self.label();
        let c = label.find('&').map_or(label.chars().next(), |at| label[at + 1..].chars().next()).unwrap_or(' ');
        if reverse { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }
    }
}

/// A panel's sort settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sorting {
    pub mode: SortMode,
    pub reverse: bool,
    pub directories_first: bool,
    pub selected_first: bool,
    /// Far's sort groups: with the default highlighting every file is in the
    /// same group, so this only shows as a check in the menu.
    pub groups: bool,
}

impl Default for Sorting {
    fn default() -> Self {
        Self { mode: Extension, reverse: false, directories_first: true, selected_first: false, groups: false }
    }
}

impl Sorting {
    /// Far's `SetSortMode`: the same mode again flips, another starts in its
    /// default direction.
    pub fn choose(&mut self, mode: SortMode) {
        self.reverse = if self.mode == mode { !self.reverse } else { mode.descending_by_default() };
        self.mode = mode;
    }
}

/// Sorts a folder's entries (`..`, if there, stays first).
pub fn sort(entries: &mut [Entry], sorting: &Sorting, marked: &HashSet<String>, dir: &Path) {
    let needs: Vec<SortMode> = sorting.mode.layers().iter().map(|l| l.0).collect();
    fill(entries, &needs, dir);
    entries.sort_by(|a, b| compare_entries(a, b, sorting, marked));
}

fn compare_entries(a: &Entry, b: &Entry, sorting: &Sorting, marked: &HashSet<String>) -> Ordering {
    match (a.is_up(), b.is_up()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    if sorting.directories_first && a.is_dir != b.is_dir {
        return b.is_dir.cmp(&a.is_dir);
    }
    if sorting.selected_first {
        let (sa, sb) = (marked.contains(&a.name), marked.contains(&b.name));
        if sa != sb {
            return sb.cmp(&sa);
        }
    }
    for &(layer, descending) in sorting.mode.layers() {
        let reverse = if layer == sorting.mode { sorting.reverse } else { descending };
        let order = compare(layer, a, b);
        let order = if reverse { order.reverse() } else { order };
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

/// Name and extension as Far sorts them: a folder's name is all name.
fn name_ext(entry: &Entry) -> (&str, &str) {
    if entry.is_dir { (&entry.name, "") } else { split_ext(&entry.name) }
}

const UNKNOWN: u64 = u64::MAX;

fn compare(mode: SortMode, a: &Entry, b: &Entry) -> Ordering {
    match mode {
        Unsorted => a.position.cmp(&b.position),
        Name => compare_text(&a.name, &b.name),
        NameOnly => compare_text(name_ext(a).0, name_ext(b).0),
        Extension => compare_text(name_ext(a).1, name_ext(b).1),
        Write => a.modified.cmp(&b.modified),
        Creation => a.created.cmp(&b.created),
        Access => a.accessed.cmp(&b.accessed),
        Change => a.changed.cmp(&b.changed),
        Size => a.size.cmp(&b.size),
        Allocated => a.allocated.cmp(&b.allocated),
        Descriptions => compare_text(a.description.as_deref().unwrap_or(""), b.description.as_deref().unwrap_or("")),
        Owner => compare_text(a.owner.as_deref().unwrap_or(""), b.owner.as_deref().unwrap_or("")),
        Links => a.links.map_or(UNKNOWN, u64::from).cmp(&b.links.map_or(UNKNOWN, u64::from)),
        Streams => streams(a).0.cmp(&streams(b).0),
        StreamsSize => streams(a).1.cmp(&streams(b).1),
    }
}

fn streams(entry: &Entry) -> (u64, u64) {
    entry.streams.map_or((UNKNOWN, UNKNOWN), |(n, size)| (u64::from(n), size))
}

/// Far's default string order: the user's locale, case ignored, digits as
/// numbers (`CompareString` with SORT_STRINGSORT | LINGUISTIC_IGNORECASE |
/// SORT_DIGITSASNUMBERS).
#[cfg(windows)]
pub fn compare_text(a: &str, b: &str) -> Ordering {
    use windows_sys::Win32::Globalization::{
        CompareStringW, LINGUISTIC_IGNORECASE, SORT_DIGITSASNUMBERS, SORT_STRINGSORT,
    };
    const LOCALE_USER_DEFAULT: u32 = 0x0400;
    let a16: Vec<u16> = a.encode_utf16().collect();
    let b16: Vec<u16> = b.encode_utf16().collect();
    // SAFETY: both slices are valid for their lengths.
    let result = unsafe {
        CompareStringW(
            LOCALE_USER_DEFAULT,
            SORT_STRINGSORT | LINGUISTIC_IGNORECASE | SORT_DIGITSASNUMBERS,
            a16.as_ptr(),
            a16.len() as i32,
            b16.as_ptr(),
            b16.len() as i32,
        )
    };
    match result {
        1 => Ordering::Less,
        2 => Ordering::Equal,
        3 => Ordering::Greater,
        _ => natural(a, b),
    }
}

#[cfg(not(windows))]
pub fn compare_text(a: &str, b: &str) -> Ordering {
    natural(a, b)
}

/// Case ignored, runs of digits compared as numbers.
fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = a.next_if(char::is_ascii_digit) {
                    na.push(c);
                }
                let mut nb = String::new();
                while let Some(c) = b.next_if(char::is_ascii_digit) {
                    nb.push(c);
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let order = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Reads what the layers need and the folder listing does not carry.
fn fill(entries: &mut [Entry], needs: &[SortMode], dir: &Path) {
    if needs.contains(&Descriptions) {
        let descriptions = read_descriptions(dir);
        for entry in entries.iter_mut() {
            entry.description = Some(descriptions.get(&entry.name.to_lowercase()).cloned().unwrap_or_default());
        }
    }
    for entry in entries.iter_mut().filter(|e| !e.is_up()) {
        let path = dir.join(&entry.name);
        if needs.contains(&Owner) && entry.owner.is_none() {
            entry.owner = Some(system::owner(&path).unwrap_or_default());
        }
        if needs.contains(&Links) && entry.links.is_none() {
            entry.links = if entry.is_dir { Some(1) } else { system::links(&path) };
        }
        if (needs.contains(&Streams) || needs.contains(&StreamsSize)) && entry.streams.is_none() {
            entry.streams = system::streams(&path, entry);
        }
    }
}

/// `Descript.ion` / `Files.bbs`: `name description` per line (a name with
/// spaces in quotes); lines that start with a space continue the one before.
fn read_descriptions(dir: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(listing) = std::fs::read_dir(dir) else { return map };
    let file = listing
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.eq_ignore_ascii_case("descript.ion") || n.eq_ignore_ascii_case("files.bbs"));
    let Some(text) = file.and_then(|f| std::fs::read(dir.join(f)).ok()) else { return map };
    for line in String::from_utf8_lossy(&text).lines() {
        if line.is_empty() || line.starts_with([' ', '\t']) {
            continue;
        }
        let (name, rest) = match line.strip_prefix('"') {
            Some(quoted) => quoted.split_once('"').unwrap_or((quoted, "")),
            None => line.split_once([' ', '\t']).unwrap_or((line, "")),
        };
        map.insert(name.to_lowercase(), rest.trim_start().to_string());
    }
    map
}

#[cfg(windows)]
mod system {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{LookupAccountSidW, OWNER_SECURITY_INFORMATION, PSID};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FindClose, FindFirstStreamW, FindNextStreamW,
        FindStreamInfoStandard, GetFileInformationByHandle, OPEN_EXISTING, WIN32_FIND_STREAM_DATA,
    };

    use crate::listing::Entry;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// `DOMAIN\name` of the file's owner.
    pub fn owner(path: &Path) -> Option<String> {
        let path = wide(path);
        let mut sid: PSID = std::ptr::null_mut();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: out pointers are valid; the descriptor is freed below.
        let error = unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut sid,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if error != 0 {
            return None;
        }
        let mut name = [0u16; 256];
        let mut domain = [0u16; 256];
        let (mut name_len, mut domain_len, mut kind) = (256u32, 256u32, 0);
        // SAFETY: `sid` points into `descriptor`, still alive; buffers as sized.
        let ok = unsafe {
            LookupAccountSidW(
                std::ptr::null(),
                sid,
                name.as_mut_ptr(),
                &mut name_len,
                domain.as_mut_ptr(),
                &mut domain_len,
                &mut kind,
            )
        };
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(descriptor) };
        if ok == 0 {
            return None;
        }
        let name = String::from_utf16_lossy(&name[..name_len as usize]);
        let domain = String::from_utf16_lossy(&domain[..domain_len as usize]);
        Some(if domain.is_empty() { name } else { format!("{domain}\\{name}") })
    }

    pub fn links(path: &Path) -> Option<u32> {
        let path = wide(path);
        // SAFETY: NUL-terminated path; the handle is closed below.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: open handle, valid out struct.
        let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
        // SAFETY: opened above.
        unsafe { CloseHandle(handle) };
        (ok != 0).then_some(info.nNumberOfLinks)
    }

    /// Number and total size of the data streams (none found: unknown).
    pub fn streams(path: &Path, _entry: &Entry) -> Option<(u32, u64)> {
        let path = wide(path);
        let mut data: WIN32_FIND_STREAM_DATA = unsafe { std::mem::zeroed() };
        // SAFETY: NUL-terminated path, valid out struct; closed below.
        let handle = unsafe { FindFirstStreamW(path.as_ptr(), FindStreamInfoStandard, (&raw mut data).cast(), 0) };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let (mut count, mut size) = (0u32, 0u64);
        loop {
            count += 1;
            size += data.StreamSize as u64;
            // SAFETY: open find handle.
            if unsafe { FindNextStreamW(handle, (&raw mut data).cast()) } == 0 {
                break;
            }
        }
        // SAFETY: opened above.
        unsafe { FindClose(handle) };
        Some((count, size))
    }
}

#[cfg(not(windows))]
mod system {
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    use crate::listing::Entry;

    pub fn owner(path: &Path) -> Option<String> {
        std::fs::symlink_metadata(path).ok().map(|m| m.uid().to_string())
    }

    pub fn links(path: &Path) -> Option<u32> {
        std::fs::symlink_metadata(path).ok().map(|m| m.nlink() as u32)
    }

    pub fn streams(_path: &Path, entry: &Entry) -> Option<(u32, u64)> {
        Some((1, entry.size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, position: usize) -> Entry {
        Entry { name: name.into(), position, ..Entry::default() }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn extension_then_name_and_folders_first() {
        let mut list: Vec<Entry> =
            ["b.txt", ".gitconfig", "README", "a.bak", "c.TXT"].iter().enumerate().map(|(i, n)| entry(n, i)).collect();
        let mut folder = entry("zz", 9);
        folder.is_dir = true;
        list.push(folder);
        list.insert(0, Entry::up(None));
        sort(&mut list, &Sorting::default(), &HashSet::new(), Path::new("."));
        assert_eq!(names(&list), ["..", "zz", "README", "a.bak", ".gitconfig", "b.txt", "c.TXT"]);
    }

    #[test]
    fn digits_sort_as_numbers_and_size_starts_descending() {
        let mut list: Vec<Entry> = ["file10", "file9", "File1"].iter().enumerate().map(|(i, n)| entry(n, i)).collect();
        let mut sorting = Sorting { mode: Name, ..Sorting::default() };
        sort(&mut list, &sorting, &HashSet::new(), Path::new("."));
        assert_eq!(names(&list), ["File1", "file9", "file10"]);

        list[0].size = 5;
        list[1].size = 50;
        list[2].size = 5;
        sorting.choose(Size);
        assert!(sorting.reverse, "size starts from the biggest");
        assert_eq!(sorting.mode.indicator(sorting.reverse), 'S');
        sort(&mut list, &sorting, &HashSet::new(), Path::new("."));
        // equal sizes keep Name ascending (the second layer has its own direction)
        assert_eq!(names(&list), ["file9", "File1", "file10"]);
        sorting.choose(Size);
        assert!(!sorting.reverse, "the same mode again flips");
    }

    #[test]
    fn unsorted_keeps_the_folder_order_and_marked_can_go_first() {
        let mut list: Vec<Entry> = ["c", "a", "b"].iter().enumerate().map(|(i, n)| entry(n, i)).collect();
        let mut sorting = Sorting { mode: Unsorted, ..Sorting::default() };
        sort(&mut list, &sorting, &HashSet::new(), Path::new("."));
        assert_eq!(names(&list), ["c", "a", "b"]);
        sorting.selected_first = true;
        let marked: HashSet<String> = ["b".to_string()].into();
        sort(&mut list, &sorting, &marked, Path::new("."));
        assert_eq!(names(&list), ["b", "c", "a"]);
    }

    #[test]
    fn indicators_follow_the_hotkeys() {
        assert_eq!(Extension.indicator(false), 'x');
        assert_eq!(NameOnly.indicator(false), 'n');
        assert_eq!(Change.indicator(true), 'G');
        assert_eq!(Allocated.indicator(false), 'a');
    }
}
