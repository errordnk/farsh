//! Far's Change drive (Alt+F1 / Alt+F2): the drives on Windows, the mounted
//! file systems on Linux and macOS, with their size and free space.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::grid::Grid;
use crate::palette;


// Far's menu colors: white on cyan, yellow hotkeys, the selection on black
const BG: Color = palette::bg(palette::CYAN);
const TEXT: Color = palette::fg(palette::WHITE);
const HOT: Color = palette::fg(palette::YELLOW);
const SELECTED_BG: Color = palette::bg(palette::BLACK);
const SELECTED_TEXT: Color = palette::fg(palette::WHITE);

/// How long the sizes may take: a dead network share must not hang the panel.
const SIZE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    /// Where the panel goes: `C:\` or a mount point.
    pub path: PathBuf,
    /// `C:` or the mount point, as shown.
    pub name: String,
    /// `fixed`, `network`... on Windows, the file system type elsewhere.
    pub kind: String,
    pub total: Option<u64>,
    pub free: Option<u64>,
}

/// The drives (Windows) or mounted file systems (Linux, macOS), sizes filled in
/// for those that answer in time.
pub fn list() -> Vec<Drive> {
    let mut drives = mounts();
    let (tx, rx) = mpsc::channel();
    // Far leaves a network drive's sizes out unless asked (DRIVE_SHOW_REMOTE)
    for (index, drive) in drives.iter().enumerate().filter(|(_, d)| d.kind != "network") {
        let tx = tx.clone();
        let path = drive.path.clone();
        std::thread::spawn(move || {
            let _ = tx.send((index, space(&path)));
        });
    }
    drop(tx);
    let deadline = Instant::now() + SIZE_TIMEOUT;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok((index, Some((total, free)))) => {
                drives[index].total = Some(total);
                drives[index].free = Some(free);
            }
            Ok((_, None)) => {}
            Err(_) => break,
        }
    }
    drives
}

#[cfg(windows)]
fn mounts() -> Vec<Drive> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| {
            let letter = (b'A' + bit) as char;
            let root = format!("{letter}:\\");
            let wide: Vec<u16> = root.encode_utf16().chain([0]).collect();
            let kind = match unsafe { GetDriveTypeW(wide.as_ptr()) } {
                2 => "removable",
                3 => "fixed",
                4 => "network",
                5 => "cdrom",
                6 => "ram",
                _ => "unknown",
            };
            Drive { path: PathBuf::from(root), name: format!("{letter}:"), kind: kind.into(), total: None, free: None }
        })
        .collect()
}

#[cfg(windows)]
fn space(path: &Path) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
    let (mut free, mut total) = (0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some((total, free))
}

#[cfg(unix)]
fn space(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    let block = stat.f_frsize as u64;
    Some((stat.f_blocks as u64 * block, stat.f_bavail as u64 * block))
}

/// File system types worth going to although they are not a `/dev` device.
#[cfg(unix)]
const NETWORK_TYPES: [&str; 12] =
    ["nfs", "nfs4", "cifs", "smb3", "smbfs", "afpfs", "fuse.sshfs", "fuse.rclone", "9p", "drvfs", "virtiofs", "zfs"];

#[cfg(target_os = "linux")]
fn mounts() -> Vec<Drive> {
    let table = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mut drives: Vec<Drive> = Vec::new();
    for line in table.lines() {
        let mut fields = line.split(' ');
        let (Some(source), Some(target), Some(kind)) = (fields.next(), fields.next(), fields.next()) else { continue };
        let device = source.starts_with("/dev/") && kind != "squashfs";
        if !device && !NETWORK_TYPES.contains(&kind) {
            continue;
        }
        let target = unescape_mount(target);
        if target.starts_with("/snap/") {
            continue;
        }
        // mounted over: the last one is what is there
        drives.retain(|d| d.name != target);
        drives.push(Drive { path: PathBuf::from(&target), name: target, kind: kind.into(), total: None, free: None });
    }
    sort_mounts(&mut drives);
    drives
}

