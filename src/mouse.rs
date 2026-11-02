//! A mouse event as the bytes a terminal sends to a program that asked for the mouse.

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// The report for `event` if the program's `mode` wants it, in its `encoding`.
pub fn encode(event: MouseEvent, mode: MouseProtocolMode, encoding: MouseProtocolEncoding) -> Option<Vec<u8>> {
    let button = |b: MouseButton| match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    // (button code, is a release)
    let (mut code, release) = match event.kind {
        MouseEventKind::Down(b) => (button(b), false),
        MouseEventKind::Up(b) => (button(b), true),
        MouseEventKind::Drag(b) => (button(b) + 32, false),
        MouseEventKind::Moved => (35, false),
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::ScrollLeft => (66, false),
        MouseEventKind::ScrollRight => (67, false),
    };
    let wanted = match mode {
        MouseProtocolMode::None => false,
        MouseProtocolMode::Press => matches!(event.kind, MouseEventKind::Down(_) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown),
        MouseProtocolMode::PressRelease => !matches!(event.kind, MouseEventKind::Drag(_) | MouseEventKind::Moved),
        MouseProtocolMode::ButtonMotion => !matches!(event.kind, MouseEventKind::Moved),
        MouseProtocolMode::AnyMotion => true,
    };
    if !wanted {
        return None;
    }
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }
    let (col, row) = (event.column as u32 + 1, event.row as u32 + 1);
    Some(match encoding {
        MouseProtocolEncoding::Sgr => format!("\x1b[<{code};{col};{row}{}", if release { 'm' } else { 'M' }).into_bytes(),
        MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
            // A release has no button in the old encodings; positions are one byte (UTF-8: a char).
            let code = if release { 3 + (code & !3) } else { code } + 32;
            let mut out = b"\x1b[M".to_vec();
            for value in [code as u32, col + 32, row + 32] {
                if encoding == MouseProtocolEncoding::Utf8 {
                    out.extend(char::from_u32(value).unwrap_or(' ').to_string().into_bytes());
                } else if value <= 255 {
                    out.push(value as u8);
                } else {
                    return None;
                }
            }
            out
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }
    }

    #[test]
    fn sgr_reports_press_and_release_with_one_based_positions() {
        let sgr = MouseProtocolEncoding::Sgr;
        let mode = MouseProtocolMode::PressRelease;
        assert_eq!(encode(event(MouseEventKind::Down(MouseButton::Left), 4, 9), mode, sgr), Some(b"\x1b[<0;5;10M".to_vec()));
        assert_eq!(encode(event(MouseEventKind::Up(MouseButton::Left), 4, 9), mode, sgr), Some(b"\x1b[<0;5;10m".to_vec()));
        assert_eq!(encode(event(MouseEventKind::ScrollDown, 0, 0), mode, sgr), Some(b"\x1b[<65;1;1M".to_vec()));
    }

    #[test]
    fn the_mode_decides_what_is_reported() {
        let sgr = MouseProtocolEncoding::Sgr;
        let drag = event(MouseEventKind::Drag(MouseButton::Left), 1, 1);
        let moved = event(MouseEventKind::Moved, 1, 1);
        let up = event(MouseEventKind::Up(MouseButton::Left), 1, 1);
        assert_eq!(encode(drag, MouseProtocolMode::None, sgr), None);
        assert_eq!(encode(up, MouseProtocolMode::Press, sgr), None);
        assert_eq!(encode(drag, MouseProtocolMode::PressRelease, sgr), None);
        assert!(encode(drag, MouseProtocolMode::ButtonMotion, sgr).is_some());
        assert_eq!(encode(moved, MouseProtocolMode::ButtonMotion, sgr), None);
        assert_eq!(encode(moved, MouseProtocolMode::AnyMotion, sgr), Some(b"\x1b[<35;2;2M".to_vec()));
    }

    #[test]
    fn modifiers_add_to_the_button_code() {
        let mut ev = event(MouseEventKind::Down(MouseButton::Right), 0, 0);
        ev.modifiers = KeyModifiers::SHIFT | KeyModifiers::CONTROL;
        assert_eq!(
            encode(ev, MouseProtocolMode::PressRelease, MouseProtocolEncoding::Sgr),
            Some(b"\x1b[<22;1;1M".to_vec())
        );
    }

    #[test]
    fn the_old_encoding_is_one_byte_per_value_and_has_no_button_on_release() {
        let default = MouseProtocolEncoding::Default;
        let mode = MouseProtocolMode::PressRelease;
        assert_eq!(
            encode(event(MouseEventKind::Down(MouseButton::Left), 0, 0), mode, default),
            Some(vec![0x1b, b'[', b'M', 32, 33, 33])
        );
        assert_eq!(
            encode(event(MouseEventKind::Up(MouseButton::Left), 0, 0), mode, default),
            Some(vec![0x1b, b'[', b'M', 35, 33, 33])
        );
        assert_eq!(encode(event(MouseEventKind::Down(MouseButton::Left), 300, 0), mode, default), None);
    }
}
