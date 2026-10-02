@echo off
rem ============================================================
rem autotyper — запуск двойным кликом (Windows)
rem Двойной клик открывает консольное окно с утилитой.
rem Останов: Ctrl+C или просто закройте окно.
rem Скрипт ищет бинарник: рядом с собой, в target\release проекта
rem (если лежит в корне или в launchers\), иначе собирает cargo'й.
rem Файл сохранён в UTF-8, chcp 65001 включает UTF-8 в консоли.
rem ============================================================
chcp 65001 >nul
setlocal
set "SCRIPTDIR=%~dp0"

set "BIN="
if exist "%SCRIPTDIR%autotyper.exe" set "BIN=%SCRIPTDIR%autotyper.exe"
if not defined BIN if exist "%SCRIPTDIR%target\release\autotyper.exe" set "BIN=%SCRIPTDIR%target\release\autotyper.exe"
if not defined BIN if exist "%SCRIPTDIR%..\target\release\autotyper.exe" set "BIN=%SCRIPTDIR%..\target\release\autotyper.exe"

rem Бинарника нет — пробуем собрать в корне проекта (там, где Cargo.toml).
if not defined BIN (
    set "PROJDIR=%SCRIPTDIR%"
    if not exist "%SCRIPTDIR%Cargo.toml" set "PROJDIR=%SCRIPTDIR%.."
    echo [запуск] Бинарник не найден, собираю ^(cargo build --release^)...
    pushd "%PROJDIR%"
    cargo build --release
    popd
    if errorlevel 1 (
        echo [ошибка] Сборка не удалась. Установите Rust: https://rustup.rs
        pause
        exit /b 1
    )
    if exist "%SCRIPTDIR%autotyper.exe" set "BIN=%SCRIPTDIR%autotyper.exe"
    if not defined BIN if exist "%SCRIPTDIR%target\release\autotyper.exe" set "BIN=%SCRIPTDIR%target\release\autotyper.exe"
    if not defined BIN if exist "%SCRIPTDIR%..\target\release\autotyper.exe" set "BIN=%SCRIPTDIR%..\target\release\autotyper.exe"
)

if not defined BIN (
    echo [ошибка] Бинарник так и не найден
    pause
    exit /b 1
)

echo [запуск] Настройки: CONFIG.json рядом с бинарником ^(правьте этот файл^)
echo [запуск] Лог:       autotyper.log рядом с бинарником
echo [запуск] Останов:   Ctrl+C или закройте это окно
"%BIN%"
pause
