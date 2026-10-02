#!/bin/bash
# ============================================================
# autotyper — запуск двойным кликом (macOS)
# Двойной клик по этому файлу открывает Terminal и запускает утилиту.
# Останов: Ctrl+C в открывшемся окне терминала (или закрыть окно).
#
# Скрипт ищет бинарник: рядом с собой, в target/release проекта
# (если лежит в корне или в launchers/), иначе собирает cargo'й.
#
# ВАЖНО: разрешение Accessibility должно быть выдано терминалу,
# который открывает .command-файлы (обычно Terminal.app):
# System Settings → Privacy & Security → Accessibility
# ============================================================
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

find_bin() {
    for cand in "$SCRIPT_DIR/autotyper" \
                "$SCRIPT_DIR/target/release/autotyper" \
                "$SCRIPT_DIR/../target/release/autotyper"; do
        if [ -x "$cand" ]; then
            echo "$cand"
            return 0
        fi
    done
    return 1
}

BIN="$(find_bin)" || BIN=""

# Бинарника нет — пробуем собрать в корне проекта (там, где Cargo.toml).
if [ -z "$BIN" ]; then
    PROJ_DIR="$SCRIPT_DIR"
    [ -f "$PROJ_DIR/Cargo.toml" ] || PROJ_DIR="$SCRIPT_DIR/.."
    echo "[запуск] Бинарник не найден, собираю (cargo build --release)..."
    if ! (cd "$PROJ_DIR" && cargo build --release); then
        echo "[ошибка] Сборка не удалась. Установите Rust: https://rustup.rs"
        read -n 1 -s -r -p "Нажмите любую клавишу, чтобы закрыть..."
        exit 1
    fi
    BIN="$(find_bin)"
    if [ -z "$BIN" ]; then
        echo "[ошибка] Бинарник так и не найден"
        read -n 1 -s -r -p "Нажмите любую клавишу, чтобы закрыть..."
        exit 1
    fi
fi

echo "[запуск] Бинарник:   $BIN"
echo "[запуск] Настройки: CONFIG.json рядом с бинарником (правьте этот файл)"
echo "[запуск] Лог:       autotyper.log рядом с бинарником"
echo "[запуск] Останов:   Ctrl+C или закройте это окно"
exec "$BIN"
