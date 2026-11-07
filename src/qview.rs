//! Ctrl+Q: Far's quick view panel (qview.cpp of Far 3.0.6446): the other
//! panel's file under the cursor in a viewer, or for a folder what Far's
//! GetDirInfo counts in it (scanned in the background; `~` and the
//! highlight color until it is done). Under the view a separator with the
//! file's type, and the name.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crossterm::style::Color;

use crate::grid::Grid;
use crate::listing;
use crate::viewer::Viewer;

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
/// COL_PANELHIGHLIGHTTEXT: values while the scan is not finished.
#[allow(non_snake_case)]
fn HIGHLIGHT() -> Color {
    crate::colors::fg("Panel.Text.Highlight")
}
/// COL_PANELSELECTEDINFO: the file type, yellow on cyan.
#[allow(non_snake_case)]
fn SELECTED_INFO_BG() -> Color {
    crate::colors::bg("Panel.Title.Selected")
}
#[allow(non_snake_case)]
fn BLACK() -> Color {
    crate::colors::fg("Panel.Title.Selected")
}

/// What GetDirInfo counts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirInfo {
    pub dirs: u64,
    pub files: u64,
    pub size: u64,
    pub allocated: u64,
    pub cluster: u64,
    pub slack: u64,
    pub mft: u64,
    pub done: bool,
}

/// The quick view of one path.
pub struct State {
    pub path: PathBuf,
    viewer: Option<Viewer>,
    file_type: String,
    scan: Option<Arc<Mutex<DirInfo>>>,
    /// A binary file that is not a placement: nothing is shown but a line saying so.
    binary: bool,
}

impl State {
    pub fn new(path: PathBuf) -> Self {
        let is_dir = path.is_dir();
        let scan = is_dir.then(|| {
            let info = Arc::new(Mutex::new(DirInfo { cluster: cluster_size(&path), ..DirInfo::default() }));
            let (shared, root) = (info.clone(), path.clone());
            std::thread::spawn(move || {
                scan(&root, &shared);
                lock(&shared).done = true;
            });
            info
        });
        // a placement (Som shows it) first, then a binary file (nothing to show), then text
        let media = crate::srp::in_som() && crate::srp::kind_of(&path).is_some();
        let binary = !is_dir && !media && is_binary(&path);
        let viewer = if is_dir || binary { None } else { Viewer::open_quick(&path).ok() };
        let file_type = if is_dir { String::new() } else { file_type(&path) };
        Self { path, viewer, file_type, scan, binary }
    }

    pub fn viewer(&mut self) -> Option<&mut Viewer> {
        self.viewer.as_mut()
    }

    /// The placement of the file shown changed (a new track) and is not drawn yet.
    pub fn viewer_busy(&self) -> bool {
        self.viewer.as_ref().is_some_and(Viewer::busy)
    }

    pub fn scanning(&self) -> bool {
        self.scan.as_ref().is_some_and(|s| !lock(s).done)
    }
}

fn lock(info: &Mutex<DirInfo>) -> std::sync::MutexGuard<'_, DirInfo> {
    info.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// GetDirInfo: folders, files, their size and allocation; the part of the
/// allocation past the size is slack on files of a cluster or more, MFT
/// overhead on smaller ones.
fn scan(dir: &Path, info: &Mutex<DirInfo>) {
    for entry in listing::read(dir) {
        let path = dir.join(&entry.name);
        if entry.is_dir {
            lock(info).dirs += 1;
            if entry.reparse.is_none() {
                scan(&path, info);
            }
            continue;
        }
        let mut data = lock(info);
        data.files += 1;
        data.size += entry.size;
        data.allocated += entry.allocated;
        if entry.allocated > entry.size {
            if entry.allocated >= data.cluster {
                data.slack += entry.allocated - entry.size;
            } else {
                data.mft += entry.allocated - entry.size;
            }
        }
    }
}

/// Far's size2str for the quick view: the float size in 10 cells, trimmed, with `B`.
fn size2str(size: u64) -> String {
    let text = listing::float_size(size);
    if text.ends_with('B') { text } else { format!("{text}B") }
}

fn percent(part: u64, total: u64) -> u64 {
    if total == 0 { 0 } else { part * 100 / total }
}

