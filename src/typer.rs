//! Посимвольный «человеческий» ввод текста.
//!
//! Ключевая идея: мгновенная скорость печати — не константа, а случайная
//! величина с нормальным распределением вокруг `average_speed_cps`
//! (`speed_variance_percent` — сигма в процентах от средней). Поверх
//! базовой задержки добавляются «человеческие» паузы по категории символа.
//!
//! Ввод отправляется в `dyn KeySink`: либо в фокусное окно через enigo
//! (все ОС), либо — на macOS — конкретному процессу через CGEventPostToPid
//! (фоновый режим, окно не обязано быть в фокусе).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use rand::rngs::ThreadRng;
use rand::Rng;
use rand_distr::Normal;

use crate::config::{Config, NewlineMode};

/// Минимальная задержка между символами, мс: обрезка «слишком быстрых»
/// выборок распределения (защита от нулевых/отрицательных задержек и от
/// потери событий CoreGraphics на macOS).
const MIN_DELAY_MS: f64 = 8.0;
/// Максимальная задержка между символами, мс: обрезка «слишком медленных»
/// выборок («хвост» нормального распределения).
const MAX_DELAY_MS: f64 = 1500.0;
/// Мгновенная скорость не ниже 10% средней — иначе деление даёт гигантские
/// задержки из далёкого левого хвоста распределения.
const MIN_SPEED_FRACTION: f64 = 0.1;

/// Куда отправляются «нажатия»: в фокусное окно (enigo) или в конкретный
/// процесс (macOS-фон). Абстракция позволяет typer.rs оставаться
/// платформенно-нейтральным.
pub trait KeySink {
    /// Ввод одного символа (Unicode, включая кириллицу).
    fn type_char(&mut self, c: char) -> Result<()>;
    /// Перевод строки: Shift+Enter либо чистый Enter.
    fn press_newline(&mut self, shift: bool) -> Result<()>;
    /// Чистый Enter (финальная отправка в режиме type_and_send).
    fn press_enter(&mut self) -> Result<()>;
}

/// Фокусный приёмник: классическая печать через enigo в активное окно.
pub struct EnigoSink {
    enigo: Enigo,
}

impl EnigoSink {
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| anyhow!("не удалось инициализировать движок ввода enigo: {e}"))?;
        Ok(Self { enigo })
    }
}

impl KeySink for EnigoSink {
    fn type_char(&mut self, c: char) -> Result<()> {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        self.enigo
            .text(s)
            .with_context(|| format!("не удалось ввести символ {c:?} (движок вернул ошибку)"))
    }

    fn press_newline(&mut self, shift: bool) -> Result<()> {
        if shift {
            self.enigo
                .key(Key::Shift, Direction::Press)
                .context("не удалось нажать Shift")?;
            self.enigo
                .key(Key::Return, Direction::Click)
                .context("не удалось нажать Enter")?;
            self.enigo
                .key(Key::Shift, Direction::Release)
                .context("не удалось отпустить Shift")?;
        } else {
            self.enigo
                .key(Key::Return, Direction::Click)
                .context("не удалось нажать Enter")?;
        }
        Ok(())
    }

    fn press_enter(&mut self) -> Result<()> {
        self.press_newline(false)
    }

}

/// Категория символа — определяет дополнительную «человеческую» паузу.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharKind {
    Newline,
    Space,
    Comma,
    /// `. ! ? ; :`
    Punctuation,
    Regular,
}

/// Классификация символа по категории паузы.
pub fn classify_char(c: char) -> CharKind {
    match c {
        '\n' | '\r' => CharKind::Newline,
        ' ' | '\t' => CharKind::Space,
        ',' => CharKind::Comma,
        '.' | '!' | '?' | ';' | ':' => CharKind::Punctuation,
        _ => CharKind::Regular,
    }
}

/// Дополнительная пауза (мс) после символа данной категории согласно конфигу.
pub fn extra_pause_ms(kind: CharKind, cfg: &Config) -> u64 {
    match kind {
        CharKind::Newline => cfg.pause_after_newline_ms,
        CharKind::Space => cfg.pause_after_space_ms,
        CharKind::Comma => cfg.pause_after_comma_ms,
        CharKind::Punctuation => cfg.pause_after_punctuation_ms,
        CharKind::Regular => 0,
    }
}

