//! Конфигурация autotyper: структуры, дефолты, загрузка/создание файла,
//! валидация.
//!
//! Файл ищется по порядку:
//! 1. `CONFIG.json` в «своей» директории (см. `app_dir`): корень проекта
//!    при разработке либо каталог бинарника;
//! 2. если каталог не доступен на запись — запасной
//!    `$HOME/.config/autotyper/CONFIG.json`;
//! 3. если файла нет — создаётся дефолтный (с `//`-комментариями).
//!
//! Поддержка комментариев: полные и хвостовые `//`-комментарии вырезаются
//! перед разбором JSON (см. `strip_json_comments`).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Значение по умолчанию для опционального поля `pause_after_comma_ms`
/// (в JSON из спецификации его может не быть).
fn default_pause_after_comma_ms() -> u64 {
    60
}

/// Дефолт хоткея паузы: одиночная клавиша F8, БЕЗ модификаторов.
/// Ctrl+клавиша не годится: редакторы перехватывают такие сочетания сами
/// (Ctrl+A — в начало строки, Ctrl+P — строкой вверх и т.п.).
/// F8 нейтральна почти везде. Выделено в функцию, чтобы и serde выдавал
/// её старым конфигам без поля `pause_hotkey`.
fn default_pause_hotkey() -> HotkeyConfig {
    HotkeyConfig {
        modifiers: Vec::new(),
        key: "f8".to_string(),
    }
}

/// Дефолт фоновой печати: включена (рабочий режим автора, macOS).
fn default_background_mode() -> bool {
    true
}

/// Режим ввода символа перевода строки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NewlineMode {
    /// `\n` вводится как Shift+Enter — в чатах случайно не отправит сообщение.
    ShiftEnter,
    /// `\n` вводится как обычный Enter.
    Enter,
}

/// Один хоткей: модификаторы (ctrl/alt/shift/meta) + основная клавиша.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    /// Список модификаторов, например ["ctrl"] или ["ctrl", "shift"].
    #[serde(default)]
    pub modifiers: Vec<String>,
    /// Основная клавиша: "v", "escape", "enter", "p", "f8", ...
    pub key: String,
}

/// Платформенные пресеты основного хоткея.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeysConfig {
    /// macOS: по умолчанию Ctrl+V (именно Ctrl, а не Cmd — чтобы не
    /// конфликтовать с системной вставкой Cmd+V).
    pub macos: HotkeyConfig,
    /// Windows/Linux: по умолчанию Alt+V.
    pub other: HotkeyConfig,
}

/// Главный конфиг приложения (отражает CONFIG.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub hotkey: HotkeysConfig,
    /// Хоткей полного СБРОСА печати (по умолчанию Esc): печать прекращается
    /// и больше не возобновляется — в отличие от паузы.
    pub panic_hotkey: HotkeyConfig,
    /// Хоткей ПАУЗЫ/продолжения (переключатель, по умолчанию F8).
    /// При возобновлении проверяется, что фокус вернулся в целевое окно.
    /// Поле опциональное: старым конфигам без него подставляется дефолт.
    #[serde(default = "default_pause_hotkey")]
    pub pause_hotkey: HotkeyConfig,
    /// Опциональный хоткей «напечатать и нажать Enter в конце»
    /// (по умолчанию выключен — отсутствует в JSON).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_and_send_hotkey: Option<HotkeyConfig>,
    /// Средняя скорость печати, символов в секунду.
    ///
    /// Дефолт 42 (из рабочего конфига автора): очень быстрая, почти
    /// мгновенная печать. «Живая» машинистка — примерно 14
    /// (40 WPM ≈ 3.3 симв/с × 4).
    pub average_speed_cps: f64,
    /// Стандартное отклонение мгновенной скорости, % от средней (нормальное
    /// распределение).
    pub speed_variance_percent: f64,
    /// Пауза перед началом печати, мс — чтобы пользователь успел переключиться
    /// в целевое окно.
    pub delay_before_typing_ms: u64,
    pub newline_mode: NewlineMode,
    pub pause_after_space_ms: u64,
    /// Пауза после `. ! ? ; :`
    pub pause_after_punctuation_ms: u64,
    pub pause_after_newline_ms: u64,
    /// Пауза после запятой (опциональное поле, см. default выше).
    #[serde(default = "default_pause_after_comma_ms")]
    pub pause_after_comma_ms: u64,
    /// Фоновая печать БЕЗ удержания фокуса. Только macOS (CGEventPostToPid):
    /// печать идёт в окно, которое было в фокусе в момент хоткея, и вы можете
    /// переключаться в другие окна. На Windows/Linux молча игнорируется.
    #[serde(default = "default_background_mode")]
    pub background_mode: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: HotkeysConfig {
                macos: HotkeyConfig {
                    modifiers: vec!["ctrl".to_string()],
                    key: "v".to_string(),
                },
                other: HotkeyConfig {
                    modifiers: vec!["alt".to_string()],
                    key: "v".to_string(),
                },
            },
            panic_hotkey: HotkeyConfig {
                modifiers: Vec::new(),
                key: "escape".to_string(),
            },
            pause_hotkey: default_pause_hotkey(),
            type_and_send_hotkey: None,
            // Значения ниже — из «рабочего» конфига автора: очень быстрая
            // печать с небольшим разбросом (42 симв/с, σ=10%) и короткими
            // микропаузами.
            average_speed_cps: 42.0,
            speed_variance_percent: 10.0,
            delay_before_typing_ms: 400,
            newline_mode: NewlineMode::ShiftEnter,
            pause_after_space_ms: 10,
            pause_after_punctuation_ms: 80,
            pause_after_newline_ms: 20,
            pause_after_comma_ms: default_pause_after_comma_ms(),
            background_mode: default_background_mode(),
        }
    }
}