/// Draws the panel in (`left`, `width`, `height`). `bare` is Ctrl+Q's layout
/// (the user's call, 2026-11-07): no frame, no title, no name line — the
/// viewer (or the folder's figures, or why there is nothing) to the edges.
pub fn paint(grid: &mut Grid, left: u16, width: u16, height: u16, active: bool, state: Option<&mut State>, bare: bool) {
    if bare {
        return paint_bare(grid, left, width, height, state);
    }
    if width < 6 || height < 5 {
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
    let title = " Quick view ";
    let (fg, bg) = if active { (BLACK(), SELECTED_INFO_BG()) } else { (TEXT(), BG()) };
    grid.put(left + width.saturating_sub(title.chars().count() as u16) / 2, 0, title, fg, bg);
    // the separator above the name
    grid.put(left, bottom - 2, &format!("\u{255F}{}\u{2562}", "\u{2500}".repeat(inner)), TEXT(), BG());
    let Some(state) = state else { return };
    let name = state.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    grid.put(left + 1, bottom - 1, &listing::left(&name, inner), TEXT(), BG());
    if !state.file_type.is_empty() {
        let text = listing::truncate_left(&format!(" {} ", state.file_type), inner);
        grid.put(left + width.saturating_sub(text.chars().count() as u16) / 2, bottom - 2, &text, INFO(), SELECTED_INFO_BG());
    }
    if let Some(scan) = &state.scan {
        let data = lock(scan).clone();
        let room = inner.saturating_sub("Folder".len() + 5);
        let shown = listing::truncate_path(&state.path.display().to_string(), room);
        let print = |grid: &mut Grid, y: u16, text: &str, color: Color| {
            let room = right.saturating_sub(left + 2) as usize;
            grid.put(left + 2, y, &text.chars().take(room).collect::<String>(), color, BG());
        };
        print(grid, 2, &format!("Folder \"{shown}\""), TEXT());
        let labels = ["Folders:", "Files:", "Files size:", "Allocated size:", "Cluster size:", "Files slack:", "MFT overhead:"];
        let column = labels.iter().map(|l| l.len()).max().unwrap_or(0) + 1;
        let (color, prefix) = if data.done { (INFO(), "") } else { (HIGHLIGHT(), "~") };
        let rows = [
            (4, labels[0], data.dirs.to_string()),
            (5, labels[1], data.files.to_string()),
            (6, labels[2], size2str(data.size)),
            (7, labels[3], format!("{} ({}%)", size2str(data.allocated), percent(data.allocated, data.size))),
            (9, labels[4], crate::dialog::group(data.cluster)),
            (10, labels[5], format!("{} ({}%)", size2str(data.slack), percent(data.slack, data.allocated))),
            (11, labels[6], format!("{} ({}%)", size2str(data.mft), percent(data.mft, data.allocated))),
        ];
        for (y, label, value) in rows {
            if y >= bottom - 2 {
                break;
            }
            print(grid, y, &format!("{label:<column$}"), TEXT());
            let x = left + 2 + column as u16;
            let room = right.saturating_sub(x) as usize;
            grid.put(x, y, &format!("{prefix}{value}").chars().take(room).collect::<String>(), color, BG());
        }
    } else if state.binary {
        let text: String = crate::lang::tr("A binary file: nothing to show").chars().take(inner).collect();
        grid.put(left + 1, 1, &text, TEXT(), BG());
    } else if let Some(viewer) = &mut state.viewer {
        viewer.paint_area(grid, left + 1, 1, width - 2, height.saturating_sub(4));
    }
}

/// Ctrl+Q's viewer: the whole rectangle is the file's (a placement, the text),
/// or the folder's figures from the top, or a short line for a binary file.
fn paint_bare(grid: &mut Grid, left: u16, width: u16, height: u16, state: Option<&mut State>) {
    grid.fill(left, 0, width, height, BG());
    if width == 0 || height == 0 {
        return;
    }
    let Some(state) = state else { return };
    let line = |grid: &mut Grid, y: u16, text: &str, color: Color| {
        if y < height {
            grid.put(left, y, &text.chars().take(width as usize).collect::<String>(), color, BG());
        }
    };
    if state.binary {
        line(grid, 0, crate::lang::tr("A binary file: nothing to show"), TEXT());
    } else if let Some(scan) = &state.scan {
        let data = lock(scan).clone();
        let shown = listing::truncate_path(&state.path.display().to_string(), (width as usize).saturating_sub("Folder".len() + 3));
        line(grid, 0, &format!("Folder \"{shown}\""), TEXT());
        let (color, prefix) = if data.done { (INFO(), "") } else { (HIGHLIGHT(), "~") };
        let rows = [
            ("Folders:", data.dirs.to_string()),
            ("Files:", data.files.to_string()),
            ("Files size:", size2str(data.size)),
            ("Allocated size:", format!("{} ({}%)", size2str(data.allocated), percent(data.allocated, data.size))),
            ("Cluster size:", crate::dialog::group(data.cluster)),
            ("Files slack:", format!("{} ({}%)", size2str(data.slack), percent(data.slack, data.allocated))),
            ("MFT overhead:", format!("{} ({}%)", size2str(data.mft), percent(data.mft, data.allocated))),
        ];
        let column = rows.iter().map(|(l, _)| l.len()).max().unwrap_or(0) + 1;
        for (i, (label, value)) in rows.iter().enumerate() {
            let y = 2 + i as u16;
            line(grid, y, &format!("{label:<column$}"), TEXT());
            if y < height && (column as u16) < width {
                let room = (width as usize).saturating_sub(column);
                grid.put(left + column as u16, y, &format!("{prefix}{value}").chars().take(room).collect::<String>(), color, BG());
            }
        }
    } else if let Some(viewer) = &mut state.viewer {
        viewer.paint_area(grid, left, 0, width, height);
    }
}

/// Whether `path` reads as a binary file: a NUL, or more than a tenth control
/// bytes (other than tab, line ends, form feed, escape), in its first 8 KiB.
/// A UTF-16/32 file starts with its BOM and is text.
pub fn is_binary(path: &Path) -> bool {
    use std::io::Read as _;
    let Ok(file) = std::fs::File::open(path) else { return false };
    let mut head = Vec::with_capacity(8192);
    if file.take(8192).read_to_end(&mut head).is_err() || head.is_empty() {
        return false;
    }
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) || head.starts_with(&[0, 0, 0xFE, 0xFF]) {
        return false;
    }
    if head.contains(&0) {
        return true;
    }
    let control = head.iter().filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B)).count();
    control * 10 > head.len()
}

