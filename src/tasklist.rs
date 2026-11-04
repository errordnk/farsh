//! Ctrl+W, Far's task list (plist.cpp): the windows Alt+Tab would show,
//! each as `pid │ title` (F2: the process's name instead), the one farsh
//! runs in checked; Enter brings the window up, Del kills its process
//! after "Kill task", Ctrl+R reads the list again. Outside Windows there
//! are no windows to switch to: the processes are listed (`ps`), F2 has
//! nothing else to show, and Del sends SIGTERM.

#[derive(Clone, Debug)]
pub struct Task {
    pub pid: u32,
    pub title: String,
    pub image: String,
    pub own: bool,
    /// The window to switch to (Windows).
    pub window: isize,
}

/// The menu's items: `{pid:>width} │ {title or image}`.
pub fn entries(tasks: &[Task], show_image: bool) -> Vec<crate::vmenu::Entry> {
    let width = tasks.iter().map(|t| t.pid).max().unwrap_or(0).max(1).ilog10() as usize + 1;
    tasks
        .iter()
        .map(|t| {
            let name = if show_image { if t.image.is_empty() { "???" } else { &t.image } } else { &t.title };
            crate::vmenu::Entry::new(format!("{:>width$} \u{2502} {name}", t.pid)).checked(t.own.then_some('\u{221A}'))
        })
        .collect()
}

#[cfg(windows)]
mod platform {
    use super::Task;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows_sys::Win32::System::Console::GetConsoleWindow;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetLastActivePopup, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, SetForegroundWindow, ShowWindowAsync, GA_ROOTOWNER, GWL_EXSTYLE, SW_RESTORE, WS_EX_TOOLWINDOW,
    };

    /// The window shows in Alt+Tab (the Old New Thing rule Far uses).
    unsafe fn is_alttab_window(window: HWND) -> bool {
        unsafe {
            if IsWindowVisible(window) == 0 {
                return false;
            }
            let mut tried = GetAncestor(window, GA_ROOTOWNER);
            let mut walk: HWND = std::ptr::null_mut();
            while tried != walk {
                walk = tried;
                tried = GetLastActivePopup(walk);
                if IsWindowVisible(tried) != 0 {
                    break;
                }
            }
            if walk != window {
                return false;
            }
            if GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0 {
                return false;
            }
            let mut cloaked: i32 = 0;
            let ok = DwmGetWindowAttribute(window, DWMWA_CLOAKED as u32, (&raw mut cloaked).cast(), 4);
            !(ok >= 0 && cloaked != 0)
        }
    }

    unsafe extern "system" fn collect(window: HWND, param: LPARAM) -> i32 {
        unsafe {
            let windows = &mut *(param as *mut Vec<(HWND, u32)>);
            if is_alttab_window(window) {
                let mut pid = 0;
                if GetWindowThreadProcessId(window, &mut pid) != 0 {
                    windows.push((window, pid));
                }
            }
        }
        1
    }

    fn title(window: HWND) -> String {
        unsafe {
            let len = GetWindowTextLengthW(window);
            let mut buf = vec![0u16; len as usize + 1];
            let got = GetWindowTextW(window, buf.as_mut_ptr(), buf.len() as i32);
            String::from_utf16_lossy(&buf[..got.max(0) as usize])
        }
    }

    /// os::process::get_process_name: the image's file name.
    fn image(pid: u32) -> String {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return String::new();
            }
            let mut buf = vec![0u16; 32768];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(process);
            if ok == 0 {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit('\\').next().unwrap_or(&path).to_string()
        }
    }

    pub fn list() -> Vec<Task> {
        let mut windows: Vec<(HWND, u32)> = Vec::new();
        unsafe {
            EnumWindows(Some(collect), &raw mut windows as LPARAM);
        }
        let (own_pid, console) = unsafe { (GetCurrentProcessId(), GetConsoleWindow()) };
        windows
            .into_iter()
            .map(|(window, pid)| Task { pid, title: title(window), image: image(pid), own: pid == own_pid || window == console, window: window as isize })
            .collect()
    }

    pub fn kill(task: &Task) -> std::io::Result<()> {
        unsafe {
            let process = OpenProcess(PROCESS_TERMINATE, 0, task.pid);
            if process.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let ok = TerminateProcess(process, 0xFFFF_FFFF);
            let error = std::io::Error::last_os_error();
            CloseHandle(process);
            if ok == 0 { Err(error) } else { Ok(()) }
        }
    }

    /// SwitchToWindow.
    pub fn switch_to(task: &Task) {
        let window = task.window as HWND;
        unsafe {
            SetForegroundWindow(window);
            if IsIconic(window) != 0 {
                ShowWindowAsync(window, SW_RESTORE);
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Task;

    pub fn list() -> Vec<Task> {
        let own = std::process::id();
        let output = std::process::Command::new("ps").args(["-e", "-o", "pid=", "-o", "comm="]).output();
        let text = output.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        text.lines()
            .filter_map(|line| {
                let line = line.trim_start();
                let (pid, name) = line.split_once(char::is_whitespace)?;
                let pid: u32 = pid.parse().ok()?;
                let name = name.trim().to_string();
                let image = name.rsplit('/').next().unwrap_or(&name).to_string();
                Some(Task { pid, title: name, image, own: pid == own, window: 0 })
            })
            .collect()
    }

    pub fn kill(task: &Task) -> std::io::Result<()> {
        if unsafe { libc::kill(task.pid as libc::pid_t, libc::SIGTERM) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
    }

    pub fn switch_to(_: &Task) {}
}

pub use platform::{kill, list, switch_to};
