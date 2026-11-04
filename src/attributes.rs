//! Ctrl+A: reading and setting what Far's "Attributes" dialog shows
//! (setattr.cpp): the file attributes, the four times, the owner and where a
//! link points. The dialog itself is `Dialog::attributes`.
//!
//! On Linux and macOS there are no such attributes: "Read only" is the write
//! permission bits, "Hidden" is a leading dot (shown, not set), the rest is
//! not there; the times are modification, birth (not settable), access and
//! status change (not settable).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};

pub const READONLY: u32 = 0x1;
pub const HIDDEN: u32 = 0x2;
pub const SYSTEM: u32 = 0x4;
pub const DIRECTORY: u32 = 0x10;
pub const ARCHIVE: u32 = 0x20;
pub const TEMPORARY: u32 = 0x100;
pub const SPARSE: u32 = 0x200;
pub const REPARSE_POINT: u32 = 0x400;
pub const COMPRESSED: u32 = 0x800;
pub const OFFLINE: u32 = 0x1000;
pub const NOT_INDEXED: u32 = 0x2000;
pub const ENCRYPTED: u32 = 0x4000;

/// The dialog's check boxes in Far's order (AttributeMap): the attribute
/// and its label.
pub const MAIN: [(u32, &str, &str); 10] = [
    (READONLY, "readonly", "&Read only"),
    (ARCHIVE, "archive", "&Archive"),
    (HIDDEN, "hidden", "&Hidden"),
    (SYSTEM, "system", "&System"),
    (COMPRESSED, "compressed", "&Compressed"),
    (ENCRYPTED, "encrypted", "&Encrypted"),
    (NOT_INDEXED, "notindexed", "Not &indexed"),
    (SPARSE, "sparse", "&Sparse"),
    (TEMPORARY, "temporary", "&Temporary"),
    (OFFLINE, "offline", "&Offline"),
];

/// The "More" dialog's check boxes (the rest of Far's AttributeMap).
pub const ADVANCED: [(u32, &str, &str); 10] = [
    (REPARSE_POINT, "reparse", "&Reparse point"),
    (0x1_0000, "virtual", "&Virtual"),
    (0x8000, "integrity", "&Integrity stream"),
    (0x2_0000, "noscrub", "&No scrub data"),
    (0x8_0000, "pinned", "&Pinned"),
    (0x10_0000, "unpinned", "&Unpinned"),
    (0x4_0000, "recallopen", "R&ecall on open"),
    (0x40_0000, "recallaccess", "Rec&all on data access"),
    (0x2000_0000, "sequential", "Strictly se&quential"),
    (0x40, "device", "Device"),
];

/// Which attributes this system can change.
pub fn settable(attribute: u32) -> bool {
    cfg!(windows) || attribute == READONLY
}

/// Write, creation, access, change: Far's TimeMap order.
pub type Times = [Option<SystemTime>; 4];

/// What a file is, as the dialog shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub attributes: u32,
    pub times: Times,
    pub owner: String,
    /// A link: Far's label for it (`&Junction:`, `S&ymlink:`) and where it points.
    pub link: Option<(String, String)>,
}

impl Info {
    pub fn is_dir(&self) -> bool {
        self.attributes & DIRECTORY != 0
    }
}

/// What to do to a file: attributes to set and to clear, the date and time
/// fields as typed (`None` keeps the file's; a blank part of one takes the
/// file's own, as Far's construct_time does), an owner to give it.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub set: u32,
    pub clear: u32,
    pub times: [Option<(String, String)>; 4],
    pub format: Format,
    pub owner: Option<String>,
}

impl Default for Change {
    fn default() -> Self {
        Self { set: 0, clear: 0, times: Default::default(), format: Format::system(), owner: None }
    }
}

impl Change {
    fn is_empty(&self) -> bool {
        self.set == 0 && self.clear == 0 && self.times.iter().all(Option::is_none) && self.owner.is_none()
    }
}

