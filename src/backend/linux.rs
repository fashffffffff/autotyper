//! Linux-специфика: X11 обязателен, Unicode зависит от раскладки/uinput.
//!
//! `frontmost_pid()` реализован через свойства EWMH: `_NET_ACTIVE_WINDOW`
//! корневого окна + `_NET_WM_PID` целевого окна. Используется x11rb —
//! чистый Rust-клиент X11 (без системных библиотек и без unsafe;
//! x11rb и так приходит транзитивно через enigo с фичей x11rb).

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

/// PID процесса переднего окна (через EWMH-свойства X11).
/// `None`, если X11 недоступен или свойств нет (например, отдельные WM).
pub fn frontmost_pid() -> Option<i32> {
    // Подключаемся к X-серверу из $DISPLAY.
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;

    // _NET_ACTIVE_WINDOW корня: одно значение типа WINDOW.
    let active_atom = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let reply = conn
        .get_property(false, root, active_atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let window = reply.value32()?.next()?;
    if window == 0 {
        return None; // активного окна нет (например, пустой рабочий стол)
    }

    // _NET_WM_PID этого окна: значение типа CARDINAL.
    let pid_atom = conn
        .intern_atom(false, b"_NET_WM_PID")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let reply = conn
        .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let pid = reply.value32()?.next()? as i32;

    (pid > 0).then_some(pid)
}

/// Предупреждения для stderr на старте (динамические — по переменным
/// окружения сессии) плюс общие статические.
pub fn warnings() -> Vec<&'static str> {
    let mut warnings = Vec::new();

    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        warnings.push(
            "Linux: обнаружена сессия Wayland (WAYLAND_DISPLAY). enigo умеет только X11 \
             (XTest), под Wayland синтез ввода не сработает — войдите в X11-сессию \
             (например, пункт «на Xorg» на экране входа)",
        );
    }
    if std::env::var_os("DISPLAY").is_none() {
        warnings.push(
            "Linux: не задана переменная DISPLAY — X11-сервер не найден, \
             перехват и синтез ввода невозможны",
        );
    }
    warnings.push(
        "Linux: Unicode (кириллица и др.) на X11 зависит от активной раскладки \
         клавиатуры; если символы не вводятся — см. README: раздел про uinput \
         и права на /dev/uinput (группа input)",
    );

    warnings
}
