<#
.SYNOPSIS
    Executa o emulador Xenon-Emu em modo de Teste Real com interface gráfica HD 720p,
    áudio host, rede real opt-in e suporte a teclado e controle XInput.
.EXAMPLE
    .\runner\run_real.ps1
    .\runner\run_real.ps1 -SkipBootAnim
#>
param(
    [switch]$SkipBootAnim,
    [switch]$NoBuild,
    [string[]]$ExtraEnv = @()
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoDir = Split-Path -Parent $scriptDir
$rootDir = Split-Path -Parent $repoDir

Write-Host "========================================================" -ForegroundColor Cyan
Write-Host "  [Xenon-Emu] Teste Real do Emulador Xbox 360" -ForegroundColor Cyan
Write-Host "========================================================" -ForegroundColor Cyan

# 1. Encerra processos residuais se houver
Get-Process -Name *xenon* -ErrorAction SilentlyContinue | Stop-Process -Force

# 2. Compilação automática se o binário não existir ou se não tiver -NoBuild
$exe = Join-Path $repoDir "target\release\xenon-emu.exe"
if (-not $NoBuild -or -not (Test-Path $exe)) {
    Write-Host "[*] Verificando compilação (Release)..." -ForegroundColor Yellow
    Push-Location $repoDir
    try {
        cargo build --release
    } finally {
        Pop-Location
    }
}

# 3. Limpa variáveis de diagnóstico antigas e define ambiente de teste real
Get-ChildItem Env: | Where-Object { $_.Name -like 'XENON_*' } | ForEach-Object { Remove-Item "Env:$($_.Name)" }

$env:XENON_HOST = 'real'
$env:XENON_NET_ONLINE = '1'
$env:XENON_HEADLESS = '0'

if ($SkipBootAnim) {
    $env:XENON_SKIP_BOOTANIM = '1'
    Write-Host "[*] Pulo de animação ativado (XENON_SKIP_BOOTANIM=1)" -ForegroundColor DarkYellow
}

foreach ($e in $ExtraEnv) {
    $k, $v = $e -split '=', 2
    Set-Item "Env:$k" $v
    Write-Host "[*] Variável customizada: $k=$v" -ForegroundColor DarkGray
}

Write-Host "[*] Executável: $exe" -ForegroundColor Green
Write-Host "[*] Diretório de trabalho: $rootDir" -ForegroundColor Green
Write-Host "`n[*] Controles da Janela Interativa:" -ForegroundColor Cyan
Write-Host "    - [WASD] / Setas: D-Pad e Analógico esquerdo"
Write-Host "    - [Espaço] / [Enter]: Botão (A)"
Write-Host "    - [Esc] / [Backspace]: Botão (B) / Cancelar"
Write-Host "    - [X] / [Y]: Botões (X) e (Y)"
Write-Host "    - [Q] / [E]: Bumpers (LB) e (RB)"
Write-Host "    - [Z] / [V]: Gatilhos (LT) e (RT)"
Write-Host "    - [F1] / [Home]: Start | [F2] / [Tab]: Back"
Write-Host "    - [Shift+ESC] ou clique no [X]: Fechar janela"
Write-Host "========================================================`n" -ForegroundColor Cyan

Push-Location $rootDir
try {
    & $exe
} finally {
    Pop-Location
}