/// Applies `change` to `path`, and with `subfolders` to everything inside it.
pub fn apply_tree(path: &Path, change: &Change, subfolders: bool) -> Result<(), String> {
    apply(path, change)?;
    if subfolders && path.is_dir() && !path.is_symlink() {
        let entries = std::fs::read_dir(path).map_err(|error| format!("{}: {error}", path.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("{}: {error}", path.display()))?;
            apply_tree(&entry.path(), change, true)?;
        }
    }
    Ok(())
}

/// Applies `change` to the one file `path` (what Far's process_single_file does).
pub fn apply(path: &Path, change: &Change) -> Result<(), String> {
    if change.is_empty() {
        return Ok(());
    }
    let info = read(path).map_err(|error| format!("Cannot set attributes for\n{}\n{error}", path.display()))?;
    let wanted = (info.attributes | change.set) & !change.clear;
    platform::set_attributes(path, info.attributes, wanted)?;
    let mut times: Times = [None; 4];
    for (i, typed) in change.times.iter().enumerate() {
        let Some((date, time)) = typed else { continue };
        let wanted = change.format.parse(date, time, info.times[i]).map_err(|()| format!("Cannot set file time for\n{}\nWrong date or time", path.display()))?;
        times[i] = wanted.filter(|t| Some(*t) != info.times[i]);
    }
    if times.iter().any(Option::is_some) {
        platform::set_times(path, &times).map_err(|error| format!("Cannot set file time for\n{}\n{error}", path.display()))?;
    }
    if let Some(owner) = change.owner.as_deref().filter(|owner| !owner.is_empty() && *owner != info.owner) {
        platform::set_owner(path, owner).map_err(|error| format!("Cannot set owner for\n{}\n{error}", path.display()))?;
    }
    Ok(())
}

pub fn read(path: &Path) -> std::io::Result<Info> {
    platform::read(path)
}

/// The Windows "Properties" sheet of `path` (Far's "System properties").
pub fn system_properties(path: &Path) -> Result<(), String> {
    platform::system_properties(path)
}

pub fn system_properties_available() -> bool {
    cfg!(windows)
}

// ---------------------------------------------------------------- dates and times

/// How the user's locale writes dates and times (Far's `locale`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub order: DateOrder,
    pub date_separator: char,
    pub time_separator: char,
    pub decimal: char,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateOrder {
    Mdy,
    Dmy,
    Ymd,
}

impl Format {
    pub fn system() -> Self {
        platform::format()
    }

    /// The date field's mask (`99.99.9999N`) and its title (`DD.MM.YYYYY`).
    pub fn date_mask(&self) -> (String, String) {
        let s = self.date_separator;
        match self.order {
            DateOrder::Ymd => (format!("N9999{s}99{s}99"), format!("YYYYY{s}MM{s}DD")),
            DateOrder::Dmy => (format!("99{s}99{s}9999N"), format!("DD{s}MM{s}YYYYY")),
            DateOrder::Mdy => (format!("99{s}99{s}9999N"), format!("MM{s}DD{s}YYYYY")),
        }
    }

    /// The time field's mask (`99:99:99,9999999`) and its title (`hh:mm:ss`).
    pub fn time_mask(&self) -> (String, String) {
        let (s, d) = (self.time_separator, self.decimal);
        (format!("99{s}99{s}99{d}9999999"), format!("hh{s}mm{s}ss"))
    }

    /// `time` as the two fields show it, in local time, to 100 ns.
    pub fn to_strings(&self, time: SystemTime) -> (String, String) {
        let local: DateTime<Local> = time.into();
        let s = self.date_separator;
        let date = match self.order {
            DateOrder::Ymd => format!(" {}{s}{}{s}{}", local.format("%Y"), local.format("%m"), local.format("%d")),
            DateOrder::Dmy => format!("{}{s}{}{s}{} ", local.format("%d"), local.format("%m"), local.format("%Y")),
            DateOrder::Mdy => format!("{}{s}{}{s}{} ", local.format("%m"), local.format("%d"), local.format("%Y")),
        };
        let (t, d) = (self.time_separator, self.decimal);
        let ticks = local.nanosecond() % 1_000_000_000 / 100;
        let time = format!("{}{t}{}{t}{}{d}{ticks:07}", local.format("%H"), local.format("%M"), local.format("%S"));
        (date, time)
    }

