//! What a panel lists and how Far Manager 3 shows it: reading a folder with
//! its attributes, sorting, the default highlighting groups and Far's own
//! formats for names, sizes and dates (from Far's filelist.cpp, panelmix.cpp
//! and strmix.cpp).

use std::path::Path;
use std::time::SystemTime;

use crossterm::style::Color;

pub const TAG_MOUNT_POINT: u32 = 0xA000_0003;
pub const TAG_SYMLINK: u32 = 0xA000_000C;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub changed: Option<SystemTime>,
    /// Space taken on the disk.
    pub allocated: u64,
    /// Hidden or system (dimmed, as Far's first highlighting groups do).
    pub hidden: bool,
    /// The reparse tag of a junction, symlink, cloud placeholder...
    pub reparse: Option<u32>,
    /// Place in the folder's own order (Far's "Unsorted").
    pub position: usize,
    /// Read only when a sort mode needs them (as Far does).
    pub owner: Option<String>,
    pub links: Option<u32>,
    /// Number and total size of the data streams.
    pub streams: Option<(u32, u64)>,
    pub description: Option<String>,
}

impl Entry {
    pub fn up(modified: Option<SystemTime>) -> Self {
        Self { name: "..".into(), is_dir: true, modified, ..Self::default() }
    }

    pub fn is_up(&self) -> bool {
        self.name == ".."
    }
}

/// FILETIME ticks (100 ns from 1601) as a time.
#[cfg(windows)]
fn file_time(ticks: i64) -> Option<SystemTime> {
    // the Unix epoch is 11644473600 s after 1601
    let since_unix = u64::try_from(ticks).ok()?.checked_sub(116_444_736_000_000_000)?;
    Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_nanos(since_unix.saturating_mul(100)))
}

/// The entries of `dir` (without `.` and `..`), in the folder's own order.
/// Read as Far reads them: one directory handle, FILE_FULL_DIR_INFO records
/// (all four times, the allocated size and the reparse tag at once).
#[cfg(windows)]
pub fn read(dir: &Path) -> Vec<Entry> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FULL_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileFullDirectoryInfo, GetFileInformationByHandleEx, OPEN_EXISTING,
    };

    let path: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `path` is NUL-terminated.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut entries = Vec::new();
    let mut buffer = vec![0u64; 64 * 1024 / 8];
    loop {
        let size = (buffer.len() * 8) as u32;
        // SAFETY: the buffer is 8-byte aligned and `size` bytes long.
        if unsafe { GetFileInformationByHandleEx(handle, FileFullDirectoryInfo, buffer.as_mut_ptr().cast(), size) } == 0 {
            break;
        }
        let base = buffer.as_ptr().cast::<u8>();
        let mut offset = 0usize;
        loop {
            // SAFETY: the system wrote a chain of records into the buffer; each
            // starts at an 8-byte aligned offset given by the previous one.
            let info = unsafe { &*base.add(offset).cast::<FILE_FULL_DIR_INFO>() };
            let len = info.FileNameLength as usize / 2;
            // SAFETY: the name runs `FileNameLength` bytes from `FileName`.
            let name16 = unsafe { std::slice::from_raw_parts(info.FileName.as_ptr(), len) };
            let name = String::from_utf16_lossy(name16);
            if name != "." && name != ".." {
                let attrs = info.FileAttributes;
                entries.push(Entry {
                    name,
                    is_dir: attrs & FILE_ATTRIBUTE_DIRECTORY != 0,
                    size: info.EndOfFile as u64,
                    modified: file_time(info.LastWriteTime),
                    created: file_time(info.CreationTime),
                    accessed: file_time(info.LastAccessTime),
                    changed: file_time(info.ChangeTime),
                    allocated: info.AllocationSize as u64,
                    hidden: attrs & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0,
                    // for a reparse point the EA size field holds the tag
                    reparse: (attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0).then_some(info.EaSize),
                    position: entries.len(),
                    ..Entry::default()
                });
            }
            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }
    }
    // SAFETY: opened above, closed once.
    unsafe { CloseHandle(handle) };
    entries
}

