//! Windows-специфика.
//!
//! Фоновая печать в конкретное окно на Windows ненадёжна (современные
//! приложения игнорируют PostMessage(WM_CHAR)), поэтому тут только
//! `frontmost_pid` для проверки фокуса при возобновлении паузы.

use std::os::raw::c_void;

#[link(name = "user32")]
extern "system" {
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(window: *mut c_void, process_id: *mut u32) -> u32;
}

/// PID процесса переднего окна.
pub fn frontmost_pid() -> Option<i32> {
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        if GetWindowThreadProcessId(window, &mut pid) == 0 {
            return None;
        }
        Some(pid as i32)
    }
}

/// Предупреждения для stderr на старте.
pub fn warnings() -> Vec<&'static str> {
    vec![
        "Windows: при работе через RDP синтез ввода действует только пока \
         RDP-сессия активна (не свёрнута/не отключена)",
    ]
}
