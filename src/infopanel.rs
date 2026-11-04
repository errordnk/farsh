//! Ctrl+L: Far's information panel (infolist.cpp of Far 3.0.6446): the
//! computer and the user, the disk the other panel is on, the memory, and
//! the folder's description file. Values are right aligned in yellow
//! (COL_PANELINFOTEXT) with a space after; each section has a title in a
//! separator with `[-]` on its left.

use std::path::Path;

use crossterm::style::Color;

use crate::grid::Grid;
use crate::listing;

#[allow(non_snake_case)]
fn BG() -> Color {
    crate::colors::bg("Panel.Text")
}
#[allow(non_snake_case)]
fn TEXT() -> Color {
    crate::colors::fg("Panel.Text")
}
#[allow(non_snake_case)]
fn INFO() -> Color {
    crate::colors::fg("Panel.Text.Info")
}
#[allow(non_snake_case)]
fn SELECTED_TITLE_BG() -> Color {
    crate::colors::bg("Panel.Title.Selected")
}
#[allow(non_snake_case)]
fn BLACK() -> Color {
    crate::colors::fg("Panel.Title.Selected")
}

/// Far's folder information files (System.FolderInfo's default).
const FOLDER_INFO: [&str; 5] = ["DirInfo", "File_Id.diz", "Descript.ion", "ReadMe.*", "Read.Me"];

/// What the panel shows, read once when it is drawn.
pub struct Info {
    pub computer: String,
    pub user: String,
    pub access_level: Option<&'static str>,
    pub elevated: String,
    /// "Fixed disk C:\ (NTFS)", the root, total and free bytes, label, serial.
    pub disk: Option<Disk>,
    /// (committable, addressable, physical) as (total, available), and installed.
    pub memory: Option<([(u64, u64); 3], Option<u64>)>,
}

pub struct Disk {
    pub title: String,
    pub space: Option<(u64, u64)>,
    pub label: String,
    pub serial: String,
}

/// Far's size2str: the float size with its unit and `B` (`476 GB`, `97 B`).
fn size2str(size: u64) -> String {
    let text = listing::float_size(size);
    if text.ends_with('B') { text } else { format!("{text}B") }
}

fn percent(part: u64, total: u64) -> u64 {
    if total == 0 { 0 } else { part * 100 / total }
}

/// The description file of `dir`, as Far looks for one (the first of
/// FolderInfo that is there; a mask like `ReadMe.*` takes the first match).
pub fn folder_description(dir: &Path) -> Option<std::path::PathBuf> {
    let configured = crate::settings::string("System.FolderInfo");
    let names: Vec<&str> = configured.split(',').map(str::trim).filter(|n| !n.is_empty()).collect();
    let names = if names.is_empty() { FOLDER_INFO.to_vec() } else { names };
    for name in names {
        if let Some(stem) = name.strip_suffix(".*") {
            let entries = std::fs::read_dir(dir).ok()?;
            let found = entries.flatten().map(|e| e.path()).find(|p| {
                p.is_file() && p.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(stem)) && p.extension().is_some()
            });
            if found.is_some() {
                return found;
            }
            continue;
        }
        let entries = std::fs::read_dir(dir).ok()?;
        if let Some(path) = entries.flatten().map(|e| e.path()).find(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))) {
            return Some(path);
        }
    }
    None
}

