@echo off
setlocal enabledelayedexpansion

echo ========================================================
echo   [Xenon-Emu] Script de Compilacao do Emulador
echo ========================================================

cd /d "%~dp0\.."
set REPO_DIR=%CD%

set BUILD_PROFILE=release
if /i "%1"=="debug" (
    set BUILD_PROFILE=debug
    echo [*] Perfil selecionado: DEBUG
    cargo build
) else (
    echo [*] Perfil selecionado: RELEASE (Otimizado)
    cargo build --release
)

if %ERRORLEVEL% NEQ 0 (
    echo.
    echo [!] ERRO: A compilacao falhou com codigo de erro %ERRORLEVEL%!
    pause
    exit /b %ERRORLEVEL%
)

echo.
echo [+] Compilacao concluida com sucesso!
echo [+] Executavel gerado em: %REPO_DIR%\target\%BUILD_PROFILE%\xenon-emu.exe
echo ========================================================