/// `/proc/mounts` writes a space in a path as `\040` (and tab, newline, `\` likewise).
#[cfg(target_os = "linux")]
fn unescape_mount(field: &str) -> String {
    let mut out = String::new();
    let mut rest = field;
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        let code = rest.get(at + 1..at + 4).and_then(|digits| u8::from_str_radix(digits, 8).ok());
        match code {
            Some(byte) => {
                out.push(byte as char);
                rest = &rest[at + 4..];
            }
            None => {
                out.push('\\');
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(target_os = "macos")]
fn mounts() -> Vec<Drive> {
    // MNT_DONTBROWSE: the system's own volumes (/System/Volumes/...), hidden in Finder too
    const MNT_DONTBROWSE: u32 = 0x0010_0000;
    let mut table: *mut libc::statfs = std::ptr::null_mut();
    let count = unsafe { libc::getmntinfo(&mut table, libc::MNT_NOWAIT) };
    let entries = if count > 0 && !table.is_null() {
        unsafe { std::slice::from_raw_parts(table, count as usize) }
    } else {
        &[]
    };
    let text = |chars: &[libc::c_char]| unsafe { std::ffi::CStr::from_ptr(chars.as_ptr()) }.to_string_lossy().into_owned();
    let mut drives: Vec<Drive> = Vec::new();
    for entry in entries {
        let (source, target, kind) = (text(&entry.f_mntfromname), text(&entry.f_mntonname), text(&entry.f_fstypename));
        if entry.f_flags & MNT_DONTBROWSE != 0 {
            continue;
        }
        if !source.starts_with("/dev/") && !NETWORK_TYPES.contains(&kind.as_str()) {
            continue;
        }
        drives.retain(|d| d.name != target);
        drives.push(Drive { path: PathBuf::from(&target), name: target, kind, total: None, free: None });
    }
    sort_mounts(&mut drives);
    drives
}

/// `/` first, then by path.
#[cfg(unix)]
fn sort_mounts(drives: &mut [Drive]) {
    drives.sort_by(|a, b| (a.name != "/", &a.name).cmp(&(b.name != "/", &b.name)));
}

/// Far's sizes: three significant digits, `866 G`, `57,3 G`, `1,82 T`.
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    let number = if unit == 0 || value >= 100.0 {
        format!("{}", value.round() as u64)
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    };
    format!("{} {}", number.replace('.', ","), UNITS[unit])
}

/// The drive `dir` is on: the longest drive path it starts with.
fn drive_of(drives: &[Drive], dir: &Path) -> usize {
    let dir = dir.to_string_lossy().to_lowercase();
    drives
        .iter()
        .enumerate()
        .filter(|(_, d)| {
            let path = d.path.to_string_lossy().to_lowercase();
            dir.starts_with(&path) || dir == path.trim_end_matches(['\\', '/'])
        })
        .max_by_key(|(_, d)| d.path.as_os_str().len())
        .map_or(0, |(index, _)| index)
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Open,
    Closed,
    Chosen(PathBuf),
}

pub struct DriveMenu {
    drives: Vec<Drive>,
    selected: usize,
    /// The panel it is for.
    pub right: bool,
}

impl DriveMenu {
    /// The list for the left or right panel, with the drive `dir` is on highlighted.
    pub fn new(right: bool, dir: &Path) -> Self {
        Self::with(list(), right, dir)
    }

    fn with(drives: Vec<Drive>, right: bool, dir: &Path) -> Self {
        let selected = drive_of(&drives, dir);
        Self { drives, selected, right }
    }

    fn choose(&self, index: usize) -> Outcome {
        self.drives.get(index).map_or(Outcome::Open, |d| Outcome::Chosen(d.path.clone()))
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let last = self.drives.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::F(10) => return Outcome::Closed,
            KeyCode::Up => self.selected = if self.selected == 0 { last } else { self.selected - 1 },
            KeyCode::Down => self.selected = if self.selected >= last { 0 } else { self.selected + 1 },
            KeyCode::Home | KeyCode::PageUp => self.selected = 0,
            KeyCode::End | KeyCode::PageDown => self.selected = last,
            KeyCode::Enter => return self.choose(self.selected),
            // a drive letter goes straight there
            KeyCode::Char(c) => {
                let wanted = format!("{}:", c.to_ascii_uppercase());
                if let Some(index) = self.drives.iter().position(|d| d.name == wanted) {
                    return self.choose(index);
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    /// Column widths: name, kind, total, free.
    fn widths(&self) -> (usize, usize, usize) {
        let name = self.drives.iter().map(|d| d.name.chars().count()).max().unwrap_or(2).min(40);
        let kind = self.drives.iter().map(|d| d.kind.chars().count()).max().unwrap_or(5);
        let sizes = self
            .drives
            .iter()
            .flat_map(|d| [d.total, d.free])
            .flatten()
            .map(|n| size(n).chars().count())
            .max()
            .unwrap_or(1)
            .max(6);
        (name, kind, sizes)
    }

    /// The double frame: column, row, width, height. Far puts it 5 columns
    /// into its panel (diskmenu.cpp) and centers it up and down.
    fn frame(&self, cols: u16, rows: u16) -> (u16, u16, u16, u16) {
        let (name, kind, sizes) = self.widths();
        // ║, the check mark column, " name kind │ total │ free ", ║
        let width = (1 + 2 + name + 1 + kind + 1 + 1 + 1 + sizes + 1 + 1 + 1 + sizes + 1 + 1) as u16;
        let width = width.max(18).min(cols.saturating_sub(4));
        let height = (self.drives.len() as u16 + 2).min(rows.saturating_sub(2));
        let from = if self.right { cols / 2 } else { 0 };
        let x = (from + 5).min(cols.saturating_sub(width + 2)).max(2);
        let y = (rows.saturating_sub(height)) / 2;
        (x, y, width, height)
    }

    pub fn paint(&self, grid: &mut Grid) {
        let (x, y, w, h) = self.frame(grid.cols, grid.rows);
        let (name_w, kind_w, size_w) = self.widths();
        // the box around the frame (a menu's: 2 columns, 1 row), and its shadow
        let (bx, by, bw, bh) = (x.saturating_sub(2), y.saturating_sub(1), w + 4, h + 2);
        grid.fill_with(bx, by, bw, bh, TEXT, BG);
        palette::drop_shadow(grid, bx, by, bw, bh);

        let first = 2 + 1 + name_w + 1 + kind_w + 1;
        let second = first + 1 + 1 + size_w + 1;
        let inner = w.saturating_sub(2) as usize;
        let border = |left: char, right: char, joint: char| {
            let mut line: Vec<char> = std::iter::repeat_n('\u{2550}', inner).collect();
            for at in [first, second] {
                if at > 0 && at <= inner {
                    line[at - 1] = joint;
                }
            }
            format!("{left}{}{right}", line.into_iter().collect::<String>())
        };
        grid.put(x, y, &border('\u{2554}', '\u{2557}', '\u{2550}'), TEXT, BG);
        grid.put(x, y + h - 1, &border('\u{255A}', '\u{255D}', '\u{2567}'), TEXT, BG);
        let title = " Change drive ";
        grid.put(x + w.saturating_sub(title.chars().count() as u16) / 2, y, title, TEXT, BG);

        for (index, drive) in self.drives.iter().enumerate().take(h.saturating_sub(2) as usize) {
            let row = y + 1 + index as u16;
            let selected = index == self.selected;
            let (fg, bg) = if selected { (SELECTED_TEXT, SELECTED_BG) } else { (TEXT, BG) };
            grid.put(x, row, "\u{2551}", TEXT, BG);
            grid.put(x + w - 1, row, "\u{2551}", TEXT, BG);
            grid.fill_with(x + 1, row, w - 2, 1, fg, bg);
            let name: String = drive.name.chars().take(name_w).collect();
            let shown = |n: Option<u64>| n.map(size).unwrap_or_default();
            let text = format!(
                "  {name:<name_w$} {kind:<kind_w$} \u{2502} {total:>size_w$} \u{2502} {free:>size_w$} ",
                kind = drive.kind,
                total = shown(drive.total),
                free = shown(drive.free),
            );
            let text: String = text.chars().take(inner).collect();
            grid.put(x + 1, row, &text, fg, bg);
            // the drive letter is its hot key, yellow on the selection too
            if drive.name.len() == 2 && drive.name.ends_with(':') {
                grid.put(x + 3, row, &drive.name[..1], HOT, bg);
            }
        }
    }

    /// A left click: a row goes to that drive, outside closes.
    pub fn click(&mut self, event: MouseEvent, cols: u16, rows: u16) -> Outcome {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Outcome::Open;
        }
        let (x, y, w, h) = self.frame(cols, rows);
        let (col, row) = (event.column, event.row);
        if col > x && col + 1 < x + w && row > y && row + 1 < y + h {
            let index = (row - y - 1) as usize;
            self.selected = index;
            return self.choose(index);
        }
        if col + 2 >= x && col < x + w + 2 && row + 1 >= y && row <= y + h {
            return Outcome::Open;
        }
        Outcome::Closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn drive(name: &str, path: &str) -> Drive {
        Drive { path: PathBuf::from(path), name: name.into(), kind: "fixed".into(), total: Some(1 << 40), free: Some(5 << 30) }
    }

    #[test]
    fn sizes_are_written_the_far_way() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(866 * (1 << 30)), "866 G");
        assert_eq!(size(57 * (1 << 30) + (1 << 29) - (1 << 26)), "57,4 G");
        assert_eq!(size(2_000_000_000_000), "1,82 T");
    }

    #[test]
    fn the_current_drive_is_highlighted_and_letters_and_enter_choose() {
        let drives = vec![drive("C:", "C:\\"), drive("D:", "D:\\"), drive("X:", "X:\\")];
        let mut menu = DriveMenu::with(drives, false, Path::new("d:\\fallen5\\interface"));
        assert_eq!(menu.selected, 1);
        assert_eq!(menu.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)), Outcome::Chosen("X:\\".into()));
        menu.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(menu.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Outcome::Chosen("C:\\".into()));
        assert_eq!(menu.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)), Outcome::Closed);
    }

    #[test]
    fn the_deepest_mount_point_wins() {
        let drives = vec![drive("/", "/"), drive("/mnt/wrk", "/mnt/wrk"), drive("/mnt/wrk2", "/mnt/wrk2")];
        let menu = DriveMenu::with(drives, true, Path::new("/mnt/wrk2/films"));
        assert_eq!(menu.selected, 2);
    }

    #[test]
    fn painting_and_clicking_a_row() {
        let drives = vec![drive("C:", "C:\\"), drive("D:", "D:\\")];
        let mut menu = DriveMenu::with(drives, true, Path::new("C:\\"));
        let mut grid = Grid::new(100, 30);
        menu.paint(&mut grid);
        let (x, y, _, _) = menu.frame(100, 30);
        assert!(x >= 50, "over the right panel");
        let row: String = grid.row(y + 2).iter().map(|c| c.text.clone()).collect();
        assert!(row.contains("D: fixed") && row.contains("1,00 T") && row.contains("5,00 G"), "{row}");
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x + 3,
            row: y + 2,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(menu.click(click, 100, 30), Outcome::Chosen("D:\\".into()));
    }

    #[test]
    fn the_real_list_is_not_empty() {
        let drives = list();
        for d in &drives {
            println!("{:<20} {:<10} {:>8} {:>8}", d.name, d.kind, d.total.map(size).unwrap_or_default(), d.free.map(size).unwrap_or_default());
        }
        assert!(!drives.is_empty());
    }
}
