//! autotyper — по глобальному хоткею читает текст из буфера обмена
//! и «печатает» его в активное окно другого приложения, имитируя
//! живой ручной ввод (случайная скорость, паузы после пробелов,
//! знаков препинания и переводов строки).
//!
//! Управление во время печати:
//! * пауза/продолжение — pause_hotkey (по умолчанию F8);
//! * полный сброс — panic_hotkey (по умолчанию Esc).
//!
//! Вся логика живёт в библиотечных модулях (см. lib.rs); main только
//! связывает их и запускает блокирующий перехват клавиатуры.

use std::sync::Arc;

use anyhow::{Context, Result};

use autotyper::{backend, config, hotkey, log::Logger};

fn main() {
    if let Err(err) = run() {
        eprintln!("[autotyper] фатальная ошибка: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // Конфиг: CONFIG.json в «своей» директории (см. config::app_dir);
    // при отсутствии — создаётся дефолтный с комментариями.
    let mut cfg = config::load_or_create().context("не удалось загрузить конфигурацию")?;
    // Фоновая печать реализована только на macOS; на других ОС файл
    // конфига общий, поэтому просто игнорируем флаг с предупреждением.
    if cfg.background_mode && !cfg!(target_os = "macos") {
        eprintln!(
            "[autotyper] background_mode=true поддерживается только на macOS — игнорируется"
        );
        cfg.background_mode = false;
    }
    let cfg = Arc::new(cfg);

    // Лог: autotyper.log в той же директории, что и CONFIG.json;
    // если директория недоступна на запись — запасной ~/.config/autotyper.
    let log_path = config::log_file_path().context("не удалось определить путь лог-файла")?;
    let (logger, effective_log_path) = match Logger::open(&log_path) {
        Ok(logger) => (Arc::new(logger), log_path),
        Err(primary_err) => {
            let fallback = config::fallback_app_dir()
                .context("не удалось определить резервный каталог лога")?
                .join("autotyper.log");
            eprintln!(
                "[autotyper] {} недоступен на запись ({primary_err:#}), лог: {}",
                log_path.display(),
                fallback.display()
            );
            let logger = Logger::open(&fallback).with_context(|| {
                format!("не удалось открыть лог-файл {}", fallback.display())
            })?;
            (Arc::new(logger), fallback)
        }
    };
    logger.log(&format!(
        "=== autotyper {} запущен (pid {}, конфиг: {}) ===",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        config::config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".to_string()),
    ));

    // Короткие инфо-сообщения для запуска из терминала.
    eprintln!(
        "[autotyper] запущен. Печать: {} | пауза/продолжение: {} | полный сброс: {}",
        cfg.active_hotkey().display(),
        cfg.pause_hotkey.display(),
        cfg.panic_hotkey.display(),
    );
    if let Some(send_hk) = cfg.type_and_send_hotkey.as_ref() {
        eprintln!("[autotyper] печать + Enter в конце: {}", send_hk.display());
    }
    // Явно показываем, ГДЕ лежат настройки — чтобы их редактировали
    // в файле, а не в коде.
    if let Ok(cfg_path) = config::config_path() {
        eprintln!(
            "[autotyper] настройки (редактируйте этот файл, комментарии — прямо в нём): {}",
            cfg_path.display()
        );
    }
    eprintln!("[autotyper] лог: {}", effective_log_path.display());
    if cfg.background_mode {
        eprintln!(
            "[autotyper] ФОНОВЫЙ режим ВКЛЮЧЁН: печать идёт в окно, которое было \
             в фокусе в момент хоткея; фокус удерживать не нужно"
        );
    }
    for warning in backend::environment_warnings() {
        eprintln!("[autotyper] внимание: {warning}");
    }

    // Блокирующий вызов: слушаем глобальные события клавиатуры, пока жив
    // процесс (завершение — Ctrl+C в терминале / stop юнита).
    hotkey::run(cfg, logger)
}