/// The entries of `dir`, unsorted; a dot name counts as hidden.
#[cfg(not(windows))]
pub fn read(dir: &Path) -> Vec<Entry> {
    use std::os::unix::fs::MetadataExt;
    let time = |secs: i64, nanos: i64| {
        u64::try_from(secs).ok().map(|s| SystemTime::UNIX_EPOCH + std::time::Duration::new(s, nanos as u32))
    };
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .enumerate()
        .map(|(position, item)| {
            let name = item.file_name().to_string_lossy().into_owned();
            let link = item.file_type().is_ok_and(|t| t.is_symlink());
            let meta = std::fs::metadata(item.path()).ok().or_else(|| item.metadata().ok());
            Entry {
                hidden: name.starts_with('.'),
                name,
                is_dir: meta.as_ref().is_some_and(|m| m.is_dir()),
                size: meta.as_ref().map_or(0, |m| m.len()),
                modified: meta.as_ref().and_then(|m| m.modified().ok()),
                created: meta.as_ref().and_then(|m| m.created().ok()),
                accessed: meta.as_ref().and_then(|m| m.accessed().ok()),
                changed: meta.as_ref().and_then(|m| time(m.ctime(), m.ctime_nsec())),
                allocated: meta.as_ref().map_or(0, |m| m.blocks() * 512),
                reparse: link.then_some(TAG_SYMLINK),
                position,
                ..Entry::default()
            }
        })
        .collect()
}

/// Far's `name_ext`: the extension is from the last dot, a leading dot included
/// (so `.gitconfig` is all extension).
pub fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) => (&name[..dot], &name[dot..]),
        None => (name, ""),
    }
}

/// The extension without its dot (`""` when there is none).
pub fn extension(name: &str) -> &str {
    split_ext(name).1.strip_prefix('.').unwrap_or("")
}

/// Colors of one entry: (normal, under the cursor) foregrounds.
pub fn highlight(entry: &Entry) -> (Color, Color) {
    const ARC: &str = "*.zip,*.rar,*.[7bgxl]z,*.[bg]zip,*.tar,*.t[agbxl]z,*.z,*.ar[cj],*.r[0-9][0-9],*.a[0-9][0-9],*.bz2,*.cab,*.jar,*.lha,*.lzh,*.ha,*.ac[bei],*.pa[ck],*.rk,*.cpio,*.rpm,*.zoo,*.hqx,*.sit,*.ice,*.uc2,*.ain,*.imp,*.777,*.ufa,*.boa,*.bs[2a],*.sea,*.[ah]pk,*.ddi,*.x2,*.rkv,*.[lw]sz,*.h[ay]p,*.lim,*.sqz,*.chz,*.aa[br],*.zst";
    const TEMP: &str = "*.bak,*.tmp";
    if entry.hidden {
        (Color::DarkCyan, Color::DarkGrey)
    } else if entry.is_dir {
        (Color::White, Color::White)
    } else if is_executable(&entry.name) {
        (Color::Green, Color::Green)
    } else if matches_any(ARC, &entry.name) {
        (Color::Magenta, Color::Magenta)
    } else if matches_any(TEMP, &entry.name) {
        (Color::DarkYellow, Color::DarkYellow)
    } else {
        (Color::Cyan, Color::Black)
    }
}

/// Far's `<exec>` group: `*.exe,*.cmd,*.bat,*.com,%PATHEXT%`.
fn is_executable(name: &str) -> bool {
    let ext = format!(".{}", extension(name)).to_lowercase();
    if [".exe", ".cmd", ".bat", ".com"].contains(&ext.as_str()) {
        return true;
    }
    cfg!(windows)
        && std::env::var("PATHEXT").is_ok_and(|list| list.split(';').any(|e| !e.is_empty() && e.eq_ignore_ascii_case(&ext)))
}