    /// The fields back to a time (Far's construct_time): a blank field takes
    /// the `original`'s part, both blank keep the original. `Err` when what
    /// is typed is not a date or time.
    pub fn parse(&self, date: &str, time: &str, original: Option<SystemTime>) -> Result<Option<SystemTime>, ()> {
        let blank = |text: &str| text.chars().all(|c| !c.is_ascii_digit());
        if blank(date) && blank(time) {
            return Ok(original);
        }
        let original: Option<DateTime<Local>> = original.map(Into::into);
        let numbers = |text: &str| -> Vec<u32> {
            text.split(|c: char| !c.is_ascii_digit()).filter(|part| !part.is_empty()).filter_map(|part| part.parse().ok()).collect()
        };
        let day = if blank(date) {
            original.ok_or(())?.date_naive()
        } else {
            let n = numbers(date);
            let [a, b, c] = n[..] else { return Err(()) };
            let (y, m, d) = match self.order {
                DateOrder::Ymd => (a, b, c),
                DateOrder::Dmy => (c, b, a),
                DateOrder::Mdy => (c, a, b),
            };
            NaiveDate::from_ymd_opt(y as i32, m, d).ok_or(())?
        };
        let clock = if blank(time) {
            original.ok_or(())?.time()
        } else {
            // the fraction's blank places are zeros: `,12` is 0.12 s
            let fraction: String = time.split(self.decimal).nth(1).unwrap_or("").chars().map(|c| if c.is_ascii_digit() { c } else { '0' }).collect();
            let whole = time.split(self.decimal).next().unwrap_or("");
            let parts = numbers(whole);
            let (h, m, s) = match parts[..] {
                [h, m, s] => (h, m, s),
                [h, m] => (h, m, 0),
                [h] => (h, 0, 0),
                _ => return Err(()),
            };
            let ticks: u32 = format!("{fraction:0<7}")[..7].parse().map_err(|_| ())?;
            NaiveTime::from_hms_nano_opt(h, m, s, ticks * 100).ok_or(())?
        };
        let local = Local.from_local_datetime(&NaiveDateTime::new(day, clock)).earliest().ok_or(())?;
        Ok(Some(local.into()))
    }
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW};
    use windows_sys::Win32::Security::{LookupAccountNameW, LookupAccountSidW, OWNER_SECURITY_INFORMATION, PSID, SID_NAME_USE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, DecryptFileW, EncryptFileW, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, FileBasicInfo,
        GetFileInformationByHandleEx, OPEN_EXISTING, SetFileAttributesW, SetFileInformationByHandle,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{FILE_SET_SPARSE_BUFFER, FSCTL_DELETE_REPARSE_POINT, FSCTL_SET_COMPRESSION, FSCTL_SET_SPARSE};

    /// What SetFileAttributes sets; the rest each has its own call.
    /// (Far's ESetFileAttributes: what it ignores, it ignores.)
    const SIMPLE: u32 = !(DIRECTORY | COMPRESSED | ENCRYPTED | SPARSE | REPARSE_POINT | 0x40);
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const TAG_MOUNT_POINT: u32 = 0xA000_0003;
    const TAG_SYMLINK: u32 = 0xA000_000C;
    const TAG_LX_SYMLINK: u32 = 0xA000_001D;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: opened by `open`, closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open(path: &Path, access: u32) -> std::io::Result<Handle> {
        let name = wide(path);
        // SAFETY: `name` is NUL-terminated.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE { Err(std::io::Error::last_os_error()) } else { Ok(Handle(handle)) }
    }

    fn check(ok: i32) -> std::io::Result<()> {
        if ok == 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    }

    /// 100 ns ticks since 1601 to a time; 0 is "none".
    fn from_ticks(ticks: i64) -> Option<SystemTime> {
        const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
        if ticks <= 0 {
            return None;
        }
        let since = ticks - UNIX_EPOCH_TICKS;
        let duration = std::time::Duration::from_nanos(since.unsigned_abs() * 100);
        if since >= 0 { SystemTime::UNIX_EPOCH.checked_add(duration) } else { SystemTime::UNIX_EPOCH.checked_sub(duration) }
    }

    fn to_ticks(time: Option<SystemTime>) -> i64 {
        const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
        match time {
            None => 0,
            Some(time) => match time.duration_since(SystemTime::UNIX_EPOCH) {
                Ok(after) => UNIX_EPOCH_TICKS + (after.as_nanos() / 100) as i64,
                Err(before) => UNIX_EPOCH_TICKS - (before.duration().as_nanos() / 100) as i64,
            },
        }
    }

    fn basic_info(handle: &Handle) -> std::io::Result<FILE_BASIC_INFO> {
        // SAFETY: plain data, filled in by the call.
        let mut info: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is as big as the class needs.
        check(unsafe { GetFileInformationByHandleEx(handle.0, FileBasicInfo, (&raw mut info).cast(), std::mem::size_of::<FILE_BASIC_INFO>() as u32) })?;
        Ok(info)
    }

    pub fn read(path: &Path) -> std::io::Result<Info> {
        let handle = open(path, FILE_READ_ATTRIBUTES)?;
        let basic = basic_info(&handle)?;
        let attributes = basic.FileAttributes;
        let times = [from_ticks(basic.LastWriteTime), from_ticks(basic.CreationTime), from_ticks(basic.LastAccessTime), from_ticks(basic.ChangeTime)];
        let link = (attributes & REPARSE_POINT != 0).then(|| link_of(path)).flatten();
        Ok(Info { attributes, times, owner: owner(path).unwrap_or_default(), link })
    }

    /// Far's label for a link and where it points (symlink_type, NormalizeSymlinkName).
    fn link_of(path: &Path) -> Option<(String, String)> {
        let tag = crate::listing::read(path.parent()?)
            .into_iter()
            .find(|entry| Some(std::ffi::OsStr::new(&entry.name)) == path.file_name())
            .and_then(|entry| entry.reparse)?;
        let label = match tag {
            TAG_MOUNT_POINT => "&Junction:".to_string(),
            TAG_SYMLINK => "S&ymlink:".to_string(),
            TAG_LX_SYMLINK => "S&ymlink LX:".to_string(),
            _ => return None,
        };
        let target = std::fs::read_link(path).ok()?;
        let text = target.to_string_lossy();
        let text = text.strip_prefix(r"\\?\").filter(|rest| !rest.starts_with("UNC")).map(str::to_string).unwrap_or_else(|| text.to_string());
        Some((label, text))
    }

    /// `DOMAIN\name` of the owner (Far's GetFileOwner).
    fn owner(path: &Path) -> Option<String> {
        let name = wide(path);
        let mut sid: PSID = std::ptr::null_mut();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated; the out pointers are valid.
        let status = unsafe {
            GetNamedSecurityInfoW(name.as_ptr(), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, &mut sid, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut descriptor)
        };
        if status != 0 {
            return None;
        }
        let mut account = [0u16; 256];
        let mut domain = [0u16; 256];
        let (mut account_len, mut domain_len) = (account.len() as u32, domain.len() as u32);
        let mut kind: SID_NAME_USE = 0;
        // SAFETY: `sid` points into `descriptor`, alive until LocalFree below.
        let found = unsafe {
            LookupAccountSidW(std::ptr::null(), sid, account.as_mut_ptr(), &mut account_len, domain.as_mut_ptr(), &mut domain_len, &mut kind)
        };
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(descriptor) };
        if found == 0 {
            return None;
        }
        let account = String::from_utf16_lossy(&account[..account_len as usize]);
        let domain = String::from_utf16_lossy(&domain[..domain_len as usize]);
        Some(if domain.is_empty() { account } else { format!("{domain}\\{account}") })
    }

    pub fn set_owner(path: &Path, owner: &str) -> std::io::Result<()> {
        let account: Vec<u16> = owner.encode_utf16().chain(Some(0)).collect();
        let mut sid = [0u8; 68];
        let mut sid_len = sid.len() as u32;
        let mut domain = [0u16; 256];
        let mut domain_len = domain.len() as u32;
        let mut kind: SID_NAME_USE = 0;
        // SAFETY: the buffers are as long as their lengths say.
        check(unsafe {
            LookupAccountNameW(std::ptr::null(), account.as_ptr(), sid.as_mut_ptr().cast(), &mut sid_len, domain.as_mut_ptr(), &mut domain_len, &mut kind)
        })?;
        let name = wide(path);
        // SAFETY: `name` is NUL-terminated, `sid` a valid SID.
        let status = unsafe {
            SetNamedSecurityInfoW(name.as_ptr(), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, sid.as_mut_ptr().cast(), std::ptr::null_mut(), std::ptr::null(), std::ptr::null())
        };
        if status == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(status as i32)) }
    }

    pub fn set_attributes(path: &Path, old: u32, new: u32) -> Result<(), String> {
        let fail = |what: &str, error: std::io::Error| format!("{what}\n{}\n{error}", path.display());
        let name = wide(path);
        // compression and encryption first: a read-only file refuses them
        if (old ^ new) & COMPRESSED != 0 {
            let handle = open(path, GENERIC_READ | GENERIC_WRITE).map_err(|e| fail("Cannot set attribute COMPRESSED for", e))?;
            let format: u16 = if new & COMPRESSED != 0 { 1 } else { 0 };
            let mut returned = 0u32;
            // SAFETY: `format` is the USHORT the control code takes.
            check(unsafe {
                DeviceIoControl(handle.0, FSCTL_SET_COMPRESSION, (&raw const format).cast(), 2, std::ptr::null_mut(), 0, &mut returned, std::ptr::null_mut())
            })
            .map_err(|e| fail("Cannot set attribute COMPRESSED for", e))?;
        }
        if (old ^ new) & ENCRYPTED != 0 {
            // SAFETY: `name` is NUL-terminated.
            let ok = unsafe { if new & ENCRYPTED != 0 { EncryptFileW(name.as_ptr()) } else { DecryptFileW(name.as_ptr(), 0) } };
            check(ok).map_err(|e| fail("Cannot set attribute ENCRYPTED for", e))?;
        }
        if (old ^ new) & SPARSE != 0 {
            let handle = open(path, GENERIC_READ | GENERIC_WRITE).map_err(|e| fail("Cannot set attribute SPARSE for", e))?;
            let buffer = FILE_SET_SPARSE_BUFFER { SetSparse: new & SPARSE != 0 };
            let mut returned = 0u32;
            // SAFETY: `buffer` is the structure the control code takes.
            check(unsafe {
                DeviceIoControl(handle.0, FSCTL_SET_SPARSE, (&raw const buffer).cast(), std::mem::size_of::<FILE_SET_SPARSE_BUFFER>() as u32, std::ptr::null_mut(), 0, &mut returned, std::ptr::null_mut())
            })
            .map_err(|e| fail("Cannot set attribute SPARSE for", e))?;
        }
        // a reparse point is only ever removed (the "More" dialog)
        if old & REPARSE_POINT != 0 && new & REPARSE_POINT == 0 {
            let tag = crate::listing::read(path.parent().unwrap_or(path))
                .into_iter()
                .find(|entry| Some(std::ffi::OsStr::new(&entry.name)) == path.file_name())
                .and_then(|entry| entry.reparse)
                .unwrap_or(0);
            let handle = open(path, GENERIC_READ | GENERIC_WRITE).map_err(|e| fail("Cannot remove reparse point for", e))?;
            // REPARSE_DATA_BUFFER's header: the tag, no data
            let mut header = [0u8; 8];
            header[..4].copy_from_slice(&tag.to_le_bytes());
            let mut returned = 0u32;
            // SAFETY: `header` is the 8-byte header the control code takes.
            check(unsafe {
                DeviceIoControl(handle.0, FSCTL_DELETE_REPARSE_POINT, header.as_ptr().cast(), 8, std::ptr::null_mut(), 0, &mut returned, std::ptr::null_mut())
            })
            .map_err(|e| fail("Cannot remove reparse point for", e))?;
        }
        if (old ^ new) & SIMPLE != 0 {
            let simple = new & SIMPLE;
            // SAFETY: `name` is NUL-terminated.
            check(unsafe { SetFileAttributesW(name.as_ptr(), if simple == 0 { FILE_ATTRIBUTE_NORMAL } else { simple }) })
                .map_err(|e| fail("Cannot set attributes for", e))?;
        }
        Ok(())
    }

    pub fn set_times(path: &Path, times: &Times) -> std::io::Result<()> {
        let handle = open(path, FILE_WRITE_ATTRIBUTES)?;
        // 0 leaves a time as it is, and the attributes too
        let info = FILE_BASIC_INFO {
            LastWriteTime: to_ticks(times[0]),
            CreationTime: to_ticks(times[1]),
            LastAccessTime: to_ticks(times[2]),
            ChangeTime: to_ticks(times[3]),
            FileAttributes: 0,
        };
        // SAFETY: `info` is the structure the class takes.
        check(unsafe { SetFileInformationByHandle(handle.0, FileBasicInfo, (&raw const info).cast(), std::mem::size_of::<FILE_BASIC_INFO>() as u32) })
    }

    pub fn system_properties(path: &Path) -> Result<(), String> {
        use windows_sys::Win32::UI::Shell::{SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW, ShellExecuteExW};
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW;
        let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let file = wide(&full);
        let verb: Vec<u16> = "properties".encode_utf16().chain(Some(0)).collect();
        // SAFETY: plain data; the fields that matter are set below.
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_INVOKEIDLIST;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.nShow = SW_SHOW;
        // SAFETY: `info` and the strings it points to live through the call.
        check(unsafe { ShellExecuteExW(&mut info) }).map_err(|error| format!("{}: {error}", path.display()))
    }

    pub fn format() -> Format {
        use windows_sys::Win32::Globalization::{GetLocaleInfoEx, LOCALE_IDATE, LOCALE_SDATE, LOCALE_SDECIMAL, LOCALE_STIME};
        let get = |kind: u32| -> String {
            let mut buffer = [0u16; 16];
            // SAFETY: the buffer is as long as said; a null name is the user's locale.
            let len = unsafe { GetLocaleInfoEx(std::ptr::null(), kind, buffer.as_mut_ptr(), buffer.len() as i32) };
            String::from_utf16_lossy(&buffer[..(len.max(1) - 1) as usize])
        };
        let first = |kind: u32, default: char| get(kind).chars().next().unwrap_or(default);
        let order = match get(LOCALE_IDATE).as_str() {
            "0" => DateOrder::Mdy,
            "2" => DateOrder::Ymd,
            _ => DateOrder::Dmy,
        };
        Format { order, date_separator: first(LOCALE_SDATE, '.'), time_separator: first(LOCALE_STIME, ':'), decimal: first(LOCALE_SDECIMAL, '.') }
    }
}