impl Config {
    /// Хоткей, активный на текущей платформе.
    pub fn active_hotkey(&self) -> &HotkeyConfig {
        if cfg!(target_os = "macos") {
            &self.hotkey.macos
        } else {
            &self.hotkey.other
        }
    }

    /// Семантическая валидация значений, которые serde проверить не может.
    pub fn validate(&self) -> Result<()> {
        if self.average_speed_cps <= 0.0 {
            bail!("average_speed_cps должен быть больше нуля");
        }
        if self.speed_variance_percent < 0.0 || self.speed_variance_percent > 500.0 {
            bail!("speed_variance_percent должен быть в диапазоне 0..500");
        }
        // Внимание: background_mode = true на Windows/Linux НЕ ошибка конфига —
        // он просто игнорируется с предупреждением в main (файл один на все ОС,
        // и дефолт включает фон для macOS).
        let check = |name: &str, hk: &HotkeyConfig| -> Result<()> {
            if hk.key.trim().is_empty() {
                bail!("hotkey {name}: ключ не может быть пустым");
            }
            for m in &hk.modifiers {
                if m.trim().is_empty() {
                    bail!("hotkey {name}: пустое имя модификатора");
                }
            }
            Ok(())
        };
        check("macos", &self.hotkey.macos)?;
        check("other", &self.hotkey.other)?;
        check("panic_hotkey", &self.panic_hotkey)?;
        check("pause_hotkey", &self.pause_hotkey)?;
        if let Some(hk) = &self.type_and_send_hotkey {
            check("type_and_send_hotkey", hk)?;
        }
        Ok(())
    }
}

impl HotkeyConfig {
    /// Человекочитаемое представление: "Ctrl+V", "Escape", "F8", ...
    pub fn display(&self) -> String {
        let mut out = String::new();
        for m in &self.modifiers {
            out.push_str(&capitalize(m));
            out.push('+');
        }
        out.push_str(&capitalize(&self.key));
        out
    }
}

fn capitalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => lower,
    }
}

