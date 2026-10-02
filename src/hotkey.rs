//! Перехват глобальных хоткеев (rdev) и запуск печати в отдельном потоке.
//!
//! Важно: rdev используется ТОЛЬКО для прослушивания событий клавиатуры.
//! Эмуляция ввода выполняется через KeySink: enigo (фокусный режим, все ОС)
//! либо CGEventPostToPid (фоновый режим, macOS) — rdev не умеет корректный
//! Unicode (кириллицу).
//!
//! Управление во время печати:
//! * panic-хоткей (Esc) — полный СБРОС: печать прекращается и не продолжается;
//! * pause-хоткей (Ctrl+P) — переключатель пауза/продолжение. При
//!   возобновлении выполняется проверка фокуса (уровень 1): если переднее
//!   окно не совпадает с целевым, продолжение отклоняется с подсказкой.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rdev::{Event, EventType, Key as RdevKey};
use thiserror::Error;

use crate::config::Config;
use crate::log::Logger;
use crate::typer::{EnigoSink, KeySink, Typer};

/// Логический модификатор (левая/правая клавиши не различаются).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Meta,
}

/// Ошибки разбора хоткея из конфига.
#[derive(Debug, Error)]
pub enum HotkeyError {
    #[error("неизвестный модификатор: {0:?} (допустимо: ctrl, alt, shift, meta/cmd/win)")]
    UnknownModifier(String),
    #[error("неизвестная клавиша: {0:?} (допустимо: a-z, 0-9, escape, enter, space, tab, p, f1-f12)")]
    UnknownKey(String),
}

/// Разбор имени модификатора из конфига.
pub fn parse_modifier(s: &str) -> Result<Modifier> {
    match s.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Ok(Modifier::Ctrl),
        "alt" | "option" => Ok(Modifier::Alt),
        "shift" => Ok(Modifier::Shift),
        "meta" | "cmd" | "command" | "win" | "super" => Ok(Modifier::Meta),
        other => Err(HotkeyError::UnknownModifier(other.to_string()).into()),
    }
}

/// Разбор имени клавиши из конфига в код rdev.
pub fn parse_key(s: &str) -> Result<RdevKey> {
    let lower = s.trim().to_ascii_lowercase();
    let key = match lower.as_str() {
        "a" => RdevKey::KeyA,
        "b" => RdevKey::KeyB,
        "c" => RdevKey::KeyC,
        "d" => RdevKey::KeyD,
        "e" => RdevKey::KeyE,
        "f" => RdevKey::KeyF,
        "g" => RdevKey::KeyG,
        "h" => RdevKey::KeyH,
        "i" => RdevKey::KeyI,
        "j" => RdevKey::KeyJ,
        "k" => RdevKey::KeyK,
        "l" => RdevKey::KeyL,
        "m" => RdevKey::KeyM,
        "n" => RdevKey::KeyN,
        "o" => RdevKey::KeyO,
        "p" => RdevKey::KeyP,
        "q" => RdevKey::KeyQ,
        "r" => RdevKey::KeyR,
        "s" => RdevKey::KeyS,
        "t" => RdevKey::KeyT,
        "u" => RdevKey::KeyU,
        "v" => RdevKey::KeyV,
        "w" => RdevKey::KeyW,
        "x" => RdevKey::KeyX,
        "y" => RdevKey::KeyY,
        "z" => RdevKey::KeyZ,
        "0" => RdevKey::Num0,
        "1" => RdevKey::Num1,
        "2" => RdevKey::Num2,
        "3" => RdevKey::Num3,
        "4" => RdevKey::Num4,
        "5" => RdevKey::Num5,
        "6" => RdevKey::Num6,
        "7" => RdevKey::Num7,
        "8" => RdevKey::Num8,
        "9" => RdevKey::Num9,
        "escape" | "esc" => RdevKey::Escape,
        "enter" | "return" => RdevKey::Return,
        "space" => RdevKey::Space,
        "tab" => RdevKey::Tab,
        // Символьные клавиши верхнего ряда — на случай, если пользователь
        // хочет паузу/хоткей на них.
        "=" | "equal" => RdevKey::Equal,
        "-" | "minus" => RdevKey::Minus,
        "[" | "left_bracket" => RdevKey::LeftBracket,
        "]" | "right_bracket" => RdevKey::RightBracket,
        ";" | "semicolon" => RdevKey::SemiColon,
        "'" | "quote" => RdevKey::Quote,
        "\\" | "backslash" => RdevKey::BackSlash,
        "," | "comma" => RdevKey::Comma,
        "." | "dot" => RdevKey::Dot,
        "/" | "slash" => RdevKey::Slash,
        "`" | "backquote" => RdevKey::BackQuote,
        "f1" => RdevKey::F1,
        "f2" => RdevKey::F2,
        "f3" => RdevKey::F3,
        "f4" => RdevKey::F4,
        "f5" => RdevKey::F5,
        "f6" => RdevKey::F6,
        "f7" => RdevKey::F7,
        "f8" => RdevKey::F8,
        "f9" => RdevKey::F9,
        "f10" => RdevKey::F10,
        "f11" => RdevKey::F11,
        "f12" => RdevKey::F12,
        other => return Err(HotkeyError::UnknownKey(other.to_string()).into()),
    };
    Ok(key)
}

