//! Alt+F6 (copy.cpp with Link, flink.cpp's MkHardLink/MkSymLink): a link
//! to each source, inside the destination when it is a folder that exists,
//! else at the destination itself.

use std::path::{Path, PathBuf};

/// The link types in the order of Far's "Link type" list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkType {
    Hard,
    Junction,
    /// A file or a folder link, as the source is.
    Symbolic,
    SymbolicFile,
    SymbolicFolder,
}

impl LinkType {
    pub const LABELS: [&'static str; 5] = ["&hard link", "directory &junction", "&symbolic link", "symbolic link (&file)", "symbolic link (fol&der)"];

    pub fn from_index(index: usize) -> Self {
        [Self::Hard, Self::Junction, Self::Symbolic, Self::SymbolicFile, Self::SymbolicFolder][index.min(4)]
    }
}

/// Where the link to `source` goes for what was typed (relative to `base`).
pub fn target(source: &Path, typed: &str, base: &Path) -> PathBuf {
    let typed = typed.trim().trim_matches('"');
    let dest = base.join(typed);
    let into_folder = typed.ends_with(['\\', '/']) || dest.is_dir();
    match (into_folder, source.file_name()) {
        (true, Some(name)) => dest.join(name),
        _ => dest,
    }
}

pub fn create(source: &Path, link: &Path, kind: LinkType) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("Cannot create the link\n{}\n{e}", link.display());
    if link.symlink_metadata().is_ok() {
        return Err(format!("Cannot create the link\n{}\nIt already exists", link.display()));
    }
    let is_dir = source.is_dir();
    match kind {
        LinkType::Hard if is_dir => Err(format!("Cannot create a hard link to a folder\n{}", source.display())),
        LinkType::Hard => std::fs::hard_link(source, link).map_err(fail),
        LinkType::Junction if !is_dir => Err(format!("A junction can only point to a folder\n{}", source.display())),
        LinkType::Junction => junction(&absolute(source), link).map_err(fail),
        LinkType::Symbolic => symlink(&absolute(source), link, is_dir).map_err(fail),
        LinkType::SymbolicFile => symlink(&absolute(source), link, false).map_err(fail),
        LinkType::SymbolicFolder => symlink(&absolute(source), link, true).map_err(fail),
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path, dir: bool) -> std::io::Result<()> {
    if dir { std::os::windows::fs::symlink_dir(target, link) } else { std::os::windows::fs::symlink_file(target, link) }
}

#[cfg(not(windows))]
fn symlink(target: &Path, link: &Path, _: bool) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// A mount point reparse point (what `mklink /J` makes).
#[cfg(windows)]
fn junction(target: &Path, link: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, OPEN_EXISTING};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

    let print: Vec<u16> = target.as_os_str().encode_wide().collect();
    let text = target.to_string_lossy();
    let substitute: Vec<u16> = match text.strip_prefix(r"\\?\") {
        Some(rest) => format!(r"\??\{rest}"),
        None => format!(r"\??\{text}"),
    }
    .encode_utf16()
    .collect();
    let (sub_len, print_len) = (substitute.len() * 2, print.len() * 2);
    let path_bytes = sub_len + 2 + print_len + 2;
    let mut buffer: Vec<u8> = Vec::with_capacity(16 + path_bytes);
    buffer.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer.extend_from_slice(&((8 + path_bytes) as u16).to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    for value in [0, sub_len, sub_len + 2, print_len] {
        buffer.extend_from_slice(&(value as u16).to_le_bytes());
    }
    for unit in substitute.iter().chain(&[0]).chain(&print).chain(&[0]) {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }

    std::fs::create_dir(link)?;
    let wide: Vec<u16> = link.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is a NUL-terminated path; the handle is closed below.
    let handle = unsafe {
        CreateFileW(wide.as_ptr(), GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, std::ptr::null_mut())
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = std::io::Error::last_os_error();
        let _ = std::fs::remove_dir(link);
        return Err(error);
    }
    let mut returned = 0u32;
    // SAFETY: `buffer` holds a whole REPARSE_DATA_BUFFER of the size given.
    let ok = unsafe {
        DeviceIoControl(handle, FSCTL_SET_REPARSE_POINT, buffer.as_ptr().cast(), buffer.len() as u32, std::ptr::null_mut(), 0, &mut returned, std::ptr::null_mut())
    };
    let error = std::io::Error::last_os_error();
    // SAFETY: the handle came from CreateFileW above.
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        let _ = std::fs::remove_dir(link);
        return Err(error);
    }
    Ok(())
}

/// No junctions outside Windows: a symbolic link to the folder does it.
#[cfg(not(windows))]
fn junction(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_links_and_junctions_are_made_where_far_puts_them() {
        let dir = std::env::temp_dir().join(format!("farsh-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        std::fs::create_dir_all(dir.join("into")).unwrap();
        std::fs::write(dir.join("file.txt"), "x").unwrap();
        // into a folder that exists: the source's name; else the name typed
        assert_eq!(target(&dir.join("file.txt"), "into", &dir), dir.join("into").join("file.txt"));
        assert_eq!(target(&dir.join("file.txt"), "other.txt", &dir), dir.join("other.txt"));
        create(&dir.join("file.txt"), &dir.join("hard.txt"), LinkType::Hard).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("hard.txt")).unwrap(), "x");
        assert!(create(&dir.join("folder"), &dir.join("h2"), LinkType::Hard).is_err(), "no hard link to a folder");
        assert!(create(&dir.join("file.txt"), &dir.join("hard.txt"), LinkType::Hard).is_err(), "it exists");
        create(&dir.join("folder"), &dir.join("junction"), LinkType::Junction).unwrap();
        std::fs::write(dir.join("folder").join("inside"), "y").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("junction").join("inside")).unwrap(), "y", "the junction leads to the folder");
        std::fs::remove_dir(dir.join("junction")).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