// ---------------------------------------------------------------- Linux, macOS

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    pub fn read(path: &Path) -> std::io::Result<Info> {
        let meta = std::fs::symlink_metadata(path)?;
        let target = std::fs::metadata(path).unwrap_or_else(|_| meta.clone());
        let mut attributes = 0;
        if target.is_dir() {
            attributes |= DIRECTORY;
        }
        if target.permissions().mode() & 0o222 == 0 {
            attributes |= READONLY;
        }
        if path.file_name().is_some_and(|name| name.as_bytes().starts_with(b".")) {
            attributes |= HIDDEN;
        }
        let link = meta.file_type().is_symlink().then(|| std::fs::read_link(path).ok()).flatten();
        if link.is_some() {
            attributes |= REPARSE_POINT;
        }
        let at = |secs: i64, nanos: i64| u64::try_from(secs).ok().map(|s| SystemTime::UNIX_EPOCH + std::time::Duration::new(s, nanos as u32));
        let times = [target.modified().ok(), target.created().ok(), target.accessed().ok(), at(target.ctime(), target.ctime_nsec())];
        Ok(Info { attributes, times, owner: user_name(target.uid()), link: link.map(|target| ("S&ymlink:".into(), target.display().to_string())) })
    }

    fn user_name(uid: u32) -> String {
        let mut buffer = vec![0 as libc::c_char; 4096];
        // SAFETY: plain data, filled in by the call.
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        // SAFETY: the buffers are as long as said.
        let status = unsafe { libc::getpwuid_r(uid, &mut entry, buffer.as_mut_ptr(), buffer.len(), &mut result) };
        if status != 0 || result.is_null() {
            return uid.to_string();
        }
        // SAFETY: getpwuid_r points pw_name at a NUL-terminated string in `buffer`.
        unsafe { std::ffi::CStr::from_ptr(entry.pw_name) }.to_string_lossy().into_owned()
    }

    pub fn set_owner(path: &Path, owner: &str) -> std::io::Result<()> {
        let name = CString::new(owner).map_err(std::io::Error::other)?;
        let mut buffer = vec![0 as libc::c_char; 4096];
        // SAFETY: plain data, filled in by the call.
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        // SAFETY: the buffers are as long as said, `name` is NUL-terminated.
        let status = unsafe { libc::getpwnam_r(name.as_ptr(), &mut entry, buffer.as_mut_ptr(), buffer.len(), &mut result) };
        if status != 0 || result.is_null() {
            return Err(std::io::Error::other(format!("no user {owner}")));
        }
        let file = CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
        // SAFETY: `file` is NUL-terminated; -1 leaves the group.
        if unsafe { libc::lchown(file.as_ptr(), entry.pw_uid, u32::MAX) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
    }

    pub fn set_attributes(path: &Path, old: u32, new: u32) -> Result<(), String> {
        if (old ^ new) & READONLY == 0 {
            return Ok(());
        }
        let mut permissions = std::fs::metadata(path).map_err(|e| format!("Cannot set attributes for\n{}\n{e}", path.display()))?.permissions();
        let mode = permissions.mode();
        // read only: no one writes; writable again: the owner writes
        permissions.set_mode(if new & READONLY != 0 { mode & !0o222 } else { mode | 0o200 });
        std::fs::set_permissions(path, permissions).map_err(|e| format!("Cannot set attributes for\n{}\n{e}", path.display()))
    }

    pub fn set_times(path: &Path, times: &Times) -> std::io::Result<()> {
        if times[1].is_some() || times[3].is_some() {
            return Err(std::io::Error::other("only the modification and access times can be set here"));
        }
        let spec = |time: Option<SystemTime>| match time.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()) {
            Some(d) => libc::timespec { tv_sec: d.as_secs() as libc::time_t, tv_nsec: d.subsec_nanos() as _ },
            None => libc::timespec { tv_sec: 0, tv_nsec: libc::UTIME_OMIT },
        };
        let specs = [spec(times[2]), spec(times[0])];
        let file = CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
        // SAFETY: `file` is NUL-terminated, `specs` two timespecs.
        if unsafe { libc::utimensat(libc::AT_FDCWD, file.as_ptr(), specs.as_ptr(), 0) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
    }

    pub fn system_properties(_path: &Path) -> Result<(), String> {
        Err("There are no system properties here".into())
    }

    pub fn format() -> Format {
        Format { order: DateOrder::Dmy, date_separator: '.', time_separator: ':', decimal: ',' }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DMY: Format = Format { order: DateOrder::Dmy, date_separator: '.', time_separator: ':', decimal: ',' };

    #[test]
    fn a_time_goes_to_far_fields_and_back_to_100_ns() {
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::new(1_786_000_000, 954_872_300);
        let (date, clock) = DMY.to_strings(time);
        assert_eq!(date.len(), 11, "{date:?}");
        assert!(date.ends_with(' '), "the N place of DD.MM.YYYYY: {date:?}");
        assert_eq!(clock.len(), 16, "{clock:?}");
        assert!(clock.ends_with(",9548723"), "{clock:?}");
        assert_eq!(DMY.parse(&date, &clock, None), Ok(Some(time)));
    }

    #[test]
    fn blank_fields_keep_the_original_and_junk_is_an_error() {
        let original = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(DMY.parse("  .  .     ", "  :  :  ,       ", Some(original)), Ok(Some(original)));
        let (_, clock) = DMY.to_strings(original);
        let moved = DMY.parse("01.02.2020 ", &clock, Some(original)).unwrap().unwrap();
        let local: DateTime<Local> = moved.into();
        assert_eq!(local.format("%d.%m.%Y").to_string(), "01.02.2020");
        assert_eq!(DMY.parse("31.02.2020 ", &clock, Some(original)), Err(()));
    }

    #[test]
    fn the_masks_are_fars() {
        assert_eq!(DMY.date_mask(), ("99.99.9999N".into(), "DD.MM.YYYYY".into()));
        assert_eq!(DMY.time_mask(), ("99:99:99,9999999".into(), "hh:mm:ss".into()));
    }

    #[test]
    fn read_only_and_a_time_are_set_and_read_back() {
        let dir = std::env::temp_dir().join(format!("farsh-attr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let when = SystemTime::UNIX_EPOCH + std::time::Duration::new(1_600_000_000, 123_456_700);
        let format = Format::system();
        let change = Change { set: READONLY, times: [Some(format.to_strings(when)), None, None, None], format, ..Change::default() };
        apply(&file, &change).unwrap();
        let info = read(&file).unwrap();
        assert!(info.attributes & READONLY != 0);
        assert_eq!(info.times[0], Some(when));
        assert!(!info.owner.is_empty());
        apply(&file, &Change { clear: READONLY, ..Change::default() }).unwrap();
        assert_eq!(read(&file).unwrap().attributes & READONLY, 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

// ---------------------------------------------------------------- the dialog's starting point

/// How Far opens the dialog (setattr.cpp's DialogMode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    File,
    Folder,
    Multiple,
}

/// Everything the dialog starts from and keeps while it is open (Far's
/// SetAttrDlgParam): per attribute of [`MAIN`] then [`ADVANCED`] the
/// initial and current state (0 off, 1 on, 2 leave), whether it has the
/// third state, and whether the user changed it.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    pub paths: Vec<PathBuf>,
    pub mode: Mode,
    pub initial: [u8; 20],
    pub current: [u8; 20],
    pub three: [bool; 20],
    pub manual: [bool; 20],
    /// The date and time fields as the dialog opened (blank where they differ).
    pub dates: [(String, String); 4],
    pub owner: String,
    pub link: Option<(String, String)>,
    pub format: Format,
    /// A folder is among the paths: "Process subfolders" is offered.
    pub folders: bool,
}

impl Setup {
    /// What the dialog opens with for `paths` (setattr.cpp: one file or
    /// folder as it is; several: each attribute on, off or "some").
    pub fn new(paths: Vec<PathBuf>) -> Result<Self, String> {
        let format = Format::system();
        let all: Vec<u32> = MAIN.iter().chain(&ADVANCED).map(|(a, _, _)| *a).collect();
        let infos: Vec<Info> = paths.iter().map(|p| read(p).map_err(|e| format!("{}: {e}", p.display()))).collect::<Result<_, _>>()?;
        let Some(first) = infos.first() else { return Err("Nothing to set attributes for".into()) };
        let strings = |time: Option<SystemTime>| time.map(|t| format.to_strings(t)).unwrap_or_default();
        let mut setup = Setup {
            mode: if paths.len() > 1 { Mode::Multiple } else if first.is_dir() { Mode::Folder } else { Mode::File },
            initial: [0; 20],
            current: [0; 20],
            three: [false; 20],
            manual: [false; 20],
            dates: std::array::from_fn(|i| strings(first.times[i])),
            owner: first.owner.clone(),
            link: if paths.len() == 1 { first.link.clone() } else { None },
            format,
            folders: infos.iter().any(Info::is_dir),
            paths,
        };
        let folder_present = setup.folders;
        for (i, attribute) in all.iter().enumerate() {
            let count = infos.iter().filter(|info| info.attributes & attribute != 0).count();
            setup.initial[i] = if count == infos.len() { 1 } else if count > 0 { 2 } else { 0 };
            setup.three[i] = setup.mode == Mode::Multiple && (folder_present || setup.initial[i] == 2);
        }
        if setup.mode == Mode::Multiple {
            for i in 0..4 {
                if infos.iter().any(|info| info.times[i] != first.times[i]) {
                    setup.dates[i] = Default::default();
                }
            }
            if infos.iter().any(|info| info.owner != first.owner) {
                setup.owner.clear();
            }
        }
        setup.current = setup.initial;
        Ok(setup)
    }

    /// The attribute of state `i`.
    pub fn attribute(i: usize) -> u32 {
        if i < 10 { MAIN[i].0 } else { ADVANCED[i - 10].0 }
    }
}