/// Разобранный хоткей: множество модификаторов + основная клавиша.
#[derive(Debug, Clone)]
pub struct ParsedHotkey {
    pub modifiers: HashSet<Modifier>,
    pub key: RdevKey,
}

impl ParsedHotkey {
    pub fn from_config(hk: &crate::config::HotkeyConfig) -> Result<Self> {
        let mut modifiers = HashSet::new();
        for m in &hk.modifiers {
            modifiers.insert(parse_modifier(m)?);
        }
        let key = parse_key(&hk.key)?;
        Ok(Self { modifiers, key })
    }
}

/// Физическая клавиша rdev -> логический модификатор (если это он).
fn modifier_of(key: RdevKey) -> Option<Modifier> {
    match key {
        RdevKey::ControlLeft | RdevKey::ControlRight => Some(Modifier::Ctrl),
        RdevKey::Alt => Some(Modifier::Alt),
        RdevKey::ShiftLeft | RdevKey::ShiftRight => Some(Modifier::Shift),
        RdevKey::MetaLeft | RdevKey::MetaRight => Some(Modifier::Meta),
        _ => None,
    }
}

/// Общее состояние между callback rdev и печатающими потоками.
struct Inner {
    cfg: Arc<Config>,
    /// Основной хоткей печати (выбран по текущей ОС).
    hotkey: ParsedHotkey,
    /// Опциональный хоткей «напечатать и отправить».
    send_hotkey: Option<ParsedHotkey>,
    /// Хоткей полного сброса печати.
    panic_hotkey: ParsedHotkey,
    /// Хоткей паузы/продолжения (переключатель).
    pause_hotkey: ParsedHotkey,

    /// Сейчас нажатые модификаторы (обновляется в callback rdev).
    pressed_modifiers: Mutex<HashSet<Modifier>>,
    /// Идёт ли печать прямо сейчас (Arc: флаг сбрасывает печатающий поток).
    typing: Arc<AtomicBool>,
    /// Флаг полного сброса; печатающий поток проверяет его на каждой итерации.
    cancel: Arc<AtomicBool>,
    /// Флаг паузы: печатающий поток замирает, пока флаг стоит.
    paused: Arc<AtomicBool>,
    /// Защита от auto-repeat: повторный срабатывание только после
    /// отпускания клавиши (иначе удержание жонглирует паузой).
    main_key_released: AtomicBool,
    send_key_released: AtomicBool,
    pause_key_released: AtomicBool,
    /// PID целевого процесса текущего сеанса: для проверки фокуса при
    /// возобновлении паузы (уровень 1).
    session_target_pid: Mutex<Option<i32>>,
    logger: Arc<Logger>,
}

impl Inner {
    /// Текущий набор удерживаемых модификаторов (пустой при отравленном lock).
    fn current_modifiers(&self) -> HashSet<Modifier> {
        self.pressed_modifiers
            .lock()
            .map(|mods| mods.clone())
            .unwrap_or_default()
    }

    /// Активен ли фоновый режим (постановка событий конкретному PID).
    fn background_active(&self) -> bool {
        self.cfg.background_mode && cfg!(target_os = "macos")
    }

    fn handle_event(&self, event: Event) {
        match event.event_type {
            EventType::KeyPress(key) => self.handle_key_press(key),
            EventType::KeyRelease(key) => self.handle_key_release(key),
            _ => {} // мышь и прочее не интересуют
        }
    }