/// Draws the panel in (`left`, `width`, `height`) about the folder `dir`
/// the other panel shows (`in_archive`: Far's plugin panel, which has no
/// folder description).
pub fn paint(grid: &mut Grid, left: u16, width: u16, height: u16, active: bool, info: &Info, dir: &Path, in_archive: bool) {
    if width < 6 || height < 3 {
        return;
    }
    let (right, bottom) = (left + width - 1, height - 1);
    grid.fill(left, 0, width, height, BG());
    let inner = (width - 2) as usize;
    let horizontal = "\u{2550}".repeat(inner);
    grid.put(left, 0, &format!("\u{2554}{horizontal}\u{2557}"), TEXT(), BG());
    grid.put(left, bottom, &format!("\u{255A}{horizontal}\u{255D}"), TEXT(), BG());
    for row in 1..bottom {
        grid.put(left, row, "\u{2551}", TEXT(), BG());
        grid.put(right, row, "\u{2551}", TEXT(), BG());
    }
    let title = " Information ";
    let (fg, bg) = if active { (BLACK(), SELECTED_TITLE_BG()) } else { (TEXT(), BG()) };
    grid.put(left + (width.saturating_sub(title.chars().count() as u16)) / 2, 0, title, fg, bg);

    let mut y = 1u16;
    // PrintText at left + 2; PrintInfo right aligned before the border, with a space after
    let line = |grid: &mut Grid, y: &mut u16, label: &str, value: &str| {
        if *y > bottom - 1 {
            return;
        }
        let room = right.saturating_sub(left + 2) as usize;
        grid.put(left + 2, *y, &label.chars().take(room).collect::<String>(), TEXT(), BG());
        let label_end = left + 2 + label.chars().count() as u16;
        let max = right.saturating_sub(label_end).saturating_sub(2) as usize;
        let value = listing::truncate_left(value, max);
        let x = right.saturating_sub(value.chars().count() as u16 + 1);
        if x > left && x > label_end {
            grid.put(x, *y, &format!("{value} "), INFO(), BG());
        }
        *y += 1;
    };
    let section = |grid: &mut Grid, y: &mut u16, title: &str| {
        if *y >= bottom {
            return;
        }
        grid.put(left, *y, &format!("\u{255F}{}\u{2562}", "\u{2500}".repeat(inner)), TEXT(), BG());
        let title = listing::truncate_left(&format!(" {title} "), (width as usize).saturating_sub(4));
        grid.put(left + width.saturating_sub(title.chars().count() as u16) / 2, *y, &title, TEXT(), BG());
        grid.put(left + 1, *y, "[-]", TEXT(), BG());
        *y += 1;
    };

    line(grid, &mut y, "Computer name", &info.computer);
    line(grid, &mut y, "User name", &info.user);
    if let Some(level) = info.access_level {
        line(grid, &mut y, "Access level", level);
    }
    line(grid, &mut y, "Elevated", &info.elevated);

    match &info.disk {
        Some(disk) => {
            section(grid, &mut y, &disk.title);
            if let Some((total, free)) = disk.space {
                line(grid, &mut y, "Space, total", &size2str(total));
                line(grid, &mut y, "Space, available", &format!("{}%, {}", percent(free, total), size2str(free)));
            }
            line(grid, &mut y, "Volume label", &disk.label);
            line(grid, &mut y, "Serial number", &disk.serial);
        }
        None => section(grid, &mut y, "Disk"),
    }

    section(grid, &mut y, "Memory");
    if let Some((metrics, installed)) = &info.memory {
        for (kind, (total, available)) in ["Committable", "Addressable", "Physical"].iter().zip(metrics) {
            line(grid, &mut y, &format!("{kind}, total"), &size2str(*total));
            line(grid, &mut y, &format!("{kind}, available"), &format!("{}%, {}", percent(*available, *total), size2str(*available)));
        }
        if let Some(installed) = installed {
            line(grid, &mut y, "Physical, installed", &size2str(*installed));
        }
    }

    if !in_archive {
        section(grid, &mut y, "Description");
        let text = folder_description(dir).and_then(|path| std::fs::read(path).ok()).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        match text {
            // the file in a viewer from here to the bottom
            Some(text) if y < bottom => {
                for row in text.lines() {
                    if y >= bottom {
                        break;
                    }
                    let shown: String = row.replace('\t', "        ").chars().take(inner).collect();
                    grid.put(left + 1, y, &shown, TEXT(), BG());
                    y += 1;
                }
            }
            _ => line(grid, &mut y, "Folder description file is absent", ""),
        }
    }
}

