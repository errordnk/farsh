//! farsh as the user's shell (Linux/macOS: `/etc/shells` and passwd).
//!
//! Which shell is the real one, and what farsh does when it is started the way
//! a login shell is: as `-farsh` (what login and sshd do for a session), with
//! `-c cmd` (ssh, scp, rsync, `make`, vim's `:!`), with a script, or without a
//! terminal. Anything but an interactive session on a terminal is the real
//! shell's job: farsh just becomes it (`exec`), with the same arguments, so
//! nothing is printed and no panels exist. The real shell is never farsh.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static LOGIN: AtomicBool = AtomicBool::new(false);

/// This session was started as a login shell (`-farsh`, `-l`, `--login`).
pub fn login() -> bool {
    LOGIN.load(Ordering::Relaxed)
}

#[cfg_attr(not(unix), allow(dead_code))]
pub fn set_login(login: bool) {
    LOGIN.store(login, Ordering::Relaxed);
}

/// Whether farsh was started as a login shell: its name begins with `-`, or
/// it was told so.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn is_login(argv0: &str, args: &[String]) -> bool {
    let name = Path::new(argv0).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    name.starts_with('-') || args.iter().any(|a| a == "-l" || a == "--login")
}

/// Whether the arguments ask for something other than an interactive session
/// (`-c cmd`, a script, `-s`, anything farsh does not know): that is the real
/// shell's. `-l`, `--login` and `-i` are fine.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn wants_plain_shell(args: &[String]) -> bool {
    args.iter().any(|a| !matches!(a.as_str(), "-l" | "--login" | "-i"))
}

/// Whether `program` is farsh itself, by name or by being this very file.
pub fn is_farsh(program: &str) -> bool {
    let stem = Path::new(program).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    if stem == "farsh" {
        return true;
    }
    match (std::fs::canonicalize(program), std::env::current_exe().and_then(std::fs::canonicalize)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The shell the user chose, if any: `$FARSH_SHELL`, the profile's
/// `Shell.Program`, or `$SHELL`, the first that is set and is not farsh (with
/// farsh as the login shell `$SHELL` is farsh).
pub fn chosen_shell() -> Option<String> {
    [
        std::env::var("FARSH_SHELL").unwrap_or_default(),
        crate::settings::string("Shell.Program"),
        std::env::var("SHELL").unwrap_or_default(),
    ]
    .into_iter()
    .find(|shell| !shell.is_empty() && !is_farsh(shell))
}

/// Becomes the real shell, with `args`, as a login shell if `login`. Returns
/// only if that failed.
#[cfg(unix)]
pub fn exec_real_shell(login: bool, args: &[String]) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let shell = crate::shell::shell_program();
    let mut command = std::process::Command::new(&shell);
    if login {
        let name = Path::new(&shell).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        command.arg0(format!("-{name}"));
    }
    // what runs under it finds the real shell in $SHELL, not farsh
    command.args(args).env("SHELL", &shell).exec()
}

/// A session that is a login shell must not end because farsh broke: on a
/// panic the terminal is put right and the real shell takes over.
#[cfg(unix)]
pub fn install_panic_fallback() {
    use std::io::Write;
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[?1049l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?25h\x1b[m\r\n");
        let _ = out.flush();
        default(info);
        let error = exec_real_shell(true, &[]);
        eprintln!("farsh: cannot start {}: {error}", crate::shell::shell_program());
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn login_is_a_leading_dash_or_a_flag() {
        assert!(is_login("-farsh", &[]));
        assert!(is_login("/home/u/.local/bin/-farsh", &[]));
        assert!(is_login("farsh", &args(&["-l"])));
        assert!(is_login("farsh", &args(&["--login"])));
        assert!(!is_login("farsh", &[]));
    }

    #[test]
    fn only_a_plain_interactive_session_is_farshs() {
        assert!(!wants_plain_shell(&[]));
        assert!(!wants_plain_shell(&args(&["-l", "-i"])));
        assert!(wants_plain_shell(&args(&["-c", "echo hi"])));
        assert!(wants_plain_shell(&args(&["script.sh"])));
        assert!(wants_plain_shell(&args(&["-l", "-c", "x"])));
    }

    #[test]
    fn farsh_is_never_its_own_shell() {
        assert!(is_farsh("/home/u/.local/bin/farsh"));
        assert!(is_farsh("farsh.exe"));
        assert!(!is_farsh("/bin/bash"));
    }
}