    fn handle_key_press(&self, key: RdevKey) {
        // Модификаторы только обновляют состояние.
        if let Some(m) = modifier_of(key) {
            if let Ok(mut mods) = self.pressed_modifiers.lock() {
                mods.insert(m);
            }
            return;
        }

        let active = self.current_modifiers();

        // 1. Паник-хоткей: полный сброс — немедленная остановка без продолжения.
        if key == self.panic_hotkey.key && active == self.panic_hotkey.modifiers {
            self.cancel.store(true, Ordering::SeqCst);
            self.logger.log("panic-хоткей: полный СБРОС печати (продолжение невозможно)");
            eprintln!("[autotyper] сброс печати (Esc): остановлено, продолжение невозможно");
            return;
        }

        // 2. Хоткей паузы/продолжения (переключатель).
        if key == self.pause_hotkey.key
            && active == self.pause_hotkey.modifiers
            && self.pause_key_released.load(Ordering::SeqCst)
        {
            self.pause_key_released.store(false, Ordering::SeqCst);
            self.toggle_pause();
            return;
        }

        // 3. Основной хоткей: печать без отправки.
        if key == self.hotkey.key
            && active == self.hotkey.modifiers
            && self.main_key_released.load(Ordering::SeqCst)
            && !self.typing.load(Ordering::SeqCst)
        {
            self.main_key_released.store(false, Ordering::SeqCst);
            self.start_typing(false);
            return;
        }

        // 4. Хоткей «напечатать и отправить» (если настроен).
        if let Some(send) = &self.send_hotkey {
            if key == send.key
                && active == send.modifiers
                && self.send_key_released.load(Ordering::SeqCst)
                && !self.typing.load(Ordering::SeqCst)
            {
                self.send_key_released.store(false, Ordering::SeqCst);
                self.start_typing(true);
            }
        }
    }

    fn handle_key_release(&self, key: RdevKey) {
        if let Some(m) = modifier_of(key) {
            if let Ok(mut mods) = self.pressed_modifiers.lock() {
                mods.remove(&m);
            }
            return;
        }
        if key == self.hotkey.key {
            self.main_key_released.store(true, Ordering::SeqCst);
        }
        if let Some(send) = &self.send_hotkey {
            if key == send.key {
                self.send_key_released.store(true, Ordering::SeqCst);
            }
        }
        if key == self.pause_hotkey.key {
            self.pause_key_released.store(true, Ordering::SeqCst);
        }
    }

    /// Переключатель паузы. Ставит паузу либо снимает её — но снятие
    /// возможно только при подтверждении фокуса (уровень 1).
    fn toggle_pause(&self) {
        if !self.typing.load(Ordering::SeqCst) {
            return; // пауза имеет смысл только во время печати
        }
        if !self.paused.load(Ordering::SeqCst) {
            self.paused.store(true, Ordering::SeqCst);
            self.logger.log("пауза печати");
            eprintln!("[autotyper] пауза (продолжение: {})", self.cfg.pause_hotkey.display());
            return;
        }

        // Возобновление. В фоновом режиме фокус не нужен вовсе.
        if self.background_active() {
            self.paused.store(false, Ordering::SeqCst);
            self.logger.log("печать продолжена (фоновый режим, проверка фокуса не требуется)");
            eprintln!("[autotyper] продолжаю печать");
            return;
        }

        // Фокусный режим: сверяем переднее окно с целевым (уровень 1).
        let expected = self
            .session_target_pid
            .lock()
            .map(|guard| *guard)
            .unwrap_or(None);
        match expected {
            Some(expected_pid) => match crate::backend::frontmost_pid() {
                Some(current) if current == expected_pid => {
                    self.paused.store(false, Ordering::SeqCst);
                    self.logger.log("печать продолжена (фокус подтверждён)");
                    eprintln!("[autotyper] продолжаю печать");
                }
                Some(current) => {
                    // Фокус в другом приложении: НЕ продолжаем, стоим на паузе.
                    let msg = format!(
                        "возобновление отклонено: фокус в другом приложении (pid {current}, \
                         ожидался {expected_pid}). Вернитесь в целевое окно и нажмите паузу ещё раз"
                    );
                    self.logger.log(&msg);
                    eprintln!("[autotyper] {msg}");
                }
                None => {
                    // Не удалось определить фокус — продолжаем с предупреждением,
                    // иначе пользователь застрянет на паузе.
                    self.logger.log(
                        "возобновление: не удалось определить переднее окно, продолжаю без проверки",
                    );
                    self.paused.store(false, Ordering::SeqCst);
                    eprintln!("[autotyper] продолжаю печать (фокус проверить не удалось)");
                }
            },
            None => {
                // Цель сеанса не запомнилась — просто продолжаем.
                self.paused.store(false, Ordering::SeqCst);
                self.logger.log("печать продолжена (целевое окно не запоминалось)");
                eprintln!("[autotyper] продолжаю печать");
            }
        }
    }

