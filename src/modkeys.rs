//! Shift, Ctrl and Alt held down, for the key bar (Far's KeyBar shows what
//! F1..F12 do with them). crossterm drops the key events of a modifier on
//! its own, so on Windows they are taken off the console's input queue
//! before it reads: [`wait`] waits for input, takes the modifier keys at the
//! head of the queue and notes the modifier state every input record
//! carries; the rest is left to crossterm, one record at a time, so that no
//! modifier key goes to it unseen. A terminal sends these keys in
//! win32-input-mode (Windows Terminal, Som). Elsewhere there is no way to
//! hear them, and the key bar stays plain.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;

static HELD: AtomicU8 = AtomicU8::new(0);

/// The modifiers held now (`SHIFT | CTRL | ALT`).
pub fn held() -> u8 {
    HELD.load(Ordering::Relaxed)
}

fn set(state: u8) -> bool {
    HELD.swap(state, Ordering::Relaxed) != state
}

/// Waits up to `timeout` for input. Returns (input for crossterm is there,
/// the modifiers changed).
pub fn wait(timeout: Duration) -> std::io::Result<(bool, bool)> {
    #[cfg(windows)]
    {
        system::wait(timeout)
    }
    #[cfg(not(windows))]
    {
        Ok((crossterm::event::poll(timeout)?, false))
    }
}

/// The modifiers in a console's control key state.
#[cfg_attr(not(windows), allow(dead_code))]
fn from_control_state(state: u32) -> u8 {
    const RIGHT_ALT: u32 = 0x01;
    const LEFT_ALT: u32 = 0x02;
    const RIGHT_CTRL: u32 = 0x04;
    const LEFT_CTRL: u32 = 0x08;
    const SHIFT_PRESSED: u32 = 0x10;
    let mut held = 0;
    if state & SHIFT_PRESSED != 0 {
        held |= SHIFT;
    }
    if state & (LEFT_CTRL | RIGHT_CTRL) != 0 {
        held |= CTRL;
    }
    if state & (LEFT_ALT | RIGHT_ALT) != 0 {
        held |= ALT;
    }
    held
}

/// The state after a key event of a modifier key `vk` (VK_SHIFT, VK_CONTROL,
/// VK_MENU and their left/right forms), `None` for any other key.
#[cfg_attr(not(windows), allow(dead_code))]
fn after_modifier_key(vk: u16, down: bool, state: u32) -> Option<u8> {
    let bit = match vk {
        0x10 | 0xa0 | 0xa1 => SHIFT,
        0x11 | 0xa2 | 0xa3 => CTRL,
        0x12 | 0xa4 | 0xa5 => ALT,
        _ => return None,
    };
    // the key's own bit as the event says, whatever the state field claims
    let held = from_control_state(state);
    Some(if down { held | bit } else { held & !bit })
}

#[cfg(windows)]
mod system {
    use std::sync::OnceLock;
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{FOCUS_EVENT, INPUT_RECORD, KEY_EVENT, MOUSE_EVENT, PeekConsoleInputW, ReadConsoleInputW};
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    /// The console's input, as crossterm opens it.
    fn input() -> HANDLE {
        static INPUT: OnceLock<usize> = OnceLock::new();
        *INPUT.get_or_init(|| {
            let name: Vec<u16> = "CONIN$\0".encode_utf16().collect();
            const GENERIC_READ_WRITE: u32 = 0x8000_0000 | 0x4000_0000;
            let handle = unsafe {
                CreateFileW(name.as_ptr(), GENERIC_READ_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut())
            };
            handle as usize
        }) as HANDLE
    }

    pub fn wait(timeout: Duration) -> std::io::Result<(bool, bool)> {
        let handle = input();
        if handle == INVALID_HANDLE_VALUE {
            return Ok((crossterm::event::poll(timeout)?, false));
        }
        let millis = timeout.as_millis().min(u32::MAX as u128 - 1) as u32;
        if unsafe { WaitForSingleObject(handle, millis) } != WAIT_OBJECT_0 {
            return Ok((false, false));
        }
        let mut changed = false;
        loop {
            let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
            let mut count = 0u32;
            if unsafe { PeekConsoleInputW(handle, &mut record, 1, &mut count) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if count == 0 {
                return Ok((false, changed));
            }
            match u32::from(record.EventType) {
                KEY_EVENT => {
                    let key = unsafe { record.Event.KeyEvent };
                    let down = key.bKeyDown != 0;
                    if let Some(held) = super::after_modifier_key(key.wVirtualKeyCode, down, key.dwControlKeyState) {
                        changed |= super::set(held);
                        // crossterm would only throw it away
                        let mut taken = 0u32;
                        unsafe { ReadConsoleInputW(handle, &mut record, 1, &mut taken) };
                        continue;
                    }
                    changed |= super::set(super::from_control_state(key.dwControlKeyState));
                }
                MOUSE_EVENT => {
                    let mouse = unsafe { record.Event.MouseEvent };
                    changed |= super::set(super::from_control_state(mouse.dwControlKeyState));
                }
                // keys let go elsewhere are never heard of
                FOCUS_EVENT => changed |= super::set(0),
                _ => {}
            }
            return Ok((true, changed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_keys_and_control_states() {
        assert_eq!(after_modifier_key(0x11, true, 0x08), Some(CTRL));
        // a key's own event wins over a state that does not show it yet
        assert_eq!(after_modifier_key(0x10, true, 0x08), Some(CTRL | SHIFT));
        assert_eq!(after_modifier_key(0x12, false, 0x02 | 0x10), Some(SHIFT));
        assert_eq!(after_modifier_key(0x70, true, 0), None);
        assert_eq!(from_control_state(0x04 | 0x01), CTRL | ALT);
        assert_eq!(from_control_state(0x100), 0);
    }
}
