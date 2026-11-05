//! Shift, Ctrl and Alt held down, for the key bar (Far's KeyBar shows what
//! F1..F12 do with them). crossterm drops the key events of a modifier on
//! its own, so on Windows they are taken off the console's input queue
//! before it reads: [`wait`] waits for input, takes the modifier keys at the
//! head of the queue and notes the modifier state every input record
//! carries; the rest is left to crossterm, one record at a time, so that no
//! modifier key goes to it unseen. A terminal sends these keys in
//! win32-input-mode (Windows Terminal, Som). Elsewhere there is no way to
//! hear them, and the key bar stays plain.
//!
//! The key bar follows one modifier alone, never two: a modifier shows once
//! it has been held by itself for [`SETTLE`] (the second key of Ctrl+Shift
//! comes a moment after the first), and after two or more held together
//! nothing shows until all of them are let go (they are let go one by one).

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;

/// How long a modifier is held alone before the key bar shows it.
const SETTLE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy)]
struct State {
    /// The modifiers held now.
    held: u8,
    /// What the key bar shows.
    shown: u8,
    /// Two or more were held together since all were last let go.
    chord: bool,
    /// When `held` last changed.
    since: Instant,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

impl State {
    fn new(now: Instant) -> Self {
        Self { held: 0, shown: 0, chord: false, since: now }
    }

    fn set(&mut self, held: u8, now: Instant) {
        if held != self.held {
            self.held = held;
            self.since = now;
        }
        if held.count_ones() > 1 {
            self.chord = true;
        } else if held == 0 {
            self.chord = false;
        }
    }

    /// Settles `shown`; true when it changed.
    fn settle(&mut self, now: Instant) -> bool {
        let shown = if self.chord || self.held.count_ones() != 1 {
            0
        } else if now.duration_since(self.since) >= SETTLE {
            self.held
        } else {
            self.shown & self.held
        };
        std::mem::replace(&mut self.shown, shown) != shown
    }
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(state.get_or_insert_with(|| State::new(Instant::now())))
}

/// The modifier the key bar shows (`SHIFT`, `CTRL`, `ALT` or none).
pub fn held() -> u8 {
    with(|state| state.shown)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn set(held: u8) {
    with(|state| state.set(held, Instant::now()));
}

/// Waits up to `timeout` for input. Returns (input for crossterm is there,
/// what the key bar shows changed).
pub fn wait(timeout: Duration) -> std::io::Result<(bool, bool)> {
    #[cfg(windows)]
    let input = system::wait(timeout)?;
    #[cfg(not(windows))]
    let input = crossterm::event::poll(timeout)?;
    Ok((input, with(|state| state.settle(Instant::now()))))
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

    pub fn wait(timeout: Duration) -> std::io::Result<bool> {
        let handle = input();
        if handle == INVALID_HANDLE_VALUE {
            return crossterm::event::poll(timeout);
        }
        let millis = timeout.as_millis().min(u32::MAX as u128 - 1) as u32;
        if unsafe { WaitForSingleObject(handle, millis) } != WAIT_OBJECT_0 {
            return Ok(false);
        }
        loop {
            let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
            let mut count = 0u32;
            if unsafe { PeekConsoleInputW(handle, &mut record, 1, &mut count) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if count == 0 {
                return Ok(false);
            }
            match u32::from(record.EventType) {
                KEY_EVENT => {
                    let key = unsafe { record.Event.KeyEvent };
                    let down = key.bKeyDown != 0;
                    if let Some(held) = super::after_modifier_key(key.wVirtualKeyCode, down, key.dwControlKeyState) {
                        super::set(held);
                        // crossterm would only throw it away
                        let mut taken = 0u32;
                        unsafe { ReadConsoleInputW(handle, &mut record, 1, &mut taken) };
                        continue;
                    }
                    super::set(super::from_control_state(key.dwControlKeyState));
                }
                MOUSE_EVENT => {
                    let mouse = unsafe { record.Event.MouseEvent };
                    super::set(super::from_control_state(mouse.dwControlKeyState));
                }
                // keys let go elsewhere are never heard of
                FOCUS_EVENT => super::set(0),
                _ => {}
            }
            return Ok(true);
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

    #[test]
    fn one_modifier_alone_shows_two_never() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut state = State::new(t0);
        // Ctrl alone, held long enough
        state.set(CTRL, at(0));
        assert!(!state.settle(at(100)));
        assert!(state.settle(at(150)));
        assert_eq!(state.shown, CTRL);
        state.set(0, at(300));
        assert!(state.settle(at(300)));
        assert_eq!(state.shown, 0);
        // Ctrl, then Shift a moment later, let go one by one: nothing shows
        state.set(CTRL, at(1000));
        state.set(CTRL | SHIFT, at(1060));
        state.set(SHIFT, at(1500));
        assert!(!state.settle(at(2000)));
        state.set(0, at(2100));
        assert!(!state.settle(at(2100)));
        // Ctrl shown, then Alt added: back to plain, and stays plain
        state.set(CTRL, at(3000));
        assert!(state.settle(at(3200)));
        state.set(CTRL | ALT, at(3300));
        assert!(state.settle(at(3300)));
        state.set(CTRL, at(3400));
        assert!(!state.settle(at(4000)));
        assert_eq!(state.shown, 0);
    }
}