fn matches_any(masks: &str, name: &str) -> bool {
    let name: Vec<char> = name.to_lowercase().chars().collect();
    masks.split(',').any(|mask| glob(&mask.chars().collect::<Vec<_>>(), &name))
}

/// A file mask: `*`, `?` and `[a-z]` classes; `name` is lower case.
fn glob(mask: &[char], name: &[char]) -> bool {
    match mask.first() {
        None => name.is_empty(),
        Some('*') => (0..=name.len()).any(|skip| glob(&mask[1..], &name[skip..])),
        Some('?') => !name.is_empty() && glob(&mask[1..], &name[1..]),
        Some('[') => {
            let Some(close) = mask.iter().position(|&c| c == ']') else { return false };
            let Some(&c) = name.first() else { return false };
            let set = &mask[1..close];
            let mut hit = false;
            let mut i = 0;
            while i < set.len() {
                if i + 2 < set.len() && set[i + 1] == '-' {
                    hit |= (set[i]..=set[i + 2]).contains(&c);
                    i += 3;
                } else {
                    hit |= set[i] == c;
                    i += 1;
                }
            }
            hit && glob(&mask[close + 1..], &name[1..])
        }
        Some(&m) => name.first() == Some(&m) && glob(&mask[1..], &name[1..]),
    }
}

pub fn chars(s: &str) -> usize {
    s.chars().count()
}

