@echo off
REM Derleme - once elle: rustup kur
setlocal enabledelayedexpansion
cd /d "%~dp0"

REM Cargo kontrolu
where cargo >nul 2>&1
if errorlevel 1 (
    echo HATA: cargo yok - once rustup kur
    exit /b 1
)

REM Node kontrolu
where node >nul 2>&1
if errorlevel 1 (
    echo HATA: node yok - MFA icin node lazim
    exit /b 1
)

echo === kod1 derleniyor ===
cargo build --release --bin kod1

if errorlevel 1 (
    echo HATA: Derleme basarisiz
    exit /b 1
)

echo === TAMAM - baslat.bat calistir ===
