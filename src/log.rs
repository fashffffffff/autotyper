//! Простейший логгер в файл (append, без ротации).
//! Пишет строку «<таймстемп chrono> <сообщение>»; межпоточная
//! синхронизация — Mutex<File>.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::Local;

pub struct Logger {
    file: Mutex<File>,
}

impl Logger {
    /// Открывает (или создаёт) лог-файл в режиме append.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("не удалось создать каталог {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("не удалось открыть лог-файл {}", path.display()))?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    /// Дописывает строку в лог. Ошибки записи сознательно глотаем:
    /// лог не должен ломать печать.
    pub fn log(&self, message: &str) {
        let line = format!(
            "{} {}\n",
            Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            message
        );
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logger_appends_lines() {
        let dir = std::env::temp_dir().join("autotyper-test-logger");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("test.log");
        {
            let logger = Logger::open(&path).expect("open");
            logger.log("первая строка");
            logger.log("вторая строка");
        } // дропаем Mutex<File>, чтобы flush прошёл при закрытии
        let content = std::fs::read_to_string(&path).expect("read");
        assert!(content.contains("первая строка"), "{content}");
        assert!(content.contains("вторая строка"), "{content}");
        // Каждая запись содержит таймстемп вида 2026-01-02 03:04:05.123
        let mut lines = content.lines();
        let first = lines.next().expect("первая строка есть");
        assert!(
            first.starts_with("20"),
            "строка начинается с года: {first}"
        );
        assert!(first.contains(' '), "есть разделитель таймстемпа");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
