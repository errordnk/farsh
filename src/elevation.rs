//! Far's elevation (elevation.cpp): an operation the system refuses with
//! "access denied" is asked about ("Access denied / You need to provide
//! administrator permission / to delete this object:"), and on OK it is done
//! again by a copy of farsh running as administrator (`farsh --elevation
//! <pipe> <pid>`, started through UAC once and kept for the session), which
//! takes requests over a named pipe only the administrators may open, and
//! only from the farsh that started it.
//!
//! As in Far: "Do this for all current objects" (on by default) asks no more
//! for the rest of the operation, "Do not ask again in the current session"
//! for the rest of the session; Options > System's "Request administrative
//! rights" (System.ElevationMode) says for what: modification (1), reading (2).
//! A farsh already elevated asks nothing. Windows only.

use std::io;
use std::path::{Path, PathBuf};

/// What the object is wanted for (Far's MElevationRequired* lines).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    Delete,
    SetAttributes,
    Recycle,
    List,
    GetAttributes,
}

impl Why {
    pub fn text(self) -> &'static str {
        match self {
            Why::Delete => "to delete this object:",
            Why::SetAttributes => "to set attributes of this object:",
            Why::Recycle => "to move this object to recycle bin:",
            Why::List => "to list this object:",
            Why::GetAttributes => "to get attributes of this object:",
        }
    }

    /// System.ElevationMode's bit: ELEVATION_MODIFY_REQUEST or ELEVATION_READ_REQUEST.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn mode(self) -> i64 {
        match self {
            Why::List | Why::GetAttributes => 2,
            _ => 1,
        }
    }
}

/// How the user answered "Access denied".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Approval {
    pub ok: bool,
    /// "Do this for all current objects".
    pub for_all: bool,
    /// "Do not ask again in the current session".
    pub dont_ask: bool,
}

/// An object's attributes and size (what the elevated side sees).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attributes {
    pub attributes: u32,
    pub size: u64,
}

/// What the elevated farsh is asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    DeleteFile(PathBuf),
    RemoveDirectory(PathBuf),
    SetAttributes(PathBuf, u32),
    Recycle(PathBuf),
    /// The names in a folder.
    List(PathBuf),
    Attributes(PathBuf),
}

impl Request {
    fn why(&self) -> Why {
        match self {
            Request::DeleteFile(_) | Request::RemoveDirectory(_) => Why::Delete,
            Request::SetAttributes(..) => Why::SetAttributes,
            Request::Recycle(_) => Why::Recycle,
            Request::List(_) => Why::List,
            Request::Attributes(_) => Why::GetAttributes,
        }
    }

    fn path(&self) -> &Path {
        match self {
            Request::DeleteFile(p) | Request::RemoveDirectory(p) | Request::SetAttributes(p, _) | Request::Recycle(p) | Request::List(p) | Request::Attributes(p) => p,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Done,
    List(Vec<(String, Attributes)>),
    Attributes(Attributes),
}

/// The errors elevation can help with (Far's ElevationRequired).
pub fn denied(error: &io::Error) -> bool {
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_INVALID_OWNER: i32 = 1307;
    const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;
    matches!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED | ERROR_INVALID_OWNER | ERROR_PRIVILEGE_NOT_HELD))
}

/// A new operation begins (Far's ResetApprove): it is asked about again,
/// unless "Do not ask again" was the answer.
pub fn reset() {
    #[cfg(windows)]
    system::reset();
}

/// `request` failed with `error`: done elevated, after asking with `ask` as
/// Far would. `Ok(None)`: the user said Skip (or did before, for all).
/// Any other refusal (UAC declined, elevation not wanted) is `error` back.
pub fn retry(request: Request, error: io::Error, ask: impl FnOnce(Why, &Path) -> io::Result<Approval>) -> io::Result<Option<Reply>> {
    #[cfg(windows)]
    {
        system::retry(request, error, ask)
    }
    #[cfg(not(windows))]
    {
        let _ = (request, ask);
        Err(error)
    }
}