    /// Запускает сеанс печати в отдельном потоке (callback rdev должен
    /// возвращаться быстро — печать с задержками ему противопоказана).
    /// Перед запуском определяет цель: фоновый PID (macOS) либо фокусное окно.
    fn start_typing(&self, send_enter_at_end: bool) {
        if self.typing.load(Ordering::SeqCst) {
            return; // уже печатаем — повторный триггер игнорируем
        }

        // Фоновый режим (только macOS): цель — переднее окно В МОМЕНТ хоткея,
        // дальше фокус можно переключать куда угодно.
        #[cfg(target_os = "macos")]
        let background_pid: Option<i32> = if self.cfg.background_mode {
            match crate::backend::macos::frontmost_pid() {
                Some(pid) => Some(pid),
                None => {
                    self.logger.log(
                        "фоновый режим: не удалось определить целевое окно, печать не запущена",
                    );
                    eprintln!(
                        "[autotyper] фоновый режим: не удалось определить целевое окно — \
                         печать не запущена"
                    );
                    return;
                }
            }
        } else {
            None
        };
        #[cfg(not(target_os = "macos"))]
        let background_pid: Option<i32> = None;

        // Для проверки фокуса при возобновлении паузы запоминаем цель сеанса.
        let target_pid = background_pid.or_else(crate::backend::frontmost_pid);
        if let Ok(mut guard) = self.session_target_pid.lock() {
            *guard = target_pid;
        }

        self.typing.store(true, Ordering::SeqCst);
        // Сброс флагов от предыдущего сеанса.
        self.cancel.store(false, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);

        let cfg = Arc::clone(&self.cfg);
        let logger = Arc::clone(&self.logger);
        let cancel = Arc::clone(&self.cancel);
        let paused = Arc::clone(&self.paused);
        let typing = Arc::clone(&self.typing);

        let spawned = std::thread::Builder::new()
            .name("autotyper-session".to_string())
            .spawn(move || {
                type_session(&cfg, &cancel, &paused, send_enter_at_end, background_pid, &logger);
                typing.store(false, Ordering::SeqCst);
                paused.store(false, Ordering::SeqCst);
            });
        if let Err(e) = spawned {
            self.typing.store(false, Ordering::SeqCst);
            self.logger.log(&format!("не удалось запустить поток печати: {e}"));
            eprintln!("[autotyper] не удалось запустить поток печати: {e}");
        }
    }
}

