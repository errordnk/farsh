//! The terminal's own bytes, for the shell: while the panels are hidden farsh
//! does not interpret keys, it passes what the terminal sends on to the
//! shell's PTY as it came (escape sequences, kitty keyboard, mouse reports and
//! bracketed paste included; the modes were asked for by the shell's own
//! output, which farsh passes on too). Only Ctrl+O is looked for. Unix only;
//! on Windows the same is done with console records (`rawkeys`).

use std::io;
use std::time::Duration;

/// Waits up to `timeout` for input on stdin; whether there is some.
pub fn wait(timeout: Duration) -> io::Result<bool> {
    let mut fd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
    let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
    let ready = unsafe { libc::poll(&mut fd, 1, millis) };
    if ready < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted { Ok(false) } else { Err(error) };
    }
    Ok(ready > 0)
}

/// What stdin holds now; None when the terminal is gone.
pub fn read() -> io::Result<Option<Vec<u8>>> {
    let mut buffer = [0u8; 4096];
    let count = unsafe { libc::read(0, buffer.as_mut_ptr().cast(), buffer.len()) };
    if count < 0 {
        let error = io::Error::last_os_error();
        return if matches!(error.kind(), io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock) {
            Ok(Some(Vec::new()))
        } else {
            Err(error)
        };
    }
    Ok((count > 0).then(|| buffer[..count as usize].to_vec()))
}

/// Where the first Ctrl+O being pressed is in `bytes`: the byte 0x0F, or its
/// kitty keyboard form (`CSI 111 ; 5 u`, `CSI 111 ; 5 : 1 u`).
pub fn find_ctrl_o(bytes: &[u8]) -> Option<(usize, usize)> {
    const KITTY: &[u8] = b"\x1b[111;5";
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x0f {
            return Some((i, i + 1));
        }
        if bytes[i..].starts_with(KITTY) {
            let rest = &bytes[i + KITTY.len()..];
            for tail in [&b"u"[..], b":1u"] {
                if rest.starts_with(tail) {
                    return Some((i, i + KITTY.len() + tail.len()));
                }
            }
        }
        i += 1;
    }
    None
}

/// Whether the bytes are a key that runs the line: Enter, plain or kitty's.
pub fn has_enter(bytes: &[u8]) -> bool {
    bytes.contains(&b'\r') || bytes.windows(5).any(|w| w == b"\x1b[13u")
}

/// Whether the bytes are a report the terminal made on its own (mouse, focus),
/// not something typed.
pub fn is_report(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x1b[<") || bytes.starts_with(b"\x1b[I") || bytes.starts_with(b"\x1b[O")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_o_in_its_two_forms() {
        assert_eq!(find_ctrl_o(b"ab\x0fcd"), Some((2, 3)));
        assert_eq!(find_ctrl_o(b"\x1b[111;5u"), Some((0, 8)));
        assert_eq!(find_ctrl_o(b"x\x1b[111;5:1uy"), Some((1, 11)));
        // a release, other keys, other modifiers: not a press of Ctrl+O
        assert_eq!(find_ctrl_o(b"\x1b[111;5:3u"), None);
        assert_eq!(find_ctrl_o(b"\x1b[111;6u"), None);
        assert_eq!(find_ctrl_o(b"\x1b[112;5u"), None);
        assert_eq!(find_ctrl_o(b"plain"), None);
    }

    #[test]
    fn enter_and_reports() {
        assert!(has_enter(b"ls\r"));
        assert!(has_enter(b"\x1b[13u"));
        assert!(!has_enter(b"ls"));
        assert!(is_report(b"\x1b[<0;5;5M"));
        assert!(is_report(b"\x1b[I"));
        assert!(!is_report(b"\x1b[A"));
    }
}