/// `farsh --elevation <pipe> <pid>`: serves the farsh that started it.
pub fn serve(args: &[String]) -> io::Result<()> {
    #[cfg(windows)]
    {
        system::serve(args)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Err(io::Error::other("elevation is Windows only"))
    }
}

// ---------------------------------------------------------------- the wire

/// A request: op, attributes, path (length, UTF-16).
fn encode(request: &Request) -> Vec<u8> {
    let (op, attributes) = match request {
        Request::DeleteFile(_) => (1u8, 0),
        Request::RemoveDirectory(_) => (2, 0),
        Request::SetAttributes(_, a) => (3, *a),
        Request::Recycle(_) => (4, 0),
        Request::List(_) => (5, 0),
        Request::Attributes(_) => (6, 0),
    };
    let mut out = vec![op];
    out.extend(attributes.to_le_bytes());
    put_text(&mut out, &request.path().to_string_lossy());
    out
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    out.extend((wide.len() as u32).to_le_bytes());
    for unit in wide {
        out.extend(unit.to_le_bytes());
    }
}

trait ReadExt: io::Read {
    fn u8(&mut self) -> io::Result<u8> {
        let mut b = [0; 1];
        self.read_exact(&mut b)?;
        Ok(b[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        let mut b = [0; 4];
        self.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    fn u64(&mut self) -> io::Result<u64> {
        let mut b = [0; 8];
        self.read_exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }
    fn text(&mut self) -> io::Result<String> {
        let len = self.u32()? as usize;
        if len > 65536 {
            return Err(io::Error::other("elevation: text too long"));
        }
        let mut bytes = vec![0; len * 2];
        self.read_exact(&mut bytes)?;
        let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Ok(String::from_utf16_lossy(&wide))
    }
}

impl<R: io::Read + ?Sized> ReadExt for R {}

#[cfg_attr(not(windows), allow(dead_code))]
fn decode(input: &mut impl io::Read) -> io::Result<Request> {
    let op = input.u8()?;
    let attributes = input.u32()?;
    let path = PathBuf::from(input.text()?);
    Ok(match op {
        1 => Request::DeleteFile(path),
        2 => Request::RemoveDirectory(path),
        3 => Request::SetAttributes(path, attributes),
        4 => Request::Recycle(path),
        5 => Request::List(path),
        6 => Request::Attributes(path),
        _ => return Err(io::Error::other("elevation: unknown request")),
    })
}

/// A reply: the error code (0: done), then what was asked for.
#[cfg_attr(not(windows), allow(dead_code))]
fn encode_reply(result: &io::Result<Reply>) -> Vec<u8> {
    let mut out = Vec::new();
    match result {
        Err(error) => out.extend((error.raw_os_error().filter(|&e| e != 0).unwrap_or(31) as u32).to_le_bytes()),
        Ok(reply) => {
            out.extend(0u32.to_le_bytes());
            match reply {
                Reply::Done => {}
                Reply::Attributes(a) => {
                    out.extend(a.attributes.to_le_bytes());
                    out.extend(a.size.to_le_bytes());
                }
                Reply::List(entries) => {
                    out.extend((entries.len() as u32).to_le_bytes());
                    for (name, a) in entries {
                        out.extend(a.attributes.to_le_bytes());
                        out.extend(a.size.to_le_bytes());
                        put_text(&mut out, name);
                    }
                }
            }
        }
    }
    out
}

#[cfg_attr(not(windows), allow(dead_code))]
fn decode_reply(request: &Request, input: &mut impl io::Read) -> io::Result<Reply> {
    let code = input.u32()?;
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    Ok(match request {
        Request::Attributes(_) => Reply::Attributes(Attributes { attributes: input.u32()?, size: input.u64()? }),
        Request::List(_) => {
            let count = input.u32()?;
            let mut entries = Vec::new();
            for _ in 0..count {
                let attributes = input.u32()?;
                let size = input.u64()?;
                entries.push((input.text()?, Attributes { attributes, size }));
            }
            Reply::List(entries)
        }
        _ => Reply::Done,
    })
}

#[cfg(windows)]
mod system {
    use std::ffi::OsStr;
    use std::io::{self, Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::FromRawHandle;
    use std::path::Path;
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0};
    use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows_sys::Win32::Security::{GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, SetFileAttributesW, WriteFile};
    use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
    use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT};
    use windows_sys::Win32::System::Threading::{CreateEventW, GetCurrentProcess, INFINITE, OpenProcessToken, WaitForMultipleObjects};
    use windows_sys::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    use super::{Approval, Attributes, Reply, Request, Why};

    struct Session {
        /// Ask before the next elevated request (Far's m_AskApprove).
        ask: bool,
        /// The last answer was OK (m_IsApproved).
        approved: bool,
        /// "Do not ask again in the current session" (m_DontAskAgain).
        dont_ask: bool,
        helper: Option<Helper>,
    }

    static SESSION: Mutex<Session> = Mutex::new(Session { ask: true, approved: false, dont_ask: false, helper: None });

    fn session() -> std::sync::MutexGuard<'static, Session> {
        SESSION.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn reset() {
        let mut session = session();
        if session.dont_ask {
            return;
        }
        session.ask = true;
        session.approved = false;
    }

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain([0]).collect()
    }

    /// This farsh runs elevated already.
    fn elevated() -> bool {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the current process's pseudo handle; `token` is closed below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        // SAFETY: `elevation` is the size the call is given.
        let ok = unsafe { GetTokenInformation(token, TokenElevation, (&raw mut elevation).cast(), size_of::<TOKEN_ELEVATION>() as u32, &mut len) } != 0;
        // SAFETY: opened above.
        unsafe { CloseHandle(token) };
        ok && elevation.TokenIsElevated != 0
    }

    pub fn retry(request: Request, error: io::Error, ask: impl FnOnce(Why, &Path) -> io::Result<Approval>) -> io::Result<Option<Reply>> {
        let why = request.why();
        if !super::denied(&error) || crate::settings::int("System.ElevationMode") & why.mode() == 0 || elevated() {
            return Err(error);
        }
        let asking = {
            let session = session();
            session.ask && !session.dont_ask
        };
        if asking {
            let approval = ask(why, request.path())?;
            let mut session = session();
            session.ask = !approval.for_all;
            session.approved = approval.ok;
            session.dont_ask = approval.dont_ask;
        }
        let mut session = session();
        if !session.approved {
            return Ok(None);
        }
        if session.helper.as_ref().is_none_or(|helper| !helper.alive()) {
            session.helper = None;
            match Helper::start() {
                Ok(helper) => session.helper = Some(helper),
                Err(_) => {
                    // UAC said no: as Far's ResetApprove, ask again next time
                    if !session.dont_ask {
                        session.ask = true;
                        session.approved = false;
                    }
                    return Err(error);
                }
            }
        }
        let helper = session.helper.as_mut().expect("started above");
        match helper.call(&request) {
            Ok(reply) => Ok(Some(reply)),
            Err(failure) if failure.kind() == io::ErrorKind::BrokenPipe => {
                session.helper = None;
                Err(error)
            }
            Err(failure) => Err(failure),
        }
    }

    /// The elevated farsh and the pipe to it.
    struct Helper {
        pipe: HANDLE,
        process: HANDLE,
        event: HANDLE,
    }

    // SAFETY: the handles are only used under the session's lock.
    unsafe impl Send for Helper {}

    impl Drop for Helper {
        fn drop(&mut self) {
            // SAFETY: owned handles; closing the pipe ends the helper.
            unsafe {
                CloseHandle(self.pipe);
                CloseHandle(self.process);
                CloseHandle(self.event);
            }
        }
    }

    fn broken() -> io::Error {
        io::Error::new(io::ErrorKind::BrokenPipe, "the elevated farsh is gone")
    }

    impl Helper {
        fn start() -> io::Result<Self> {
            let id = std::process::id();
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let name = format!(r"\\.\pipe\farsh-elevation-{id}-{nanos:x}");
            // only the administrators may open it: the elevated farsh, not the user's other programs
            let sddl = wide(OsStr::new("D:P(A;;GA;;;BA)"));
            let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            // SAFETY: a valid SDDL string; the descriptor is freed below.
            if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let attributes = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
            let pipe_name = wide(OsStr::new(&name));
            // SAFETY: plain arguments; the handle is owned by the Helper (or closed on failure).
            let pipe = unsafe {
                CreateNamedPipeW(
                    pipe_name.as_ptr(),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    65536,
                    65536,
                    0,
                    &attributes,
                )
            };
            // SAFETY: allocated by the conversion above.
            unsafe { LocalFree(descriptor as _) };
            if pipe == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: a manual-reset event, owned by the Helper.
            let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
            let mut helper = Helper { pipe, process: std::ptr::null_mut(), event };
            let exe = std::env::current_exe()?;
            let file = wide(exe.as_os_str());
            let parameters = wide(OsStr::new(&format!("--elevation {name} {id}")));
            let verb = wide(OsStr::new("runas"));
            // SAFETY: plain data, filled in below.
            let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
            info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
            info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
            info.lpVerb = verb.as_ptr();
            info.lpFile = file.as_ptr();
            info.lpParameters = parameters.as_ptr();
            info.nShow = SW_HIDE;
            // SAFETY: the strings live to the end of the call.
            if unsafe { ShellExecuteExW(&mut info) } == 0 || info.hProcess.is_null() {
                return Err(io::Error::last_os_error());
            }
            helper.process = info.hProcess;
            helper.connect()?;
            Ok(helper)
        }

        fn alive(&self) -> bool {
            // SAFETY: an owned process handle.
            let state = unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(self.process, 0) };
            state != WAIT_OBJECT_0
        }

        fn overlapped(&self) -> OVERLAPPED {
            // SAFETY: plain data.
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            overlapped.hEvent = self.event;
            overlapped
        }

        /// Waits for the pending I/O, or for the helper to end.
        fn finish(&self, overlapped: &mut OVERLAPPED) -> io::Result<u32> {
            let handles = [self.event, self.process];
            // SAFETY: both handles are owned and open.
            let which = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
            let mut done = 0u32;
            // SAFETY: `overlapped` is the one the I/O was started with.
            if which != WAIT_OBJECT_0 || unsafe { GetOverlappedResult(self.pipe, overlapped, &mut done, 0) } == 0 {
                return Err(broken());
            }
            Ok(done)
        }

        fn connect(&self) -> io::Result<()> {
            let mut overlapped = self.overlapped();
            // SAFETY: the pipe and the event are open.
            if unsafe { ConnectNamedPipe(self.pipe, &mut overlapped) } != 0 {
                return Ok(());
            }
            // SAFETY: plain call.
            match unsafe { GetLastError() } {
                ERROR_PIPE_CONNECTED => Ok(()),
                ERROR_IO_PENDING => self.finish(&mut overlapped).map(|_| ()),
                _ => Err(io::Error::last_os_error()),
            }
        }

        fn call(&mut self, request: &Request) -> io::Result<Reply> {
            self.write_all(&super::encode(request))?;
            super::decode_reply(request, &mut *self)
        }
    }

