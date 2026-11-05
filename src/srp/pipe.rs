//! The connection to the local `somsrv` daemon (a named pipe on Windows, a Unix
//! socket elsewhere) and its framing: a little-endian u32 length, then the
//! payload. Connects to a daemon that is running; never starts or replaces
//! one (a new `somsrv` ends every session, and farsh lives in one).

use std::io;
use std::sync::Mutex;

/// `-<name>` of `SOMSRV_INSTANCE` (a daemon of its own for tests), else empty.
fn instance_suffix() -> String {
    match std::env::var("SOMSRV_INSTANCE") {
        Ok(name) if !name.is_empty() => format!("-{name}"),
        _ => String::new(),
    }
}

/// Where the daemon listens.
pub fn daemon_address() -> String {
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\somsrv{}", instance_suffix())
    }
    #[cfg(unix)]
    {
        format!("/tmp/somsrv-{}{}.sock", unsafe { libc::getuid() }, instance_suffix())
    }
}

pub struct Connection {
    inner: imp::Inner,
    /// The two writes of a message must not mix with another's.
    write_lock: Mutex<()>,
}

impl Connection {
    pub fn connect(address: &str) -> io::Result<Self> {
        Ok(Self { inner: imp::Inner::connect(address)?, write_lock: Mutex::new(()) })
    }

    pub fn write_message(&self, payload: &[u8]) -> io::Result<()> {
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.write_all(&(payload.len() as u32).to_le_bytes())?;
        self.inner.write_all(payload)
    }

    pub fn read_message(&self) -> io::Result<Vec<u8>> {
        let mut length = [0u8; 4];
        self.inner.read_exact(&mut length)?;
        let mut message = vec![0u8; u32::from_le_bytes(length) as usize];
        self.inner.read_exact(&mut message)?;
        Ok(message)
    }

    /// Ends the connection; a read that waits on another thread fails.
    pub fn close(&self) {
        self.inner.close();
    }
}

#[cfg(unix)]
mod imp {
    use std::io::{self, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::Mutex;

    pub struct Inner {
        read: Mutex<UnixStream>,
        write: Mutex<UnixStream>,
    }

    impl Inner {
        pub fn connect(address: &str) -> io::Result<Self> {
            let stream = UnixStream::connect(address)?;
            Ok(Self { read: Mutex::new(stream.try_clone()?), write: Mutex::new(stream) })
        }

        pub fn write_all(&self, bytes: &[u8]) -> io::Result<()> {
            let mut stream = self.write.lock().unwrap_or_else(|e| e.into_inner());
            stream.write_all(bytes)?;
            stream.flush()
        }

        pub fn read_exact(&self, buffer: &mut [u8]) -> io::Result<()> {
            self.read.lock().unwrap_or_else(|e| e.into_inner()).read_exact(buffer)
        }

        pub fn close(&self) {
            // shutdown works on the socket, not the handle: the blocked read, a clone, fails too
            let _ = self.write.lock().unwrap_or_else(|e| e.into_inner()).shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Overlapped I/O, as the daemon's own pipe: a synchronous handle would hold
/// a write behind a read that waits for Som's request.
#[cfg(windows)]
mod imp {
    use std::io;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, ReadFile, WriteFile,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
    use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};

    const GENERIC_READ_WRITE: u32 = 0x8000_0000 | 0x4000_0000;

    pub struct Inner {
        handle: HANDLE,
    }

    unsafe impl Send for Inner {}
    unsafe impl Sync for Inner {}

    impl Inner {
        pub fn connect(address: &str) -> io::Result<Self> {
            let name: Vec<u16> = address.encode_utf16().chain(std::iter::once(0)).collect();
            let handle = unsafe {
                CreateFileW(name.as_ptr(), GENERIC_READ_WRITE, 0, std::ptr::null(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, std::ptr::null_mut())
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { handle })
        }

        /// One overlapped read or write of up to `length` bytes.
        fn transfer(&self, read: bool, buffer: *mut u8, length: u32) -> io::Result<u32> {
            let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            overlapped.hEvent = event;
            let mut done = 0u32;
            let started = unsafe {
                if read {
                    ReadFile(self.handle, buffer, length, &mut done, &mut overlapped)
                } else {
                    WriteFile(self.handle, buffer, length, &mut done, &mut overlapped)
                }
            };
            let result = if started != 0 {
                Ok(done)
            } else if unsafe { GetLastError() } == ERROR_IO_PENDING {
                unsafe { WaitForSingleObject(event, INFINITE) };
                if unsafe { GetOverlappedResult(self.handle, &overlapped, &mut done, 0) } != 0 {
                    Ok(done)
                } else {
                    Err(io::Error::last_os_error())
                }
            } else {
                Err(io::Error::last_os_error())
            };
            unsafe { CloseHandle(event) };
            match result {
                Ok(0) => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "pipe closed")),
                other => other,
            }
        }

        pub fn write_all(&self, mut bytes: &[u8]) -> io::Result<()> {
            while !bytes.is_empty() {
                let written = self.transfer(false, bytes.as_ptr() as *mut u8, bytes.len() as u32)?;
                bytes = &bytes[written as usize..];
            }
            Ok(())
        }

        pub fn read_exact(&self, mut buffer: &mut [u8]) -> io::Result<()> {
            while !buffer.is_empty() {
                let read = self.transfer(true, buffer.as_mut_ptr(), buffer.len() as u32)?;
                buffer = &mut buffer[read as usize..];
            }
            Ok(())
        }

        /// Cancels the reads and writes that wait; they fail with "aborted".
        pub fn close(&self) {
            unsafe { CancelIoEx(self.handle, std::ptr::null()) };
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }
}
