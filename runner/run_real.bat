@echo off
setlocal enabledelayedexpansion

echo ========================================================
echo   [Xenon-Emu] Inicializacao para Teste Real do Host
echo ========================================================

cd /d "%~dp0\.."
set REPO_DIR=%CD%
cd ..
set ROOT_DIR=%CD%

set EXE=%REPO_DIR%\target\release\xenon-emu.exe
if not exist "%EXE%" (
    echo [!] Executavel nao encontrado em: %EXE%
    echo [*] Compilando em modo Release primeiro...
    cd /d "%REPO_DIR%"
    cargo build --release
    if !ERRORLEVEL! NEQ 0 (
        echo [!] Falha na compilacao!
        pause
        exit /b !ERRORLEVEL!
    )
    cd /d "%ROOT_DIR%"
)

echo [*] Configurando ambiente de teste real:
echo     - Host Real (OS-backed): XENON_HOST=real
echo     - Rede Real Opt-in:      XENON_NET_ONLINE=1
echo     - Modo Grafico:          Janela 1280x720 HD @ 60 FPS
echo     - Entradas:              Teclado virtual + Gamepad XInput
echo     - Audio:                 XAudio PCM / Host Audio

set XENON_HOST=real
set XENON_NET_ONLINE=1
set XENON_HEADLESS=0

echo.
echo [*] Executando xenon-emu a partir de: %ROOT_DIR%
echo ========================================================
"%EXE%" %*

if %ERRORLEVEL% NEQ 0 (
    echo.
    echo [!] O emulador encerrou com codigo de saida: %ERRORLEVEL%
) else (
    echo.
    echo [+] Execucao finalizada normalmente.
)
pause
