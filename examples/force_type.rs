//! Отладочная утилита: печатает строку в процесс по PID через фоновый
//! приёмник (macOS). Используется для ручной/Playwright-валидации
//! поведения в реальных приложениях (textarea в браузере и т.п.).
//!
//! Запуск: cargo run --release --example force_type -- <pid> <текст>
//! В тексте '\n' заменяется на перевод строки (Shift+Enter).

#[cfg(target_os = "macos")]
fn main() {
    use autotyper::backend::macos::PidSink;
    use autotyper::typer::KeySink;

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("использование: force_type <pid> <текст>");
        std::process::exit(2);
    }
    let pid: i32 = args[1].parse().expect("pid — число");
    let text = args[2].replace("\\n", "\n");

    let mut sink = PidSink::new(pid);
    for c in text.chars() {
        if c == '\n' {
            sink.press_newline(true).expect("newline");
        } else {
            sink.type_char(c).expect("символ");
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("пример работает только на macOS");
}
