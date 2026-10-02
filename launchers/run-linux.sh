#!/bin/bash
# ============================================================
# autotyper — запуск (Linux)
# Запуск из терминала:  ./run-linux.sh
# Двойной клик зависит от рабочего стола: обычно файловый менеджер
# предложит «Запустить» (нужен chmod +x). Для настоящего двойного
# клика используйте autotyper.desktop рядом с этим файлом.
# Останов: Ctrl+C.
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
        exit 1
    fi
    BIN="$(find_bin)"
    if [ -z "$BIN" ]; then
        echo "[ошибка] Бинарник так и не найден"
        exit 1
    fi
fi

echo "[запуск] Бинарник:   $BIN"
echo "[запуск] Настройки: CONFIG.json рядом с бинарником (правьте этот файл)"
echo "[запуск] Лог:       autotyper.log рядом с бинарником"
echo "[запуск] Останов:   Ctrl+C"
exec "$BIN"
