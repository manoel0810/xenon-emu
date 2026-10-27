<#
.SYNOPSIS
    Script de compilacao do Xenon-Emu (Release ou Debug).
.EXAMPLE
    .\runner\build.ps1
    .\runner\build.ps1 -Profile Debug
#>
param(
    [ValidateSet("Release", "Debug")]
    [string]$Profile = "Release"
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoDir = Split-Path -Parent $scriptDir

Write-Host "========================================================" -ForegroundColor Cyan
Write-Host "  [Xenon-Emu] Compilação do Emulador Xbox 360" -ForegroundColor Cyan
Write-Host "========================================================" -ForegroundColor Cyan
Write-Host "[*] Repositório: $repoDir"
Write-Host "[*] Perfil de compilação: $Profile"

Push-Location $repoDir
try {
    if ($Profile -eq "Release") {
        cargo build --release
    } else {
        cargo build
    }
    Write-Host "`n[+] Compilação concluída com sucesso!" -ForegroundColor Green
    $binPath = Join-Path $repoDir "target\$($Profile.ToLower())\xenon-emu.exe"
    Write-Host "[+] Binário disponível em: $binPath" -ForegroundColor Green
} finally {
    Pop-Location
}