    impl Read for Helper {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut overlapped = self.overlapped();
            let mut done = 0u32;
            // SAFETY: `buf` and `overlapped` outlive the I/O (it is waited for here).
            if unsafe { ReadFile(self.pipe, buf.as_mut_ptr(), buf.len() as u32, &mut done, &mut overlapped) } != 0 {
                return Ok(done as usize);
            }
            // SAFETY: plain call.
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(broken());
            }
            Ok(self.finish(&mut overlapped)? as usize)
        }
    }

    impl Write for Helper {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut overlapped = self.overlapped();
            let mut done = 0u32;
            // SAFETY: as in `read`.
            if unsafe { WriteFile(self.pipe, buf.as_ptr(), buf.len() as u32, &mut done, &mut overlapped) } != 0 {
                return Ok(done as usize);
            }
            // SAFETY: plain call.
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(broken());
            }
            Ok(self.finish(&mut overlapped)? as usize)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    // ------------------------------------------------------------ the elevated side

    pub fn serve(args: &[String]) -> io::Result<()> {
        let [name, parent] = args else { return Err(io::Error::other("usage: farsh --elevation <pipe> <pid>")) };
        let parent: u32 = parent.parse().map_err(|_| io::Error::other("bad pid"))?;
        let pipe_name = wide(OsStr::new(name));
        // SAFETY: plain arguments; the handle goes to a File below.
        let handle = unsafe { CreateFileW(pipe_name.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a handle just opened, owned by the File from now on.
        let mut pipe = unsafe { std::fs::File::from_raw_handle(handle) };
        // serve only the farsh that started us
        let mut server = 0u32;
        // SAFETY: an open pipe handle.
        if unsafe { GetNamedPipeServerProcessId(handle, &mut server) } == 0 || server != parent {
            return Err(io::Error::other("the pipe is not the parent's"));
        }
        loop {
            let request = match super::decode(&mut pipe) {
                Ok(request) => request,
                // the parent closed the pipe: done
                Err(_) => return Ok(()),
            };
            let reply = run(&request);
            pipe.write_all(&super::encode_reply(&reply))?;
        }
    }

    fn attributes(meta: &std::fs::Metadata) -> Attributes {
        Attributes { attributes: meta.file_attributes(), size: meta.len() }
    }

    fn run(request: &Request) -> io::Result<Reply> {
        match request {
            Request::DeleteFile(path) => std::fs::remove_file(path).map(|_| Reply::Done),
            Request::RemoveDirectory(path) => std::fs::remove_dir(path).map(|_| Reply::Done),
            Request::SetAttributes(path, value) => {
                let path = wide(path.as_os_str());
                // SAFETY: a NUL-terminated path.
                if unsafe { SetFileAttributesW(path.as_ptr(), *value) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Reply::Done)
            }
            Request::Recycle(path) => crate::fileops::recycle_bin(path).map(|_| Reply::Done),
            Request::List(path) => {
                let mut entries = Vec::new();
                for entry in std::fs::read_dir(path)? {
                    let entry = entry?;
                    entries.push((entry.file_name().to_string_lossy().into_owned(), attributes(&entry.metadata()?)));
                }
                Ok(Reply::List(entries))
            }
            Request::Attributes(path) => Ok(Reply::Attributes(attributes(&std::fs::symlink_metadata(path)?))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_replies_go_through_the_wire() {
        for request in [
            Request::DeleteFile(PathBuf::from(r"C:\a b\файл.txt")),
            Request::RemoveDirectory(PathBuf::from("x")),
            Request::SetAttributes(PathBuf::from("y"), 0x20),
            Request::Recycle(PathBuf::from("z")),
            Request::List(PathBuf::from("d")),
            Request::Attributes(PathBuf::from("e")),
        ] {
            assert_eq!(decode(&mut encode(&request).as_slice()).unwrap(), request);
        }
        let list = Request::List(PathBuf::from("d"));
        let reply = Reply::List(vec![("a".into(), Attributes { attributes: 0x10, size: 0 }), ("б.txt".into(), Attributes { attributes: 0x21, size: 5 })]);
        assert_eq!(decode_reply(&list, &mut encode_reply(&Ok(reply.clone())).as_slice()).unwrap(), reply);
        let error = decode_reply(&list, &mut encode_reply(&Err(io::Error::from_raw_os_error(5))).as_slice()).unwrap_err();
        assert!(denied(&error));
        let attributes = Request::Attributes(PathBuf::from("e"));
        let reply = Reply::Attributes(Attributes { attributes: 1, size: 1 << 40 });
        assert_eq!(decode_reply(&attributes, &mut encode_reply(&Ok(reply.clone())).as_slice()).unwrap(), reply);
    }
}