/// Убирает из JSON однострочные комментарии `//` (полные и хвостовые).
///
/// Сканирование строично-внимательное: отслеживаем, находимся ли мы внутри
/// JSON-строки, поэтому `//` внутри значений (например, "https://...")
/// не трогаются. Блоковые `/* */` не поддерживаются — намеренно.
pub fn strip_json_comments(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            continue;
        }
        // Начало //-комментария (вне строки): выкидываем всё до конца строки.
        if c == '/' && chars.peek() == Some(&'/') {
            for rest in chars.by_ref() {
                if rest == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Дефолтный CONFIG.json, записываемый при первом запуске.
/// Комментированная версия — чтобы «люди не путались»: каждый параметр
/// подписан. Держится в синхроне с `Config::default()` тестом
/// `commented_default_config_matches_defaults`.
const DEFAULT_CONFIG_JSON: &str = r#"// ============================================================
// Конфигурация autotyper
// Этот файл лежит в одной директории со всем остальным (рядом с
// бинарником; при разработке — в корне проекта).
// Строки с // — комментарии, парсер их игнорирует.
// После правок перезапустите autotyper (Ctrl+C и запуск заново).
// ============================================================
{
  // Хоткей запуска печати текста из буфера обмена.
  "hotkey": {
    // macOS: Ctrl+V (именно Ctrl, не Cmd — Cmd+V занят системным paste)
    "macos": { "modifiers": ["ctrl"], "key": "v" },
    // Windows и Linux: Alt+V
    "other": { "modifiers": ["alt"], "key": "v" }
  },

  // Хоткей ПОЛНОГО СБРОСА печати: печать немедленно прекращается
  // и НЕ продолжается (если печатаете не туда — жмите это).
  "panic_hotkey": { "modifiers": [], "key": "escape" },

  // Хоткей ПАУЗЫ / продолжения (переключатель). При возобновлении
  // autotyper проверит, что фокус вернулся в целевое окно.
  // Одиночная клавиша без модификаторов: Ctrl+клавиши перехватывают
  // редакторы (Ctrl+A — в начало строки, Ctrl+P — строкой вверх).
  // F8 нейтральна почти везде; альтернативы: f9, "=" и т.п.
  "pause_hotkey": { "modifiers": [], "key": "f8" },

  // Опциональный хоткей «напечатать и нажать Enter» (для чатов).
  // Чтобы включить — раскомментируйте и подберите сочетание:
  // "type_and_send_hotkey": { "modifiers": ["ctrl", "shift"], "key": "v" },

  // Средняя скорость печати, символов в секунду.
  // 42 — очень быстро (почти мгновенно); «живая» машинистка ≈ 14.
  "average_speed_cps": 42.0,

  // Разброс мгновенной скорости, % от средней (нормальное распределение).
  "speed_variance_percent": 10.0,

  // Пауза перед началом печати, мс — успеть переключиться в целевое окно.
  "delay_before_typing_ms": 400,

  // Как вводить перевод строки \n:
  //   "shift_enter" — Shift+Enter (в чатах НЕ отправит сообщение)
  //   "enter"       — обычный Enter
  "newline_mode": "shift_enter",

  // Дополнительные «человеческие» паузы, мс:
  "pause_after_space_ms": 10,       // после пробела/таба
  "pause_after_punctuation_ms": 80, // после . ! ? ; :
  "pause_after_newline_ms": 20,     // после перевода строки
  "pause_after_comma_ms": 60,       // после запятой

  // ФОНОВАЯ печать без удержания фокуса — ТОЛЬКО macOS.
  // true: печать идёт в окно, которое было в фокусе в момент хоткея,
  // и вы можете спокойно переключаться в другие окна.
  // На Windows/Linux молча игнорируется.
  "background_mode": true
}
"#;

/// Каталог, где живут CONFIG.json и лог — «одна директория со всем
/// остальным», без засорения домашней папки:
///
/// * при разработке — корень проекта (ищем Cargo.toml, поднимаясь от
///   бинарника вверх: target/release → target → корень);
/// * иначе — каталог самого бинарника («портативный» режим: exe +
///   CONFIG.json + лаунчер в одной папке);
/// * если туда не записать (например, /usr/local/bin без прав) —
///   резервный `~/.config/autotyper` (см. `fallback_app_dir`).
pub fn app_dir() -> Result<PathBuf> {
    let start = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .context("не удалось определить каталог запуска")?;

    // Dev-режим: поднимаемся до каталога с Cargo.toml.
    let mut current = start.as_path();
    loop {
        if current.join("Cargo.toml").is_file() {
            return Ok(current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => break,
        }
    }
    // Портативный режим: всё рядом с бинарником.
    Ok(start)
}

/// Резервный каталог, если основной недоступен на запись.
pub fn fallback_app_dir() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .context("не удалось определить домашний каталог ($HOME)");
    Ok(home?.join(".config").join("autotyper"))
}

/// Путь к CONFIG.json (имя капсом — чтобы файл был заметен).
pub fn config_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("CONFIG.json"))
}

/// Путь к лог-файлу (в том же каталоге, что и CONFIG.json).
pub fn log_file_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("autotyper.log"))
}

/// Записывает комментированный дефолтный конфиг по указанному пути
/// (создавая родительские каталоги при необходимости).
fn write_default_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("не удалось создать каталог {}", parent.display()))?;
    }
    fs::write(path, DEFAULT_CONFIG_JSON).with_context(|| {
        format!("не удалось записать дефолтный конфиг в {}", path.display())
    })
}

