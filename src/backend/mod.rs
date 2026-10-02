//! Платформенно-зависимая обвязка.
//!
//! Весь ввод и весь перехват делают крейты enigo/rdev, поэтому настоящий
//! платформенный код здесь небольшой:
//! * `environment_warnings()` — диагностика окружения на старте (права,
//!   X11/Wayland и т.п.), чтобы пользователь сразу видел вероятные проблемы;
//! * `frontmost_pid()` — PID процесса переднего окна; нужен для проверки
//!   фокуса при возобновлении паузы (уровень 1) и для арминга фоновой цели.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!("autotyper поддерживает только macOS, Windows и Linux");

/// Предупреждения о текущем окружении (выводятся в stderr при старте).
pub fn environment_warnings() -> Vec<&'static str> {
    #[cfg(target_os = "macos")]
    {
        macos::warnings()
    }
    #[cfg(target_os = "windows")]
    {
        windows::warnings()
    }
    #[cfg(target_os = "linux")]
    {
        linux::warnings()
    }
}

/// PID процесса, владеющего передним (активным) окном.
/// `None` = определить не удалось — вызывающий код просто пропускает
/// проверку фокуса с предупреждением.
pub fn frontmost_pid() -> Option<i32> {
    #[cfg(target_os = "macos")]
    {
        macos::frontmost_pid()
    }
    #[cfg(target_os = "windows")]
    {
        windows::frontmost_pid()
    }
    #[cfg(target_os = "linux")]
    {
        linux::frontmost_pid()
    }
}
