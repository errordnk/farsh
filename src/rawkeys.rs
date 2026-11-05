//! The console's input as the terminal sent it, for the shell: while the panels
//! are hidden farsh does not interpret keys, it passes every input record on to
//! the shell's ConPTY in win32-input-mode (`CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`),
//! which ConPTY turns back into the same record. The shell gets what it would
//! get without farsh: key releases, modifier keys alone, scan codes, AltGr.
//! Mouse records become the reports the shell's program asked for, and a
//! buffer-size record a resize. Windows only; elsewhere the terminal's own
//! bytes are the shell's input anyway.

use std::sync::OnceLock;
use std::time::Duration;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::Console::{
    GetNumberOfConsoleInputEvents, INPUT_RECORD, KEY_EVENT, MOUSE_EVENT, MOUSE_EVENT_RECORD, ReadConsoleInputW,
    WINDOW_BUFFER_SIZE_EVENT,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

const RIGHT_ALT: u32 = 0x01;
const LEFT_ALT: u32 = 0x02;
const RIGHT_CTRL: u32 = 0x04;
const LEFT_CTRL: u32 = 0x08;
const SHIFT_PRESSED: u32 = 0x10;

/// One console input record that matters to the shell.
pub enum Record {
    Key { vk: u16, sc: u16, uc: u16, down: bool, state: u32, repeat: u16 },
    Mouse(MouseEvent),
    Resize,
}

impl Record {
    /// Ctrl+O, pressed (not Alt, not Shift): farsh's key for the panels.
    pub fn is_ctrl_o_down(&self) -> bool {
        matches!(self, Record::Key { vk: 0x4F, down: true, state, .. }
            if state & (LEFT_CTRL | RIGHT_CTRL) != 0 && state & (LEFT_ALT | RIGHT_ALT | SHIFT_PRESSED) == 0)
    }

    /// Whether this is the Enter key being pressed.
    pub fn is_enter_down(&self) -> bool {
        matches!(self, Record::Key { vk: 0x0D, down: true, .. })
    }

    /// Whether the key is one of Shift, Ctrl, Alt, Caps/Num/Scroll Lock, which
    /// type nothing.
    pub fn is_modifier(&self) -> bool {
        matches!(self, Record::Key { vk: 0x10 | 0x11 | 0x12 | 0x14 | 0x90 | 0x91 | 0xA0..=0xA5, .. })
    }

    /// The bytes the shell's ConPTY reads as this key.
    pub fn win32_input(&self) -> Option<Vec<u8>> {
        match self {
            Record::Key { vk, sc, uc, down, state, repeat } => {
                Some(format!("\x1b[{vk};{sc};{uc};{};{state};{}_", u8::from(*down), (*repeat).max(1)).into_bytes())
            }
            _ => None,
        }
    }
}

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

/// Waits up to `timeout` for input; whether there is some.
pub fn wait(timeout: Duration) -> std::io::Result<bool> {
    let handle = input();
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let millis = timeout.as_millis().min(u32::MAX as u128 - 1) as u32;
    Ok(unsafe { WaitForSingleObject(handle, millis) } == WAIT_OBJECT_0)
}

/// Reads records one at a time, remembering the mouse buttons held.
#[derive(Default)]
pub struct Reader {
    buttons: u32,
}

impl Reader {
    /// The next record that matters, or None when the queue is empty.
    pub fn read(&mut self) -> Option<Record> {
        let handle = input();
        loop {
            let mut waiting = 0u32;
            if unsafe { GetNumberOfConsoleInputEvents(handle, &mut waiting) } == 0 || waiting == 0 {
                return None;
            }
            let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
            let mut taken = 0u32;
            if unsafe { ReadConsoleInputW(handle, &mut record, 1, &mut taken) } == 0 || taken == 0 {
                return None;
            }
            match u32::from(record.EventType) {
                KEY_EVENT => {
                    let key = unsafe { record.Event.KeyEvent };
                    return Some(Record::Key {
                        vk: key.wVirtualKeyCode,
                        sc: key.wVirtualScanCode,
                        uc: unsafe { key.uChar.UnicodeChar },
                        down: key.bKeyDown != 0,
                        state: key.dwControlKeyState,
                        repeat: key.wRepeatCount,
                    });
                }
                MOUSE_EVENT => {
                    let mouse = unsafe { record.Event.MouseEvent };
                    if let Some(event) = self.mouse(mouse) {
                        return Some(Record::Mouse(event));
                    }
                }
                WINDOW_BUFFER_SIZE_EVENT => return Some(Record::Resize),
                _ => {}
            }
        }
    }

    fn mouse(&mut self, record: MOUSE_EVENT_RECORD) -> Option<MouseEvent> {
        const MOVED: u32 = 0x1;
        const WHEELED: u32 = 0x4;
        const HWHEELED: u32 = 0x8;
        let state = record.dwButtonState;
        let mut modifiers = KeyModifiers::NONE;
        if record.dwControlKeyState & SHIFT_PRESSED != 0 {
            modifiers |= KeyModifiers::SHIFT;
        }
        if record.dwControlKeyState & (LEFT_CTRL | RIGHT_CTRL) != 0 {
            modifiers |= KeyModifiers::CONTROL;
        }
        if record.dwControlKeyState & (LEFT_ALT | RIGHT_ALT) != 0 {
            modifiers |= KeyModifiers::ALT;
        }
        let button = |bits: u32| match bits {
            b if b & 0x1 != 0 => MouseButton::Left,
            b if b & 0x2 != 0 => MouseButton::Right,
            _ => MouseButton::Middle,
        };
        let flags = record.dwEventFlags;
        let kind = if flags & WHEELED != 0 {
            if (state >> 16) as i16 > 0 { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown }
        } else if flags & HWHEELED != 0 {
            if (state >> 16) as i16 > 0 { MouseEventKind::ScrollRight } else { MouseEventKind::ScrollLeft }
        } else if flags & MOVED != 0 {
            match state & 0x7 {
                0 => MouseEventKind::Moved,
                held => MouseEventKind::Drag(button(held)),
            }
        } else {
            let held = state & 0x7;
            let changed = held ^ self.buttons;
            self.buttons = held;
            if changed == 0 {
                return None;
            }
            if held & changed != 0 {
                MouseEventKind::Down(button(held & changed))
            } else {
                MouseEventKind::Up(button(changed))
            }
        };
        if flags & (WHEELED | HWHEELED | MOVED) != 0 {
            self.buttons = state & 0x7;
        }
        Some(MouseEvent { kind, column: record.dwMousePosition.X.max(0) as u16, row: record.dwMousePosition.Y.max(0) as u16, modifiers })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_sent_in_win32_input_mode() {
        let key = Record::Key { vk: 0x41, sc: 0x1e, uc: 97, down: true, state: 0, repeat: 1 };
        assert_eq!(key.win32_input(), Some(b"\x1b[65;30;97;1;0;1_".to_vec()));
        let up = Record::Key { vk: 0x0D, sc: 0x1c, uc: 13, down: false, state: 0x10, repeat: 0 };
        assert_eq!(up.win32_input(), Some(b"\x1b[13;28;13;0;16;1_".to_vec()));
    }

    #[test]
    fn ctrl_o_is_ctrl_without_alt_or_shift() {
        let key = |state| Record::Key { vk: 0x4F, sc: 0x18, uc: 15, down: true, state, repeat: 1 };
        assert!(key(LEFT_CTRL).is_ctrl_o_down());
        assert!(key(RIGHT_CTRL).is_ctrl_o_down());
        assert!(!key(0).is_ctrl_o_down());
        assert!(!key(LEFT_CTRL | SHIFT_PRESSED).is_ctrl_o_down());
        assert!(!key(LEFT_CTRL | LEFT_ALT).is_ctrl_o_down());
    }
}