/// Загружает конфиг из «своей» директории (см. `app_dir`);
/// при отсутствии файла создаёт там же дефолтный. Если каталог
/// недоступен на запись — запасной вариант в `~/.config/autotyper`.
pub fn load_or_create() -> Result<Config> {
    let path = config_path()?;
    if path.exists() {
        return parse_config_file(&path);
    }
    // Пробуем создать дефолтный в основной директории.
    match write_default_config(&path) {
        Ok(()) => {
            eprintln!(
                "[autotyper] конфиг не найден, создан дефолтный (с комментариями): {}",
                path.display()
            );
            return Ok(Config::default());
        }
        Err(primary_err) => {
            // Не записалось (например, бинарник в /usr/local/bin) — fallback.
            let fallback = fallback_app_dir()?.join("CONFIG.json");
            if fallback.exists() {
                eprintln!(
                    "[autotyper] {} недоступен на запись ({primary_err:#}), \
                     использую конфиг из {}",
                    path.display(),
                    fallback.display()
                );
                return parse_config_file(&fallback);
            }
            write_default_config(&fallback)?;
            eprintln!(
                "[autotyper] конфиг не найден, создан дефолтный (с комментариями): {} \
                 (основной путь {} недоступен на запись)",
                fallback.display(),
                path.display()
            );
            Ok(Config::default())
        }
    }
}