/// The volume's cluster size, as GetDirInfo takes it.
fn cluster_size(path: &Path) -> u64 {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceW, GetVolumePathNameW};
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut root = [0u16; 512];
        // SAFETY: the buffer is as long as said.
        if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
            return 0;
        }
        let (mut sectors, mut bytes, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
        // SAFETY: `root` is NUL-terminated; the out values are u32s.
        if unsafe { GetDiskFreeSpaceW(root.as_ptr(), &mut sectors, &mut bytes, &mut free, &mut total) } == 0 {
            return 0;
        }
        u64::from(sectors) * u64::from(bytes)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).map_or(0, |m| m.blksize())
    }
}

/// The shell's name for the file's type (os::com::get_shell_filetype_description):
/// the registry's description of its extension's class.
fn file_type(path: &Path) -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{HKEY_CLASSES_ROOT, RRF_RT_REG_SZ, RegGetValueW};
        let read = |key: &str| -> Option<String> {
            let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
            let mut buffer = [0u16; 512];
            let mut len = (buffer.len() * 2) as u32;
            // SAFETY: the buffer is as long as said in bytes; no value name reads the default.
            let status = unsafe { RegGetValueW(HKEY_CLASSES_ROOT, key.as_ptr(), std::ptr::null(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut len) };
            if status != 0 {
                return None;
            }
            let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
            Some(String::from_utf16_lossy(&buffer[..end])).filter(|s| !s.is_empty())
        };
        let Some(extension) = path.extension() else { return String::new() };
        let class = read(&format!(".{}", extension.to_string_lossy()));
        class.and_then(|class| read(&class)).unwrap_or_default()
    }
    #[cfg(unix)]
    {
        let _ = path;
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_counted_and_a_file_is_viewed() {
        let dir = std::env::temp_dir().join(format!("farsh-qview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "hello quick view").unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), vec![b'x'; 5000]).unwrap();
        let mut state = State::new(dir.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while state.scanning() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut grid = Grid::new(40, 20);
        paint(&mut grid, 0, 40, 20, false, Some(&mut state), false);
        let rows: Vec<String> = (0..20).map(|row| grid.row(row).iter().map(|cell| cell.text.clone()).collect()).collect();
        assert!(rows[0].contains(" Quick view "));
        assert!(rows[2].contains("Folder \""), "{rows:#?}");
        assert!(rows[4].contains("Folders:        1"), "{:?}", rows[4]);
        assert!(rows[5].contains("Files:          2"), "{:?}", rows[5]);
        assert!(rows[6].contains("Files size:     4,90 KB"), "{:?}", rows[6]);

        let mut state = State::new(dir.join("a.txt"));
        let mut grid = Grid::new(40, 20);
        paint(&mut grid, 0, 40, 20, false, Some(&mut state), false);
        let rows: Vec<String> = (0..20).map(|row| grid.row(row).iter().map(|cell| cell.text.clone()).collect()).collect();
        assert!(rows[1].starts_with("\u{2551}hello quick view"), "{rows:#?}");
        assert!(rows[18].starts_with("\u{2551}a.txt"));
        assert!(rows[17].starts_with("\u{255F}"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
