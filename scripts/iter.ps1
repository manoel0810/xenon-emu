# Build xenon-emu, refresh XenonEmu.exe, run N frames and print a short summary.
# Usage: .\xenon-emu\scripts\iter.ps1 [-Frames 600] [-NoBuild] [-Out path] [-TimeoutSec 60] [-Env 'XENON_X=1','XENON_Y=2'] [-Exe path]
# The emulator runs from the workspace folder (parent of the repo), where dash.xex, $SystemUpdate etc. live.
param(
    [int]$Frames = 600,
    [switch]$NoBuild,
    [string]$Out = "$env:TEMP\iter_run.txt",
    [int]$TimeoutSec = 60,
    [string[]]$Env = @(),
    [string]$Exe   # run another build (e.g. from build_ref.ps1) instead of target\release; implies -NoBuild
)
$repo = Split-Path -Parent $PSScriptRoot
$root = Split-Path -Parent $repo
if (-not $NoBuild -and -not $Exe) {
    Push-Location $repo
    $b = cargo build --release 2>&1 | Out-String
    Pop-Location
    if ($b -match '(?m)^error') { Write-Host $b; throw 'build failed' }
}
if ($Exe) { $exe = $Exe } else {
    $exe = Join-Path $repo 'target\release\xenon-emu.exe'   # run the fresh build directly (XenonEmu.exe in the root may be locked)
    try { Copy-Item $exe (Join-Path $root 'XenonEmu.exe') -Force -ErrorAction Stop } catch { Write-Host 'note: root XenonEmu.exe is in use, left untouched' }
}
# clean diagnostic env, then apply the requested ones
Get-ChildItem Env: | Where-Object { $_.Name -like 'XENON_*' } | ForEach-Object { Remove-Item "Env:$($_.Name)" }
$env:XENON_SKIP_BOOTANIM = '1'
$env:XENON_MAX_FRAMES = "$Frames"
foreach ($e in $Env) { $k, $v = $e -split '=', 2; Set-Item "Env:$k" $v }
Push-Location $root
$p = Start-Process -FilePath $exe -NoNewWindow -PassThru -RedirectStandardOutput $Out -RedirectStandardError "$Out.err"
if (-not $p.WaitForExit($TimeoutSec * 1000)) {
    Write-Host "TIMEOUT after $TimeoutSec s -> killing emulator (hang)"
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    $p.WaitForExit(5000) | Out-Null
}
Pop-Location
$o = @(Get-Content $Out)
"lines: $($o.Count)  exit: $($p.ExitCode)  out: $Out"
$o | Where-Object { $_ -match 'THREAD TRAP|THREAD EXIT|Unimplemented|Reached XENON|panicked' } | Select-Object -First 8
$o | Where-Object { $_ -match '\[THREAD\].*(Primary|Dynamic)' } | Select-Object -Last 6 | ForEach-Object { $_.Trim() }