fn take(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Far's `fit_to_center`.
pub fn center(s: &str, width: usize) -> String {
    let len = chars(s);
    if len >= width {
        return take(s, width);
    }
    let left = (width - len) / 2;
    format!("{}{s}{}", " ".repeat(left), " ".repeat(width - len - left))
}

/// Far's `fit_to_right`: padded on the left, cut on the right.
pub fn right(s: &str, width: usize) -> String {
    let len = chars(s);
    if len >= width { take(s, width) } else { format!("{}{s}", " ".repeat(width - len)) }
}

/// Far's `ConvertName` for the list: the name in `width` cells, its
/// extension aligned to the right edge when `align` is on (files only).
/// The flag says the name did not fit.
pub fn list_name(name: &str, width: usize, align: bool) -> (String, bool) {
    let len = chars(name);
    if align && len <= width {
        let (base, ext) = split_ext(name);
        let ext = &ext[ext.len().min(1)..];
        let usable = !ext.is_empty() && chars(ext) + 1 != len && (len > 2 || !name.starts_with('.')) && !ext.contains(' ');
        if chars(ext) + 1 > 1 && usable {
            let (base_len, ext_len) = (chars(base), chars(ext));
            let mut between = if base_len + ext_len.max(3) <= width { width - base_len - ext_len.max(3) } else { 0 };
            if between == 0 && base_len + ext_len <= width {
                between = width - base_len - ext_len;
            }
            let after = width.saturating_sub(base_len + between + ext_len);
            return (format!("{base}{}{ext}{}", " ".repeat(between), " ".repeat(after)), false);
        }
    }
    let shown = take(name, width);
    let pad = width - chars(&shown);
    (format!("{shown}{}", " ".repeat(pad)), len > width)
}

/// The name in the status line: left aligned, or its tail when too long.
pub fn status_name(name: &str, width: usize) -> String {
    let len = chars(name);
    if len > width {
        name.chars().skip(len - width).collect()
    } else {
        format!("{name}{}", " ".repeat(width - len))
    }
}

/// Far's Size column for a folder or a reparse point, else the size itself.
pub fn size_column(entry: &Entry, width: usize) -> String {
    let have_size = !entry.is_dir && (entry.reparse.is_none() || entry.size > 0);
    if have_size {
        return file_size(entry.size, width);
    }
    let label = if entry.is_up() {
        center("Up", 6)
    } else if let Some(tag) = entry.reparse {
        reparse_name(tag)
    } else {
        center("Folder", 6)
    };
    let label = if chars(&label) + 2 <= width { format!("<{label}>") } else { label };
    right(&label, width)
}

fn reparse_name(tag: u32) -> String {
    match tag {
        TAG_MOUNT_POINT => "Junction".into(),
        TAG_SYMLINK => "Symlink".into(),
        0x8000_001B => "APPEXECLINK".into(),
        tag if tag & 0xFFFF_0FFF == 0x9000_001A => match (tag >> 12) & 0xF {
            0 => "CLOUD".into(),
            n => format!("CLOUD_{n:X}"),
        },
        _ => "?Symlink?".into(),
    }
}

const UNITS: [char; 7] = ['B', 'K', 'M', 'G', 'T', 'P', 'E'];

/// Far's `FileSizeToStr` for a size column: the number when it fits, else
/// in K, M, G... (rounded), right aligned.
pub fn file_size(size: u64, width: usize) -> String {
    let mut text = size.to_string();
    let mut unit = 0;
    if chars(&text) > width {
        let max = width.saturating_sub(2);
        while unit < UNITS.len() - 1 && chars(&text) > max {
            let divider = 1u128 << (10 * (unit + 1));
            let units = (size as u128 + divider / 2) / divider;
            if units == 0 {
                break;
            }
            unit += 1;
            text = units.to_string();
        }
        text = format!("{text} {}", UNITS[unit]);
    }
    fit_number(text, width)
}

fn fit_number(text: String, width: usize) -> String {
    let len = chars(&text);
    if len <= width {
        return format!("{}{text}", " ".repeat(width - len));
    }
    let tail: String = text.chars().skip(len - (width - 1)).collect();
    format!("\u{2026}{tail}")
}

/// Far's float size for the totals: `25,8 M`, `4,77 M`, `97 B`.
pub fn float_size(size: u64) -> String {
    let unit = if size == 0 { 0 } else { ((63 - size.leading_zeros()) / 10) as usize }.min(UNITS.len() - 1);
    if unit == 0 {
        return format!("{size} B");
    }
    let divider = 1u64 << (10 * unit);
    let integral = size / divider;
    let fraction = (size % divider) as f64 / divider as f64;
    let digits: u32 = if integral < 10 { 2 } else if integral < 100 { 1 } else { 0 };
    let multiplier = 10u64.pow(digits);
    let rounded = (fraction * multiplier as f64).round() as u64;
    let (whole, part) = if rounded == multiplier { (integral + 1, 0) } else { (integral, rounded) };
    let whole_len = if whole < 10 { 1 } else if whole < 100 { 2 } else { 3 };
    let digits = digits.min(3 - whole_len);
    let text = if digits > 0 {
        format!("{whole},{part:0width$}", width = digits as usize)
    } else {
        format!("{}", (integral as f64 + fraction).round() as u64)
    };
    format!("{text} {}", UNITS[unit])
}

/// Digits in groups of three with a no-break space, as Far shows bytes.
pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('\u{a0}');
        }
        out.push(c);
    }
    out
}

/// `dd.mm.yy` and `hh:mm` in local time.
pub fn date_time(time: Option<SystemTime>) -> (String, String) {
    time.map(|t| {
        let local = chrono::DateTime::<chrono::Local>::from(t);
        (local.format("%d.%m.%y").to_string(), local.format("%H:%M").to_string())
    })
    .unwrap_or_default()
}

/// Far's `truncate_path`: the root, an ellipsis and the tail.
pub fn truncate_path(path: &str, cells: usize) -> String {
    if chars(path) <= cells {
        return path.to_string();
    }
    if cells == 0 {
        return String::new();
    }
    let root = root_len(path);
    let prefix: String = path.chars().take(root.min(cells - 1)).collect();
    let room = cells - 1 - chars(&prefix);
    let tail: String = path.chars().skip(chars(path) - room).collect();
    format!("{prefix}\u{2026}{tail}")
}

