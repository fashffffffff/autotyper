//! macOS-специфика.
//!
//! Здесь два механизма поверх CoreGraphics/CoreFoundation (прямой FFI,
//! без дополнительных крейтов):
//!
//! 1. `frontmost_pid()` — PID владельца переднего обычного окна через
//!    `CGWindowListCopyWindowInfo` (список идёт от переднего окна к заднему;
//!    PID и layer не требуют разрешения на запись экрана, в отличие от
//!    kCGWindowName). Нужен для проверки фокуса при возобновлении паузы.
//!
//! 2. `PidSink` — приёмник «нажатий» для ФОНОВОЙ печати: события постятся
//!    конкретному процессу через `CGEventPostToPid`, поэтому целевое окно
//!    не обязано быть в фокусе. Текст — через `CGEventKeyboardSetUnicodeString`
//!    (тот же механизм, что использует enigo в фокусном режиме), поэтому
//!    кириллица работает без раскладок.

use std::os::raw::{c_char, c_void};

use anyhow::{bail, Result};

// ---------- FFI-объявления CoreGraphics / CoreFoundation ----------

type CGEventRef = *mut c_void;
type CFAllocatorRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFNumberRef = *const c_void;
type CFStringRef = *const c_void;

/// kCGWindowListOptionOnScreenOnly: только видимые окна, порядок переднее → заднее.
const KCG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
/// kCFNumberSInt32Type (= kCFNumberIntType).
const KCF_NUMBER_SINT32_TYPE: isize = 3;
/// kCFStringEncodingUTF8.
const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

/// kVK_Return — виртуальный код клавиши Enter/Return.
const KVK_RETURN: u16 = 36;
/// kCGEventFlagMaskShift (совпадает с NSEventModifierFlagShift = 1 << 1).
const KCG_EVENT_FLAG_MASK_SHIFT: u64 = 1 << 1;

#[link(name = "CoreGraphics", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    // --- синтез событий ---
    fn CGEventCreateKeyboardEvent(
        source: *const c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> CGEventRef;
    fn CGEventKeyboardSetUnicodeString(event: CGEventRef, length: isize, string: *const u16);
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventPostToPid(pid: i32, event: CGEventRef);
    // --- список окон ---
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    // --- CoreFoundation ---
    fn CFArrayGetCount(the_array: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(the_array: CFArrayRef, idx: isize) -> *const c_void;
    fn CFDictionaryGetValue(the_dict: CFDictionaryRef, key: CFStringRef) -> *const c_void;
    fn CFNumberGetValue(number: CFNumberRef, the_type: isize, value_ptr: *mut c_void) -> bool;
    fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        c_str: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFRelease(cf: *const c_void);
}

/// CFString-ключ для словаря окна (например, "kCGWindowOwnerPID").
unsafe fn window_info_key(name: &str) -> CFStringRef {
    let mut buf = name.to_owned();
    buf.push('\0');
    CFStringCreateWithCString(
        std::ptr::null(),
        buf.as_ptr() as *const c_char,
        KCF_STRING_ENCODING_UTF8,
    )
}

/// PID процесса переднего обычного окна (слой 0; док/меню пропускаются).
pub fn frontmost_pid() -> Option<i32> {
    unsafe {
        let array = CGWindowListCopyWindowInfo(KCG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY, 0);
        if array.is_null() {
            return None;
        }
        let key_pid = window_info_key("kCGWindowOwnerPID");
        let key_layer = window_info_key("kCGWindowLayer");
        let mut result = None;

        for i in 0..CFArrayGetCount(array) {
            let dict = CFArrayGetValueAtIndex(array, i) as CFDictionaryRef;
            if dict.is_null() {
                continue;
            }
            // Пропускаем служебные слои (док, меню и пр.): у обычных окон layer == 0.
            let mut layer: i32 = 0;
            let layer_ref = CFDictionaryGetValue(dict, key_layer);
            if !layer_ref.is_null()
                && CFNumberGetValue(
                    layer_ref,
                    KCF_NUMBER_SINT32_TYPE,
                    &mut layer as *mut i32 as *mut c_void,
                )
                && layer != 0
            {
                continue;
            }
            let pid_ref = CFDictionaryGetValue(dict, key_pid);
            let mut pid: i32 = -1;
            if !pid_ref.is_null()
                && CFNumberGetValue(
                    pid_ref,
                    KCF_NUMBER_SINT32_TYPE,
                    &mut pid as *mut i32 as *mut c_void,
                )
            {
                result = Some(pid);
                break;
            }
        }

        if !key_pid.is_null() {
            CFRelease(key_pid);
        }
        if !key_layer.is_null() {
            CFRelease(key_layer);
        }
        CFRelease(array);
        result
    }
}

/// Приёмник «нажатий» для фоновой печати: постит события конкретному PID,
/// даже когда целевое приложение не в фокусе.
pub struct PidSink {
    pid: i32,
}

impl PidSink {
    pub fn new(pid: i32) -> Self {
        Self { pid }
    }

    /// Отправляет событие клавиши (keydown/keyup) с заданными флагами.
    fn post_key(&self, keycode: u16, key_down: bool, flags: u64) -> Result<()> {
        unsafe {
            let event = CGEventCreateKeyboardEvent(std::ptr::null(), keycode, key_down);
            if event.is_null() {
                bail!("CGEventCreateKeyboardEvent вернула NULL");
            }
            if flags != 0 {
                CGEventSetFlags(event, flags);
            }
            CGEventPostToPid(self.pid, event);
            CFRelease(event);
        }
        Ok(())
    }

    /// Отправляет символ как unicode-событие (keydown + keyup).
    fn post_unicode(&self, units: &[u16], key_down: bool) -> Result<()> {
        unsafe {
            let event = CGEventCreateKeyboardEvent(std::ptr::null(), 0, key_down);
            if event.is_null() {
                bail!("CGEventCreateKeyboardEvent вернула NULL");
            }
            CGEventKeyboardSetUnicodeString(event, units.len() as isize, units.as_ptr());
            CGEventPostToPid(self.pid, event);
            CFRelease(event);
        }
        Ok(())
    }
}

impl crate::typer::KeySink for PidSink {
    fn type_char(&mut self, c: char) -> Result<()> {
        // Символы вне BMP занимают два юнита UTF-16 — CGEvent принимает оба.
        let mut buf = [0u16; 2];
        let units: &[u16] = c.encode_utf16(&mut buf);
        self.post_unicode(units, true)?;
        self.post_unicode(units, false)
    }

    /// Перевод строки: Shift+Return или чистый Return (флаг вешается на само
    /// событие Return — отдельные нажатия Shift не требуются).
    fn press_newline(&mut self, shift: bool) -> Result<()> {
        let flags = if shift { KCG_EVENT_FLAG_MASK_SHIFT } else { 0 };
        self.post_key(KVK_RETURN, true, flags)?;
        self.post_key(KVK_RETURN, false, flags)
    }

    fn press_enter(&mut self) -> Result<()> {
        self.press_newline(false)
    }

}

/// Предупреждения для stderr на старте (подробности — в README, раздел «Права»).
pub fn warnings() -> Vec<&'static str> {
    vec![
        "macOS: требуется System Settings → Privacy & Security → Accessibility — \
         добавьте туда терминал (Terminal/iTerm2/...), из которого запущен autotyper, \
         иначе перехват хоткеев и синтез ввода молча не работают",
        "macOS: если хоткеи не срабатывают, проверьте также \
         Privacy & Security → Input Monitoring для терминала",
    ]
}