/// What the panel shows about this machine and the disk of `dir`.
pub fn read(dir: &Path) -> Info {
    platform::read(dir)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    fn from_wide(buffer: &[u16]) -> String {
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }

    pub fn read(dir: &Path) -> Info {
        use windows_sys::Win32::System::SystemInformation::{ComputerNamePhysicalNetBIOS, GetComputerNameExW};
        use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;
        let mut buffer = [0u16; 256];
        let mut len = buffer.len() as u32;
        // SAFETY: the buffer is as long as said.
        let computer = if unsafe { GetComputerNameExW(ComputerNamePhysicalNetBIOS, buffer.as_mut_ptr(), &mut len) } != 0 { from_wide(&buffer) } else { String::new() };
        let mut user_buffer = [0u16; 256];
        let mut user_len = user_buffer.len() as u32;
        // SAFETY: the buffer is as long as said.
        let user = if unsafe { GetUserNameW(user_buffer.as_mut_ptr(), &mut user_len) } != 0 { from_wide(&user_buffer) } else { String::new() };
        Info { access_level: access_level(&user), computer, user, elevated: elevated(), disk: disk(dir), memory: memory() }
    }

    /// NetUserGetInfo's usri1_priv, as Far names it.
    fn access_level(user: &str) -> Option<&'static str> {
        use windows_sys::Win32::NetworkManagement::NetManagement::{NetApiBufferFree, NetUserGetInfo, USER_INFO_1, USER_PRIV_ADMIN, USER_PRIV_GUEST, USER_PRIV_USER};
        let name = wide(user);
        let mut info: *mut u8 = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated; the buffer comes back in `info`.
        if unsafe { NetUserGetInfo(std::ptr::null(), name.as_ptr(), 1, &mut info) } != 0 || info.is_null() {
            return None;
        }
        // SAFETY: level 1 is a USER_INFO_1.
        let privilege = unsafe { (*(info as *const USER_INFO_1)).usri1_priv };
        // SAFETY: allocated by NetUserGetInfo.
        unsafe { NetApiBufferFree(info.cast()) };
        Some(match privilege {
            USER_PRIV_GUEST => "Guest",
            USER_PRIV_USER => "User",
            USER_PRIV_ADMIN => "Administrator",
            _ => "Unknown",
        })
    }

    /// "Yes"/"No", with the elevation type when it is split (Far's MInfoUserAccessElevated).
    fn elevated() -> String {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation, TokenElevationType};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo handle of this process.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return "No".into();
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut kind = 0i32;
        let mut len = 0u32;
        // SAFETY: the out buffers are as big as said.
        let is = unsafe { GetTokenInformation(token, TokenElevation, (&raw mut elevation).cast(), 4, &mut len) } != 0 && elevation.TokenIsElevated != 0;
        // SAFETY: as above.
        let typed = unsafe { GetTokenInformation(token, TokenElevationType, (&raw mut kind).cast(), 4, &mut len) } != 0;
        // SAFETY: opened above.
        unsafe { CloseHandle(token) };
        let yes = if is { "Yes" } else { "No" };
        // TokenElevationTypeFull 2, Limited 3
        match (typed, kind) {
            (true, 2) => format!("{yes} (Full)"),
            (true, 3) => format!("{yes} (Limited)"),
            _ => yes.into(),
        }
    }

    fn disk(dir: &Path) -> Option<Disk> {
        use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW};
        let path: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut root = [0u16; 512];
        // SAFETY: the buffer is as long as said.
        if unsafe { GetVolumePathNameW(path.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
            return None;
        }
        let mut label = [0u16; 256];
        let mut fs = [0u16; 64];
        let (mut serial, mut max_len, mut flags) = (0u32, 0u32, 0u32);
        // SAFETY: the buffers are as long as said.
        let ok = unsafe { GetVolumeInformationW(root.as_ptr(), label.as_mut_ptr(), label.len() as u32, &mut serial, &mut max_len, &mut flags, fs.as_mut_ptr(), fs.len() as u32) } != 0;
        let root_text = from_wide(&root);
        if !ok {
            return Some(Disk { title: root_text, space: None, label: String::new(), serial: String::new() });
        }
        // SAFETY: `root` is NUL-terminated.
        let kind = match unsafe { GetDriveTypeW(root.as_ptr()) } {
            2 => "Removable",
            3 => "Fixed",
            4 => "Network",
            5 => "CD-ROM",
            6 => "RAM",
            _ => "Unknown",
        };
        let (mut user_total, mut total, mut user_free) = (0u64, 0u64, 0u64);
        // SAFETY: the out values are u64s.
        let space = (unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut user_free, &mut user_total, &mut total) } != 0).then_some((user_total, user_free));
        Some(Disk {
            title: format!("{kind} disk {root_text} ({})", from_wide(&fs)),
            space,
            label: from_wide(&label),
            serial: format!("{:04X}-{:04X}", serial >> 16, serial & 0xFFFF),
        })
    }

    fn memory() -> Option<([(u64, u64); 3], Option<u64>)> {
        use windows_sys::Win32::System::SystemInformation::{GetPhysicallyInstalledSystemMemory, GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: plain data; the size field is set below.
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        // SAFETY: `status` is a MEMORYSTATUSEX with its length.
        if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
            return None;
        }
        let mut kilobytes = 0u64;
        // SAFETY: the out value is a u64.
        let installed = (unsafe { GetPhysicallyInstalledSystemMemory(&mut kilobytes) } != 0).then_some(kilobytes << 10);
        Some((
            [(status.ullTotalPageFile, status.ullAvailPageFile), (status.ullTotalVirtual, status.ullAvailVirtual), (status.ullTotalPhys, status.ullAvailPhys)],
            installed,
        ))
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    pub fn read(dir: &Path) -> Info {
        let mut host = [0 as libc::c_char; 256];
        // SAFETY: the buffer is as long as said.
        let computer = if unsafe { libc::gethostname(host.as_mut_ptr(), host.len()) } == 0 {
            // SAFETY: gethostname wrote a NUL-terminated name.
            unsafe { std::ffi::CStr::from_ptr(host.as_ptr()) }.to_string_lossy().into_owned()
        } else {
            String::new()
        };
        let user = std::env::var("USER").unwrap_or_default();
        // SAFETY: no arguments.
        let elevated = if unsafe { libc::geteuid() } == 0 { "Yes" } else { "No" }.to_string();
        Info { computer, user, access_level: None, elevated, disk: disk(dir), memory: memory() }
    }

    fn disk(dir: &Path) -> Option<Disk> {
        let path = CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: plain data, filled in by the call.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `path` is NUL-terminated.
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        let block = stat.f_frsize as u64;
        Some(Disk {
            title: format!("Fixed disk {}", dir.display()),
            space: Some((stat.f_blocks as u64 * block, stat.f_bavail as u64 * block)),
            label: String::new(),
            serial: format!("{:04X}-{:04X}", (stat.f_fsid as u64 >> 16) & 0xFFFF, stat.f_fsid as u64 & 0xFFFF),
        })
    }

    fn memory() -> Option<([(u64, u64); 3], Option<u64>)> {
        // SAFETY: plain queries.
        let (pages, page) = unsafe { (libc::sysconf(libc::_SC_PHYS_PAGES), libc::sysconf(libc::_SC_PAGESIZE)) };
        if pages <= 0 || page <= 0 {
            return None;
        }
        let total = pages as u64 * page as u64;
        Some(([(0, 0), (0, 0), (total, 0)], Some(total)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_information_panel_shows_fars_sections() {
        let dir = std::env::temp_dir().join(format!("farsh-info-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let info = read(&dir);
        let mut grid = Grid::new(40, 30);
        paint(&mut grid, 0, 40, 30, false, &info, &dir, false);
        let rows: Vec<String> = (0..30).map(|row| grid.row(row).iter().map(|cell| cell.text.clone()).collect()).collect();
        assert!(rows[0].contains(" Information "));
        assert!(rows[1].starts_with("\u{2551} Computer name"));
        assert!(rows[2].starts_with("\u{2551} User name"));
        assert!(rows.iter().any(|r| r.starts_with("\u{255F}[-]") && r.contains(" Memory ")), "{rows:#?}");
        assert!(rows.iter().any(|r| r.contains(" Description ")));
        assert!(rows.iter().any(|r| r.contains("Folder description file is absent")));
        // values end a space before the border
        let total = rows.iter().find(|r| r.contains("Physical, total")).unwrap();
        assert!(total.ends_with("B \u{2551}"), "{total:?}");
        std::fs::write(dir.join("Descript.ion"), "first line\nsecond").unwrap();
        let mut grid = Grid::new(40, 30);
        paint(&mut grid, 0, 40, 30, false, &info, &dir, false);
        let rows: Vec<String> = (0..30).map(|row| grid.row(row).iter().map(|cell| cell.text.clone()).collect()).collect();
        assert!(rows.iter().any(|r| r.starts_with("\u{2551}first line")), "{rows:#?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sizes_are_far_float_sizes_with_b() {
        assert_eq!(size2str(97), "97 B");
        assert_eq!(size2str(27_052_032), "25,8 MB");
    }
}
