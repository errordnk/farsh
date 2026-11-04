//! A key press as the bytes a terminal sends for it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn encode(key: KeyEvent) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut bytes = match key.code {
        KeyCode::Char(c) if ctrl && c.is_ascii_alphabetic() => vec![c.to_ascii_lowercase() as u8 - b'a' + 1],
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => function_key(n)?,
        _ => return None,
    };
    if alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

/// The Russian (ЙЦУКЕН) letters in the order of the QWERTY keys they share.
const RUSSIAN: &str = "йцукенгшщзхъфывапролджэячсмитьбюё";
const QWERTY: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.`";

/// The character of the same key on a QWERTY keyboard, for a Russian
/// letter (the case kept): Far's Xlat / KeyToKeyLayout, so that a hot key
/// or shortcut works whatever the layout.
pub fn latin(c: char) -> Option<char> {
    let lower = c.to_lowercase().next().unwrap_or(c);
    let at = RUSSIAN.chars().position(|r| r == lower)?;
    let l = QWERTY.chars().nth(at)?;
    Some(if c != lower { l.to_ascii_uppercase() } else { l })
}

/// With Ctrl or Alt held, a Russian letter is the Latin one of its key
/// (Ctrl+Ц is Ctrl+W), as a terminal sends it in any layout.
pub fn normalize(mut key: KeyEvent) -> KeyEvent {
    if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        && let KeyCode::Char(c) = key.code
        && let Some(l) = latin(c)
    {
        key.code = KeyCode::Char(l);
    }
    key
}

/// Whether the typed `c` is the hot key `hot`, in either layout.
pub fn same(c: char, hot: char) -> bool {
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    lower(c) == lower(hot) || latin(c).is_some_and(|l| lower(l) == lower(hot))
}

fn function_key(n: u8) -> Option<Vec<u8>> {
    Some(match n {
        1 => b"\x1bOP".to_vec(),
        2 => b"\x1bOQ".to_vec(),
        3 => b"\x1bOR".to_vec(),
        4 => b"\x1bOS".to_vec(),
        5 => b"\x1b[15~".to_vec(),
        6 => b"\x1b[17~".to_vec(),
        7 => b"\x1b[18~".to_vec(),
        8 => b"\x1b[19~".to_vec(),
        9 => b"\x1b[20~".to_vec(),
        10 => b"\x1b[21~".to_vec(),
        11 => b"\x1b[23~".to_vec(),
        12 => b"\x1b[24~".to_vec(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_layout_is_the_latin_keys() {
        assert_eq!(latin('ц'), Some('w'));
        assert_eq!(latin('Ш'), Some('I'));
        assert_eq!(latin('w'), None);
        let ctrl_w = normalize(KeyEvent::new(KeyCode::Char('ц'), KeyModifiers::CONTROL));
        assert_eq!(ctrl_w.code, KeyCode::Char('w'));
        let typed = normalize(KeyEvent::new(KeyCode::Char('ц'), KeyModifiers::NONE));
        assert_eq!(typed.code, KeyCode::Char('ц'), "plain letters stay what they are");
        assert!(same('а', 'f') && same('F', 'f') && !same('а', 'a'));
        // the F9 menu and a dialog's hot keys
        let mut menu = crate::menu::Menu::new(0, [2, 2], [(crate::panels::Kind::Files, true, true); 2]);
        menu.key(KeyEvent::new(KeyCode::Char('с'), KeyModifiers::NONE));
        assert_eq!(menu.current(), 2, "с is the C of Commands");
        let mut dialog = crate::dialog::Dialog::select_group(true, "*.*");
        let mut vmenu = crate::vmenu::VMenu::new("ArcLite", vec!["&Test archive files".into()]);
        assert_eq!(vmenu.key(KeyEvent::new(KeyCode::Char('е'), KeyModifiers::NONE)), crate::vmenu::Outcome::Chosen(0));
        assert!(matches!(dialog.key(KeyEvent::new(KeyCode::Char('ф'), KeyModifiers::ALT)), crate::dialog::Outcome::Open));
    }
}
