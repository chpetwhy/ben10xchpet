@echo off
setlocal
cd /d "%~dp0"

if not exist "target\release\kod1.exe" (
    echo Binary yok - once: kur.bat calistir
    exit /b 1
)

target\release\kod1.exe
