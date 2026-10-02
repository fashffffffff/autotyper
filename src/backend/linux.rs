//! Linux-специфика: X11 обязателен, Unicode зависит от раскладки/uinput.
//!
//! `frontmost_pid()` реализован через свойства EWMH: `_NET_ACTIVE_WINDOW`
//! корня + `_NET_WM_PID` целевого окна (использует крейт x11, который и так
//! является зависимостью enigo на Linux).

use std::os::raw::{c_char, c_int, c_long, c_uchar, c_ulong, c_void};

/// PID процесса переднего окна (через EWMH-свойства X11).
/// `None`, если X11 недоступен или свойств нет (например, отдельные WM).
pub fn frontmost_pid() -> Option<i32> {
    unsafe {
        let display = x11::xlib::XOpenDisplay(std::ptr::null());
        if display.is_null() {
            return None;
        }
        // Вся работа с display — в замыкании, чтобы гарантированно закрыть его.
        let result = read_frontmost_pid(display);
        x11::xlib::XCloseDisplay(display);
        result
    }
}

unsafe fn read_frontmost_pid(display: *mut x11::xlib::Display) -> Option<i32> {
    let atom = |name: &[u8]| -> x11::xlib::Atom {
        let mut buf = name.to_vec();
        buf.push(0);
        x11::xlib::XInternAtom(display, buf.as_ptr() as *const c_char, 0)
    };

    let active_atom = atom(b"_NET_ACTIVE_WINDOW");
    if active_atom == 0 {
        return None;
    }
    let root = x11::xlib::XDefaultRootWindow(display);

    // _NET_ACTIVE_WINDOW: одно значение типа XA_WINDOW.
    let mut actual_type: c_ulong = 0;
    let mut actual_format: c_int = 0;
    let mut n_items: c_ulong = 0;
    let mut bytes_after: c_ulong = 0;
    let mut data: *mut c_uchar = std::ptr::null_mut();

    let status = x11::xlib::XGetWindowProperty(
        display,
        root,
        active_atom,
        0,
        1,
        0,
        x11::xlib::XA_WINDOW,
        &mut actual_type,
        &mut actual_format,
        &mut n_items,
        &mut bytes_after,
        &mut data,
    );
    if status != x11::xlib::Success || data.is_null() || n_items == 0 {
        return None;
    }
    // 32-битные свойства X возвращаются массивом c_long — берём первый элемент.
    let window = *(data as *const c_long) as x11::xlib::Window;
    x11::xlib::XFree(data as *mut c_void);
    if window == 0 {
        return None;
    }

    // _NET_WM_PID этого окна: значение типа XA_CARDINAL.
    let pid_atom = atom(b"_NET_WM_PID");
    if pid_atom == 0 {
        return None;
    }
    let mut data: *mut c_uchar = std::ptr::null_mut();
    let status = x11::xlib::XGetWindowProperty(
        display,
        window,
        pid_atom,
        0,
        1,
        0,
        x11::xlib::XA_CARDINAL,
        &mut actual_type,
        &mut actual_format,
        &mut n_items,
        &mut bytes_after,
        &mut data,
    );
    if status != x11::xlib::Success || data.is_null() || n_items == 0 {
        return None;
    }
    let pid = *(data as *const c_long) as i32;
    x11::xlib::XFree(data as *mut c_void);

    if pid > 0 {
        Some(pid)
    } else {
        None
    }
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
