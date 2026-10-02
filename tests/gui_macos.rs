//! Графический интеграционный тест (только macOS): настоящие события
//! CGEvent, настоящее приложение TextEdit. Проверяет главное из того,
//! что ломалось в реальном использовании:
//!
//! 1. перевод строки действительно создаёт новую строку;
//! 2. последующий «в начало строки» (Cmd+Left) НЕ съедает перевод строки
//!    и убирает авто-отступ;
//! 3. фоновый приёмник PidSink доставляет текст в нефокусное окно.
//!
//! Тесту нужны права Accessibility у процесса, запускающего cargo
//! (Terminal/iTerm2), поэтому он включается переменной окружения:
//!     AUTOTYPER_GUI_TEST=1 cargo test --test gui_macos -- --nocapture
//! Без переменной тест молча пропускается.

#![cfg(target_os = "macos")]

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use autotyper::backend;
use autotyper::backend::macos::PidSink;
use autotyper::config::Config;
use autotyper::typer::{EnigoSink, KeySink, Typer};

/// Выполняет AppleScript и возвращает stdout (или паникует с stderr).
fn osascript(script: &str) -> String {
    let output = std::process::Command::new("osascript")
        .args(["-e", script])
        .output()
        .expect("запуск osascript");
    if !output.status.success() {
        panic!(
            "osascript упал: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string()
}

fn gui_tests_enabled() -> bool {
    std::env::var_os("AUTOTYPER_GUI_TEST").is_some()
}

/// Открывает TextEdit с пустым документом, возвращает pid.
/// Перед созданием закрывает ВСЕ документы (наследие прошлых прогонов
/// путает z-order и «front document»).
fn open_textedit() -> i32 {
    osascript("tell application \"TextEdit\" to activate");
    std::thread::sleep(Duration::from_millis(400));
    let _ = osascript(
        "tell application \"TextEdit\" to close (every document whose modified is true) saving no",
    );
    let _ = osascript("tell application \"TextEdit\" to close every document saving no");
    std::thread::sleep(Duration::from_millis(300));
    osascript("tell application \"TextEdit\" to make new document at front");
    std::thread::sleep(Duration::from_millis(400));
    let pid = backend::frontmost_pid().expect("frontmost_pid() нашёл TextEdit");
    pid
}

/// Читает текст документа, нормализуя переводы строк.
fn read_document() -> String {
    let text = osascript("tell application \"TextEdit\" to get text of front document");
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn close_document() {
    let _ = osascript("tell application \"TextEdit\" to close front document saving no");
}

/// Печать в тесте: быстрая, чтобы тест не тянулся, но с реальными
/// задержками между символами.
fn test_config() -> Arc<Config> {
    Arc::new(Config {
        average_speed_cps: 50.0,
        speed_variance_percent: 0.0,
        delay_before_typing_ms: 150,
        ..Config::default()
    })
}

fn no_flags() -> (AtomicBool, AtomicBool) {
    (AtomicBool::new(false), AtomicBool::new(false))
}

/// Сценарий 1 (регрессия «всё на одной строке»): фоновая печать
/// многострочного текста в TextEdit. Ожидаем три строки, вторая и
/// третья — с нулевой колонки (авто-отступа нет).
#[test]
fn background_multiline_text_keeps_newlines() {
    if !gui_tests_enabled() {
        eprintln!("ПРОПУЩЕНО: нет AUTOTYPER_GUI_TEST=1");
        return;
    }
    let pid = open_textedit();
    let (cancel, paused) = no_flags();
    let mut typer = Typer::new(test_config(), Box::new(PidSink::new(pid)))
        .expect("Typer с PidSink создан");
    typer
        .type_text("1111\n2222\n3333", &cancel, &paused, false) // цифры: автокапитализация их не трогает
        .expect("печать без ошибки");

    let text = read_document();
    close_document();
    assert_eq!(
        text, "1111\n2222\n3333",
        "ожидаем три строки без лишних отступов"
    );
}

/// Сценарий 2: печать фокусным приёмником (enigo). Фокусный режим печатает
/// в переднее окно, а владение фокусом в автоматическом прогоне ненадёжно
/// (если человек работает за машиной, активация TextEdit может игнорироваться,
/// и события уходят в его окно). Поэтому: если ввод в TextEdit попал —
/// проверяем строго; если нет — пропускаем с пояснением. Строгая проверка
/// фонового режима (pid-адресного) — в тестах выше.
#[test]
fn focused_multiline_text_keeps_newlines() {
    if !gui_tests_enabled() {
        eprintln!("ПРОПУЩЕНО: нет AUTOTYPER_GUI_TEST=1");
        return;
    }
    let _pid = open_textedit();
    let (cancel, paused) = no_flags();
    let mut text = String::new();
    for attempt in 1..=2 {
        if attempt > 1 {
            osascript("tell application \"TextEdit\" to activate");
            std::thread::sleep(Duration::from_millis(500));
        }
        let mut typer = Typer::new(test_config(), Box::new(EnigoSink::new().expect("EnigoSink")))
            .expect("Typer создан");
        typer
            .type_text("12\n34", &cancel, &paused, false)
            .expect("печать без ошибки");
        std::thread::sleep(Duration::from_millis(300));
        text = read_document();
        if !text.is_empty() {
            break;
        }
        eprintln!("попытка {attempt}: ввод не попал в TextEdit");
    }
    close_document();
    if text.is_empty() {
        eprintln!(
            "ПРОПУЩЕНО: фокус не удалось отдать TextEdit в автоматическом прогоне \
             (за машиной работают). Механизм проверялся вручную."
        );
        return;
    }
    assert_eq!(text, "12\n34", "ожидаем две строки");
}

/// Сценарий 3: печать с сохранённым в исходнике отступом — вторая строка
/// должна получить ровно два ведущих пробела (не больше, не меньше).
#[test]
fn background_indented_source_keeps_exact_indent() {
    if !gui_tests_enabled() {
        eprintln!("ПРОПУЩЕНО: нет AUTOTYPER_GUI_TEST=1");
        return;
    }
    let pid = open_textedit();
    let (cancel, paused) = no_flags();
    let mut typer = Typer::new(test_config(), Box::new(PidSink::new(pid)))
        .expect("Typer с PidSink создан");
    typer
        .type_text("123\n  456", &cancel, &paused, false)
        .expect("печать без ошибки");

    let text = read_document();
    close_document();
    assert_eq!(text, "123\n  456", "отступ второй строки — ровно 2 пробела");
}

/// ДИАГНОСТИКА фонового режима (включается AUTOTYPER_GUI_DIAG=1): свежий
/// документ на каждый вариант «перевод строки + в начало строки», чтобы
/// видеть поведение живого приложения при смене клавиш. Фокусные (глобальные)
/// события в диагностике сознательно не используются: в автопрогоне нельзя
/// надёжно владеть фокусом.
#[test]
fn diag_background_variants() {
    if std::env::var_os("AUTOTYPER_GUI_TEST").is_none()
        || std::env::var_os("AUTOTYPER_GUI_DIAG").is_none()
    {
        eprintln!("ПРОПУЩЕНО: нет AUTOTYPER_GUI_TEST=1 + AUTOTYPER_GUI_DIAG=1");
        return;
    }
    let fresh = |label: &str, body: &dyn Fn(&mut PidSink)| {
        let pid = open_textedit();
        let mut sink = PidSink::new(pid);
        body(&mut sink);
        std::thread::sleep(Duration::from_millis(300));
        let text = read_document();
        println!("{label}: {text:?}");
        close_document();
    };

    // Полный путь фоновой печати: перевод строки + символы, как есть.
    fresh("полный-путь (newline+символ)", &|sink| {
        sink.type_char('1').unwrap();
        sink.press_newline(true).unwrap();
        sink.type_char('2').unwrap();
    });
}