/// Требует ли данный режим перевода строки удержание Shift.
/// Выделено в чистую функцию для юнит-тестов.
pub fn newline_uses_shift(mode: NewlineMode) -> bool {
    matches!(mode, NewlineMode::ShiftEnter)
}

/// Модель мгновенной скорости печати: Normal(mean = average_speed_cps,
/// sigma = mean * speed_variance_percent / 100).
pub struct SpeedModel {
    normal: Normal<f64>,
    min_cps: f64,
}

impl SpeedModel {
    pub fn new(cfg: &Config) -> Result<Self> {
        let mean = cfg.average_speed_cps.max(0.001);
        let sigma = (mean * cfg.speed_variance_percent / 100.0).max(1e-6);
        let normal = Normal::new(mean, sigma)
            .map_err(|e| anyhow!("не удалось построить нормальное распределение: {e}"))?;
        Ok(Self {
            normal,
            min_cps: mean * MIN_SPEED_FRACTION,
        })
    }

    /// Случайная задержка до следующего символа, мс.
    /// Скорость обрезается снизу, поэтому задержка всегда конечна и > 0,
    /// дополнительно зажимается в [MIN_DELAY_MS, MAX_DELAY_MS].
    pub fn sample_delay_ms(&mut self, rng: &mut ThreadRng) -> f64 {
        let cps = rng.sample(self.normal).max(self.min_cps);
        (1000.0 / cps).clamp(MIN_DELAY_MS, MAX_DELAY_MS)
    }
}

/// Итоговая статистика одного сеанса печати (для лога).
#[derive(Debug, Clone)]
pub struct TypeStats {
    /// Всего символов в исходном тексте (без `\r`).
    pub total_chars: usize,
    /// Сколько символов реально введено до завершения/отмены.
    pub typed_chars: usize,
    /// Чистое время печати (без задержки перед началом и без пауз).
    pub elapsed: Duration,
    /// Суммарное время, проведённое на паузе.
    pub paused: Duration,
    /// true, если печать прервана panic-хоткеем (полный сброс).
    pub cancelled: bool,
}

/// «Печатальщик»: владеет приёмником ввода и печатает текст посимвольно.
pub struct Typer {
    cfg: Arc<Config>,
    sink: Box<dyn KeySink>,
    speed: SpeedModel,
    rng: ThreadRng,
}

impl Typer {
    /// Создаёт экземпляр поверх выбранного приёмника (enigo или фоновый PID).
    pub fn new(cfg: Arc<Config>, sink: Box<dyn KeySink>) -> Result<Self> {
        Ok(Self {
            speed: SpeedModel::new(&cfg)?,
            sink,
            cfg,
            rng: rand::thread_rng(),
        })
    }

    /// Печатает `text` с «человеческими» задержками.
    ///
    /// * Перед началом выдерживает `delay_before_typing_ms` (с проверкой
    ///   отмены и с учётом паузы, если она уже стоит).
    /// * Посимвольно вводит текст; оба флага — `cancel` (полный сброс) и
    ///   `paused` (пауза) — проверяются на каждой итерации и внутри каждой
    ///   задержки, поэтому реакция мгновенная.
    /// * Время на паузе исключается из статистики.
    /// * Если `send_enter_at_end`, в самом конце нажимается чистый Enter.
    pub fn type_text(
        &mut self,
        text: &str,
        cancel: &AtomicBool,
        paused: &AtomicBool,
        send_enter_at_end: bool,
    ) -> Result<TypeStats> {
        run_typing(
            &mut *self.sink,
            &self.cfg,
            text,
            cancel,
            paused,
            &mut self.speed,
            &mut self.rng,
            send_enter_at_end,
        )
    }
}

