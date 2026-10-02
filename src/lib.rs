//! Библиотечная часть autotyper: вынесена из main, чтобы интеграционные
//! тесты (tests/) имели доступ к KeySink, конфигу и платформенным
//! приёмникам без запуска самого приложения.

pub mod backend;
pub mod config;
pub mod hotkey;
pub mod log;
pub mod typer;