/// Один сеанс: чтение буфера обмена -> печать -> запись статистики в лог.
/// Выполняется в выделенном потоке.
fn type_session(
    cfg: &Arc<Config>,
    cancel: &Arc<AtomicBool>,
    paused: &Arc<AtomicBool>,
    send_enter_at_end: bool,
    background_pid: Option<i32>,
    logger: &Arc<Logger>,
) {
    // Читаем текст из системного буфера обмена.
    let text = match arboard::Clipboard::new().and_then(|mut clip| clip.get_text()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[autotyper] не удалось прочитать буфер обмена: {e}");
            logger.log(&format!("ошибка чтения буфера обмена: {e}"));
            return;
        }
    };
    if text.trim().is_empty() {
        eprintln!("[autotyper] буфер обмена не содержит текста");
        logger.log("буфер обмена не содержит текста — печать пропущена");
        return;
    }

    // Выбираем приёмник ввода: фоновый PID (macOS) либо фокусный enigo.
    let sink: Box<dyn KeySink> = match background_pid {
        #[cfg(target_os = "macos")]
        Some(pid) => Box::new(crate::backend::macos::PidSink::new(pid)),
        #[cfg(target_os = "macos")]
        None => match EnigoSink::new() {
            Ok(s) => Box::new(s),
            Err(e) => {
                eprintln!("[autotyper] {e:#}");
                logger.log(&format!("ошибка инициализации движка ввода: {e:#}"));
                return;
            }
        },
        #[cfg(not(target_os = "macos"))]
        Some(_) => unreachable!("фоновый режим возможен только на macOS (проверен в validate)"),
        #[cfg(not(target_os = "macos"))]
        None => match EnigoSink::new() {
            Ok(s) => Box::new(s),
            Err(e) => {
                eprintln!("[autotyper] {e:#}");
                logger.log(&format!("ошибка инициализации движка ввода: {e:#}"));
                return;
            }
        },
    };

    let mut typer = match Typer::new(Arc::clone(cfg), sink) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[autotyper] {e:#}");
            logger.log(&format!("ошибка инициализации печати: {e:#}"));
            return;
        }
    };

    let total = text.chars().filter(|c| *c != '\r').count();
    let started_str = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
    let mode_descr = match background_pid {
        Some(pid) => format!("фоновый режим, цель pid {pid}"),
        None => "фокусное окно".to_string(),
    };
    logger.log(&format!(
        "старт печати: длина текста {total} символов, {mode_descr}, перевод строки: {}, \
         отправка Enter в конце: {send_enter_at_end}",
        match cfg.newline_mode {
            crate::config::NewlineMode::ShiftEnter => "shift_enter",
            crate::config::NewlineMode::Enter => "enter",
        }
    ));

    match typer.type_text(&text, cancel, paused, send_enter_at_end) {
        Ok(stats) => {
            let secs = stats.elapsed.as_secs_f64();
            let cps = if secs > 0.0 {
                stats.typed_chars as f64 / secs
            } else {
                0.0
            };
            let outcome = if stats.cancelled {
                "СБРОШЕНА panic-хоткеем (прогресс потерян, продолжение невозможно)"
            } else {
                "завершена"
            };
            let ended_str = chrono::Local::now().format("%H:%M:%S%.3f");
            logger.log(&format!(
                "конец печати: {outcome}; введено {}/{} символов за {:.2} c чистого времени \
                 (пауза {:.2} c); средняя фактическая скорость {:.1} симв/с; \
                 начало {started_str}, окончание {ended_str}",
                stats.typed_chars,
                stats.total_chars,
                secs,
                stats.paused.as_secs_f64(),
                cps
            ));
            eprintln!(
                "[autotyper] печать {outcome}: {}/{} символов, {:.1} симв/с",
                stats.typed_chars,
                stats.total_chars,
                cps
            );
        }
        Err(e) => {
            eprintln!("[autotyper] ошибка во время печати: {e:#}");
            logger.log(&format!("ошибка во время печати: {e:#}"));
        }
    }
}