/// Собственно посимвольная печать (выделена из Typer для юнит-тестов
/// с мок-приёмником: тесты проверяют точную последовательность «нажатий»).
#[allow(clippy::too_many_arguments)]
fn run_typing(
    sink: &mut dyn KeySink,
    cfg: &Config,
    text: &str,
    cancel: &AtomicBool,
    paused: &AtomicBool,
    speed: &mut SpeedModel,
    rng: &mut ThreadRng,
    send_enter_at_end: bool,
) -> Result<TypeStats> {
    let total_chars = text.chars().filter(|c| *c != '\r').count();

    // Пауза, чтобы пользователь успел переключиться в целевое окно.
    let (cancelled_early, _) =
        wait_with_pause(Duration::from_millis(cfg.delay_before_typing_ms), cancel, paused);
    if cancelled_early {
        return Ok(TypeStats {
            total_chars,
            typed_chars: 0,
            elapsed: Duration::ZERO,
            paused: Duration::ZERO,
            cancelled: true,
        });
    }

    let start = Instant::now();
    let mut typed = 0usize;
    let mut cancelled = false;
    let mut paused_total = Duration::ZERO;

    for c in text.chars() {
        // Нормализация CRLF -> LF: `\r` пропускаем.
        if c == '\r' {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }

        if c == '\n' {
            sink.press_newline(newline_uses_shift(cfg.newline_mode))
                .context("не удалось ввести перевод строки")?;
        } else {
            sink.type_char(c)
                .with_context(|| format!("не удалось ввести символ {c:?}"))?;
        }
        typed += 1;

        // Задержка на символ = базовая (из нормального распределения)
        // + пауза по категории символа.
        let base_ms = speed.sample_delay_ms(rng);
        let extra_ms = extra_pause_ms(classify_char(c), cfg) as f64;
        let wait = Duration::from_secs_f64((base_ms + extra_ms) / 1000.0);
        let (cancelled_now, paused_dur) = wait_with_pause(wait, cancel, paused);
        paused_total += paused_dur;
        if cancelled_now {
            cancelled = true;
            break;
        }
    }

    // Режим type_and_send: финальный чистый Enter (newline_mode тут не
    // применяется — задача именно отправить сообщение).
    if send_enter_at_end && !cancelled {
        sink.press_enter()
            .context("не удалось нажать Enter после печати")?;
    }

    Ok(TypeStats {
        total_chars,
        typed_chars: typed,
        elapsed: start.elapsed().saturating_sub(paused_total),
        paused: paused_total,
        cancelled,
    })
}