fn parse_config_file(path: &PathBuf) -> Result<Config> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("не удалось прочитать {}", path.display()))?;
    // Конфиг может содержать // комментарии — вырезаем до разбора JSON.
    let stripped = strip_json_comments(&raw);
    let cfg: Config = serde_json::from_str(&stripped)
        .with_context(|| format!("ошибка разбора JSON в {}", path.display()))?;
    cfg.validate().with_context(|| {
        format!("некорректные значения в {}", path.display())
    })?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Дефолтный конфиг должен без потерь проходить serialize -> deserialize.
    #[test]
    fn default_config_roundtrip() {
        let json = serde_json::to_string(&Config::default()).expect("serde json");
        let parsed: Config = serde_json::from_str(&json).expect("parse default");
        assert_eq!(parsed.average_speed_cps, 42.0);
        assert_eq!(parsed.speed_variance_percent, 10.0);
        assert_eq!(parsed.delay_before_typing_ms, 400);
        assert_eq!(parsed.newline_mode, NewlineMode::ShiftEnter);
        assert_eq!(parsed.pause_after_space_ms, 10);
        assert_eq!(parsed.pause_after_punctuation_ms, 80);
        assert_eq!(parsed.pause_after_newline_ms, 20);
        assert_eq!(parsed.pause_after_comma_ms, 60);
        assert_eq!(parsed.hotkey.macos.modifiers, vec!["ctrl".to_string()]);
        assert_eq!(parsed.hotkey.macos.key, "v");
        assert_eq!(parsed.hotkey.other.modifiers, vec!["alt".to_string()]);
        assert!(parsed.type_and_send_hotkey.is_none());
        assert!(parsed.background_mode, "фон по умолчанию включён (macOS)");
        assert_eq!(parsed.panic_hotkey.key, "escape");
        // Пауза — одиночная клавиша F8, без модификаторов.
        assert!(parsed.pause_hotkey.modifiers.is_empty());
        assert_eq!(parsed.pause_hotkey.key, "f8");
    }

    /// Пример конфига из спецификации (без pause_after_comma_ms и новых
    /// полей) должен парситься; отсутствующим полям присваиваются дефолты.
    #[test]
    fn parses_spec_example_json() {
        let raw = r#"{
            "hotkey": {
              "macos": {"modifiers": ["ctrl"], "key": "v"},
              "other": {"modifiers": ["alt"], "key": "v"}
            },
            "panic_hotkey": {"modifiers": [], "key": "escape"},
            "average_speed_cps": 14,
            "speed_variance_percent": 40,
            "delay_before_typing_ms": 400,
            "newline_mode": "shift_enter",
            "pause_after_space_ms": 15,
            "pause_after_punctuation_ms": 80,
            "pause_after_newline_ms": 200
        }"#;
        let cfg: Config = serde_json::from_str(raw).expect("parse spec example");
        cfg.validate().expect("validate");
        assert_eq!(cfg.pause_after_comma_ms, 60, "опциональное поле получает дефолт");
        assert!(cfg.pause_hotkey.modifiers.is_empty());
        assert_eq!(cfg.pause_hotkey.key, "f8", "дефолт паузы — одиночная F8");
        assert!(cfg.background_mode, "дефолт фона — включён (реально работает на macOS)");
    }

    /// Конфиг с // комментариями (полными и хвостовыми) парсится.
    #[test]
    fn parses_commented_json() {
        let raw = r#"// шапка конфига
        {
          // хоткей печати
          "hotkey": {
            "macos": {"modifiers": ["ctrl"], "key": "v"}, // маковский вариант
            "other": {"modifiers": ["alt"], "key": "v"}
          },
          "panic_hotkey": {"modifiers": [], "key": "escape"},
          "pause_hotkey": {"modifiers": [], "key": "f8"},
          "average_speed_cps": 20, // побыстрее
          "speed_variance_percent": 40,
          "delay_before_typing_ms": 400,
          "newline_mode": "enter",
          "pause_after_space_ms": 15,
          "pause_after_punctuation_ms": 80,
          "pause_after_newline_ms": 200
        }"#;
        let cfg: Config = serde_json::from_str(&strip_json_comments(raw)).expect("parse");
        assert_eq!(cfg.average_speed_cps, 20.0);
        assert_eq!(cfg.newline_mode, NewlineMode::Enter);
    }

    /// Вырезание комментариев не трогает // внутри строк (URL и т.п.).
    #[test]
    fn strip_comments_keeps_slashes_inside_strings() {
        let raw = r#"{
            "key": "https://example.com//double",
            "other": "a/b//c" // хвостовой комментарий
        }"#;
        let stripped = strip_json_comments(raw);
        let value: serde_json::Value = serde_json::from_str(&stripped).expect("valid json");
        assert_eq!(value["key"], "https://example.com//double");
        assert_eq!(value["other"], "a/b//c");
    }

    /// Комментированный дефолт, записываемый при первом запуске,
    /// обязан соответствовать `Config::default()`.
    #[test]
    fn commented_default_config_matches_defaults() {
        let stripped = strip_json_comments(DEFAULT_CONFIG_JSON);
        let from_file: serde_json::Value =
            serde_json::from_str(&stripped).expect("комментированный дефолт — валидный JSON");
        let default_value = serde_json::to_value(Config::default()).expect("serialize default");
        assert_eq!(from_file, default_value);
        // И он распознаётся в полноценный конфиг без ошибок.
        let cfg: Config = serde_json::from_str(&stripped).expect("parse commented default");
        cfg.validate().expect("validate");
    }

    /// Неизвестное значение newline_mode должно давать понятную ошибку.
    #[test]
    fn rejects_unknown_newline_mode() {
        let raw = r#"{
            "hotkey": {"macos": {"modifiers": ["ctrl"], "key": "v"},
                       "other": {"modifiers": ["alt"], "key": "v"}},
            "panic_hotkey": {"modifiers": [], "key": "escape"},
            "average_speed_cps": 14,
            "speed_variance_percent": 40,
            "delay_before_typing_ms": 400,
            "newline_mode": "teleport",
            "pause_after_space_ms": 15,
            "pause_after_punctuation_ms": 80,
            "pause_after_newline_ms": 200
        }"#;
        assert!(serde_json::from_str::<Config>(raw).is_err());
    }

    #[test]
    fn validates_speed_and_empty_key() {
        let mut cfg = Config::default();
        cfg.average_speed_cps = 0.0;
        assert!(cfg.validate().is_err(), "нулевая скорость недопустима");

        let mut cfg = Config::default();
        cfg.panic_hotkey.key = "   ".to_string();
        assert!(cfg.validate().is_err(), "пустой ключ недопустим");

        let mut cfg = Config::default();
        cfg.pause_hotkey.key = String::new();
        assert!(cfg.validate().is_err(), "пустой ключ паузы недопустим");
    }

    /// background_mode=true НЕ ошибка конфига (файл общий для всех ОС,
    /// дефолт включает фон) — на не-macOS он просто игнорируется в main.
    #[test]
    fn background_mode_is_not_a_validation_error() {
        let mut cfg = Config::default();
        cfg.background_mode = true;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn active_hotkey_depends_on_platform() {
        let cfg = Config::default();
        if cfg!(target_os = "macos") {
            assert_eq!(cfg.active_hotkey().modifiers, vec!["ctrl".to_string()]);
        } else {
            assert_eq!(cfg.active_hotkey().modifiers, vec!["alt".to_string()]);
        }
        assert_eq!(cfg.hotkey.macos.key, "v");
        assert_eq!(cfg.hotkey.other.key, "v");
    }

    #[test]
    fn hotkey_display_is_human_readable() {
        let hk = HotkeyConfig {
            modifiers: vec!["ctrl".into(), "shift".into()],
            key: "v".into(),
        };
        assert_eq!(hk.display(), "Ctrl+Shift+V");
        let panic = HotkeyConfig {
            modifiers: vec![],
            key: "escape".into(),
        };
        assert_eq!(panic.display(), "Escape");
        let pause = HotkeyConfig {
            modifiers: vec![],
            key: "f8".into(),
        };
        assert_eq!(pause.display(), "F8");
    }
}