/// Точка входа модуля: разбирает хоткеи из конфига и навсегда уходит в
/// блокирующий `rdev::listen`.
pub fn run(cfg: Arc<Config>, logger: Arc<Logger>) -> Result<()> {
    // Валидацию имён клавиш/модификаторов делаем на старте, а не в момент
    // нажатия — чтобы ошибочный конфиг сразу падал с понятной ошибкой.
    let hotkey = ParsedHotkey::from_config(cfg.active_hotkey())
        .context("некорректный основной хоткей в конфиге")?;
    let send_hotkey = match cfg.type_and_send_hotkey.as_ref() {
        Some(hk) => Some(
            ParsedHotkey::from_config(hk)
                .context("некорректный хоткей type_and_send_hotkey в конфиге")?,
        ),
        None => None,
    };
    let panic_hotkey = ParsedHotkey::from_config(&cfg.panic_hotkey)
        .context("некорректный panic_hotkey в конфиге")?;
    let pause_hotkey = ParsedHotkey::from_config(&cfg.pause_hotkey)
        .context("некорректный pause_hotkey в конфиге")?;

    let inner = Arc::new(Inner {
        cfg,
        hotkey,
        send_hotkey,
        panic_hotkey,
        pause_hotkey,
        pressed_modifiers: Mutex::new(HashSet::new()),
        typing: Arc::new(AtomicBool::new(false)),
        cancel: Arc::new(AtomicBool::new(false)),
        paused: Arc::new(AtomicBool::new(false)),
        main_key_released: AtomicBool::new(true),
        send_key_released: AtomicBool::new(true),
        pause_key_released: AtomicBool::new(true),
        session_target_pid: Mutex::new(None),
        logger,
    });

    let callback_inner = Arc::clone(&inner);
    let callback = move |event: Event| callback_inner.handle_event(event);

    rdev::listen(callback).map_err(|e| {
        anyhow::anyhow!("не удалось начать перехват клавиатуры: {e:?} (проверьте права на доступ)")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HotkeyConfig;

    #[test]
    fn parses_known_keys() {
        assert!(matches!(parse_key("v").expect("v"), RdevKey::KeyV));
        assert!(matches!(parse_key("V").expect("V"), RdevKey::KeyV));
        assert!(matches!(parse_key("p").expect("p"), RdevKey::KeyP));
        assert!(matches!(parse_key("escape").expect("esc"), RdevKey::Escape));
        assert!(matches!(parse_key("esc").expect("esc"), RdevKey::Escape));
        assert!(matches!(parse_key("enter").expect("enter"), RdevKey::Return));
        assert!(matches!(parse_key("5").expect("5"), RdevKey::Num5));
        assert!(matches!(parse_key("f12").expect("f12"), RdevKey::F12));
    }

    #[test]
    fn rejects_unknown_keys_and_modifiers() {
        assert!(parse_key("volup").is_err());
        assert!(parse_key("").is_err());
        assert!(parse_modifier("hyper").is_err());
        assert!(parse_modifier("").is_err());
    }

    /// Символьные клавиши верхнего ряда тоже доступны для хоткеев.
    #[test]
    fn parses_symbol_keys() {
        assert!(matches!(parse_key("=").expect("equal"), RdevKey::Equal));
        assert!(matches!(parse_key("-").expect("minus"), RdevKey::Minus));
        assert!(matches!(parse_key("[").expect("["), RdevKey::LeftBracket));
        assert!(matches!(parse_key("]").expect("]"), RdevKey::RightBracket));
        assert!(matches!(parse_key(";").expect(";"), RdevKey::SemiColon));
        assert!(matches!(parse_key("'").expect("'"), RdevKey::Quote));
        assert!(matches!(parse_key("\\").expect("\\"), RdevKey::BackSlash));
        assert!(matches!(parse_key(",").expect(","), RdevKey::Comma));
        assert!(matches!(parse_key(".").expect("."), RdevKey::Dot));
        assert!(matches!(parse_key("/").expect("/"), RdevKey::Slash));
        assert!(matches!(parse_key("`").expect("`"), RdevKey::BackQuote));
        assert!(matches!(parse_key("equal").expect("equal"), RdevKey::Equal));
    }

    #[test]
    fn parses_aliases_of_modifiers() {
        assert_eq!(parse_modifier("ctrl").expect("ctrl"), Modifier::Ctrl);
        assert_eq!(parse_modifier("Control").expect("Control"), Modifier::Ctrl);
        assert_eq!(parse_modifier("alt").expect("alt"), Modifier::Alt);
        assert_eq!(parse_modifier("Option").expect("Option"), Modifier::Alt);
        assert_eq!(parse_modifier("shift").expect("shift"), Modifier::Shift);
        assert_eq!(parse_modifier("cmd").expect("cmd"), Modifier::Meta);
        assert_eq!(parse_modifier("win").expect("win"), Modifier::Meta);
    }

    #[test]
    fn parsed_hotkey_from_config() {
        let hk = HotkeyConfig {
            modifiers: vec!["ctrl".into(), "shift".into()],
            key: "v".into(),
        };
        let parsed = ParsedHotkey::from_config(&hk).expect("parse");
        assert!(matches!(parsed.key, RdevKey::KeyV));
        assert_eq!(parsed.modifiers.len(), 2);
        assert!(parsed.modifiers.contains(&Modifier::Ctrl));
        assert!(parsed.modifiers.contains(&Modifier::Shift));

        // Пустой список модификаторов валиден (panic на голом Esc,
        // пауза на Ctrl+P).
        let bare = HotkeyConfig {
            modifiers: vec![],
            key: "escape".into(),
        };
        let parsed = ParsedHotkey::from_config(&bare).expect("parse bare");
        assert!(parsed.modifiers.is_empty());
        assert!(matches!(parsed.key, RdevKey::Escape));

        let pause = HotkeyConfig {
            modifiers: vec!["ctrl".into()],
            key: "p".into(),
        };
        let parsed = ParsedHotkey::from_config(&pause).expect("parse pause");
        assert!(matches!(parsed.key, RdevKey::KeyP));
        assert_eq!(parsed.modifiers.len(), 1);
    }
}