/// Ждёт суммарно `total`, просыпаясь каждые ~20 мс:
/// * проверяет флаг полного сброса `cancel` (в том числе во время паузы);
/// * пока стоит `paused`, счётчик ожидания не убывает — печать заморожена.
///
/// Возвращает (был ли сброс, сколько времени провели на паузе).
fn wait_with_pause(total: Duration, cancel: &AtomicBool, paused: &AtomicBool) -> (bool, Duration) {
    const CHUNK: Duration = Duration::from_millis(20);
    const PAUSE_POLL: Duration = Duration::from_millis(50);

    let mut remaining = total;
    let mut paused_total = Duration::ZERO;
    let mut pause_started: Option<Instant> = None;

    while remaining > Duration::ZERO {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if paused.load(Ordering::Relaxed) {
            if pause_started.is_none() {
                pause_started = Some(Instant::now());
            }
            sleep(PAUSE_POLL);
            continue;
        }
        if let Some(t) = pause_started.take() {
            paused_total += t.elapsed();
        }
        let step = remaining.min(CHUNK);
        sleep(step);
        remaining -= step;
    }

    if let Some(t) = pause_started {
        paused_total += t.elapsed();
    }
    (cancel.load(Ordering::Relaxed), paused_total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::default()
    }

    /// Мок-приёмник: записывает каждое «нажатие» строкой — так тесты видят
    /// точную последовательность событий печати.
    struct MockSink {
        log: Vec<String>,
    }

    impl MockSink {
        fn new() -> Self {
            Self { log: Vec::new() }
        }
    }

    impl KeySink for MockSink {
        fn type_char(&mut self, c: char) -> Result<()> {
            self.log.push(format!("ch:{c}"));
            Ok(())
        }
        fn press_newline(&mut self, shift: bool) -> Result<()> {
            self.log.push(format!("nl(shift={shift})"));
            Ok(())
        }
        fn press_enter(&mut self) -> Result<()> {
            self.log.push("enter".to_string());
            Ok(())
        }
    }

    /// Быстрый конфиг для мок-тестов: без начальной задержки.
    fn fast_config() -> Config {
        Config {
            delay_before_typing_ms: 0,
            ..Config::default()
        }
    }

    fn drive(cfg: &Config, text: &str, send_enter: bool) -> Vec<String> {
        let cancel = AtomicBool::new(false);
        let paused = AtomicBool::new(false);
        let mut sink = MockSink::new();
        let mut speed = SpeedModel::new(cfg).expect("модель");
        let mut rng = rand::thread_rng();
        run_typing(
            &mut sink,
            cfg,
            text,
            &cancel,
            &paused,
            &mut speed,
            &mut rng,
            send_enter,
        )
        .expect("печать без ошибки");
        sink.log
    }

    /// Многострочный текст печатается как есть: перевод строки и ВСЕ
    /// символы (включая ведущие пробелы отступов), без каких-либо
    /// дополнительных клавиш после перевода строки.
    #[test]
    fn multiline_types_everything_verbatim() {
        let log = drive(&fast_config(), "a\n  b", false);
        assert_eq!(
            log,
            vec!["ch:a", "nl(shift=true)", "ch: ", "ch: ", "ch:b"]
        );

        // Ведущие пробелы первой строки тоже печатаются.
        let log = drive(&fast_config(), "  a\nb\tc", false);
        assert_eq!(
            log,
            vec!["ch: ", "ch: ", "ch:a", "nl(shift=true)", "ch:b", "ch:\t", "ch:c"]
        );
    }

    /// newline_mode = "enter": перевод строки без Shift.
    #[test]
    fn newline_mode_enter_presses_plain_enter() {
        let cfg = Config {
            delay_before_typing_ms: 0,
            newline_mode: NewlineMode::Enter,
            ..Config::default()
        };
        let log = drive(&cfg, "a\nb", false);
        assert_eq!(log, vec!["ch:a", "nl(shift=false)", "ch:b"]);
    }

    /// type_and_send: в самом конце — чистый Enter.
    #[test]
    fn send_enter_at_end_presses_enter_once() {
        let log = drive(&fast_config(), "a", true);
        assert_eq!(log, vec!["ch:a", "enter"]);
    }

    #[test]
    fn classifies_chars_into_pause_categories() {
        assert_eq!(classify_char('\n'), CharKind::Newline);
        assert_eq!(classify_char('\r'), CharKind::Newline);
        assert_eq!(classify_char(' '), CharKind::Space);
        assert_eq!(classify_char('\t'), CharKind::Space);
        assert_eq!(classify_char(','), CharKind::Comma);
        for c in ['.', '!', '?', ';', ':'] {
            assert_eq!(classify_char(c), CharKind::Punctuation, "символ {c}");
        }
        // Обычные символы, включая кириллицу и цифры.
        for c in ['a', 'Z', '0', 'а', 'Я', 'ё', '-', '"'] {
            assert_eq!(classify_char(c), CharKind::Regular, "символ {c}");
        }
    }

    #[test]
    fn extra_pause_follows_config_values() {
        let mut cfg = test_config();
        cfg.pause_after_space_ms = 15;
        cfg.pause_after_punctuation_ms = 80;
        cfg.pause_after_newline_ms = 200;
        cfg.pause_after_comma_ms = 60;

        assert_eq!(extra_pause_ms(CharKind::Space, &cfg), 15);
        assert_eq!(extra_pause_ms(CharKind::Punctuation, &cfg), 80);
        assert_eq!(extra_pause_ms(CharKind::Newline, &cfg), 200);
        assert_eq!(extra_pause_ms(CharKind::Comma, &cfg), 60);
        assert_eq!(extra_pause_ms(CharKind::Regular, &cfg), 0);
    }

    /// Задержки из модели всегда положительны и зажаты в границы —
    /// отрицательных/нулевых/бесконечных задержек не бывает.
    #[test]
    fn sampled_delays_are_always_within_bounds() {
        let cfg = test_config();
        let mut model = SpeedModel::new(&cfg).expect("модель");
        let mut rng = rand::thread_rng();
        for _ in 0..2000 {
            let d = model.sample_delay_ms(&mut rng);
            assert!(d.is_finite(), "задержка конечна");
            assert!(d >= MIN_DELAY_MS && d <= MAX_DELAY_MS, "задержка {d} вне границ");
        }
    }

    /// Большая дисперсия не должна ломать границы (хвост распределения).
    #[test]
    fn huge_variance_stays_within_bounds() {
        let mut cfg = test_config();
        cfg.speed_variance_percent = 500.0;
        let mut model = SpeedModel::new(&cfg).expect("модель");
        let mut rng = rand::thread_rng();
        for _ in 0..2000 {
            let d = model.sample_delay_ms(&mut rng);
            assert!(d >= MIN_DELAY_MS && d <= MAX_DELAY_MS, "задержка {d} вне границ");
        }
    }

    /// Средняя мгновенная скорость должна быть близка к настроенной средней.
    /// Проверяем именно скорость (1000/задержка), а не среднюю задержку:
    /// по неравенству Йенсена E[1/X] > 1/E[X], поэтому средняя задержка
    /// закономерно выше 1000/average_speed_cps при большой дисперсии.
    #[test]
    fn mean_instant_speed_is_close_to_configured() {
        let cfg = test_config(); // 14 симв/с
        let mut model = SpeedModel::new(&cfg).expect("модель");
        let mut rng = rand::thread_rng();
        let n = 20_000;
        let sum: f64 = (0..n)
            .map(|_| 1000.0 / model.sample_delay_ms(&mut rng))
            .sum();
        let mean_cps = sum / n as f64;
        assert!(
            (mean_cps - cfg.average_speed_cps).abs() / cfg.average_speed_cps < 0.10,
            "средняя мгновенная скорость {mean_cps:.2} симв/с далека от {} симв/с",
            cfg.average_speed_cps
        );
    }

    /// Выбор режима перевода строки.
    #[test]
    fn newline_mode_selection() {
        assert!(newline_uses_shift(NewlineMode::ShiftEnter));
        assert!(!newline_uses_shift(NewlineMode::Enter));

        // Из JSON-строки режим парсится корректно.
        let shift: NewlineMode = serde_json::from_str("\"shift_enter\"").expect("parse");
        let plain: NewlineMode = serde_json::from_str("\"enter\"").expect("parse");
        assert_eq!(shift, NewlineMode::ShiftEnter);
        assert_eq!(plain, NewlineMode::Enter);
        assert!(newline_uses_shift(shift));
        assert!(!newline_uses_shift(plain));
    }

    /// Уже установленный сброс должен прерывать ожидание немедленно,
    /// не выжидая полную длительность.
    #[test]
    fn wait_returns_immediately_when_cancelled() {
        let cancel = AtomicBool::new(true);
        let paused = AtomicBool::new(false);
        let started = Instant::now();
        let (cancelled, paused_total) =
            wait_with_pause(Duration::from_secs(10), &cancel, &paused);
        assert!(cancelled);
        assert_eq!(paused_total, Duration::ZERO);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "ожидание прервалось немедленно"
        );
    }

    /// Пауза, снятая до истечения короткого ожидания, не ломает результат:
    /// ожидание завершается по таймауту, а не по сбросу.
    #[test]
    fn wait_completes_when_not_paused_or_cancelled() {
        let cancel = AtomicBool::new(false);
        let paused = AtomicBool::new(false);
        let (cancelled, paused_total) =
            wait_with_pause(Duration::from_millis(60), &cancel, &paused);
        assert!(!cancelled);
        assert_eq!(paused_total, Duration::ZERO);
    }
}