/// Length of `C:\`, `\\server\share\` or `/` at the start of a path.
fn root_len(path: &str) -> usize {
    let b: Vec<char> = path.chars().collect();
    if b.len() >= 3 && b[1] == ':' && (b[2] == '\\' || b[2] == '/') {
        return 3;
    }
    if path.starts_with("\\\\") {
        let mut slashes = 0;
        for (i, c) in b.iter().enumerate().skip(2) {
            if *c == '\\' {
                slashes += 1;
                if slashes == 2 {
                    return i + 1;
                }
            }
        }
        return b.len();
    }
    usize::from(path.starts_with('/'))
}

/// Cut to `cells`, the last one an ellipsis.
pub fn truncate_right(s: &str, cells: usize) -> String {
    if chars(s) <= cells {
        return s.to_string();
    }
    if cells == 0 {
        return String::new();
    }
    format!("{}\u{2026}", take(s, cells - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> Entry {
        Entry { name: name.into(), ..Entry::default() }
    }

    #[test]
    fn sizes_are_shown_as_far_shows_them() {
        assert_eq!(file_size(97, 6), "    97");
        assert_eq!(file_size(999_999, 6), "999999");
        assert_eq!(file_size(8_279_982, 6), "8086 K");
        assert_eq!(file_size(5_000_000, 6), "4883 K");
        assert_eq!(file_size(37_748_736, 6), "  36 M");
        assert_eq!(file_size(5_000_000_000, 6), "4768 M");
        assert_eq!(file_size(50_000_000_000, 6), "  47 G");
        assert_eq!(float_size(5_000_010), "4,77 M");
        assert_eq!(float_size(27_053_261), "25,8 M");
        assert_eq!(float_size(97), "97 B");
        assert_eq!(float_size(0), "0 B");
        assert_eq!(float_size(300 * 1024 * 1024), "300 M");
        assert_eq!(group_digits(5_000_010), "5\u{a0}000\u{a0}010");
    }

    #[test]
    fn folders_junctions_and_up_have_labels() {
        let mut dir = file("x");
        dir.is_dir = true;
        assert_eq!(size_column(&dir, 6), "Folder");
        assert_eq!(size_column(&dir, 10), "  <Folder>");
        assert_eq!(size_column(&Entry::up(None), 6), "  Up  ");
        assert_eq!(size_column(&Entry::up(None), 10), "  <  Up  >");
        dir.reparse = Some(TAG_MOUNT_POINT);
        assert_eq!(size_column(&dir, 6), "Juncti");
        assert_eq!(size_column(&dir, 10), "<Junction>");
        dir.reparse = Some(0x9000_701A);
        assert_eq!(size_column(&dir, 6), "CLOUD_");
    }

    #[test]
    fn extensions_are_aligned_like_far() {
        assert_eq!(list_name("old.bak", 10, true), ("old    bak".into(), false));
        assert_eq!(list_name("a.cs", 10, true), ("a      cs ".into(), false));
        assert_eq!(list_name(".claude.json.backup", 19, true), (".claude.json backup".into(), false));
        assert_eq!(list_name(".gitconfig", 12, true), (".gitconfig  ".into(), false));
        assert_eq!(list_name("README", 8, true), ("README  ".into(), false));
        assert_eq!(list_name("NTUSER.DAT{4359efd9-3ecc}.TM.blf", 19, true), ("NTUSER.DAT{4359efd9".into(), true));
        assert_eq!(list_name("ntuser.dat.LOG1", 19, false), ("ntuser.dat.LOG1    ".into(), false));
    }

    #[test]
    fn masks_and_paths() {
        assert!(matches_any("*.r[0-9][0-9]", "x.r07"));
        assert!(!matches_any("*.r[0-9][0-9]", "x.rar7"));
        assert_eq!(highlight(&file("a.7z")).0, Color::Magenta);
        assert_eq!(highlight(&file("a.bak")).0, Color::DarkYellow);
        assert_eq!(truncate_path("C:\\Users\\dnk\\very\\long", 13), "C:\\\u{2026}very\\long");
    }
}
