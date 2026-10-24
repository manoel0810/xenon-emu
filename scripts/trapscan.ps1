# Runs the emulator once (headless, watchdog, timeout) and prints only what stopped the Metro primary thread.
# Usage: .\xenon-emu\scripts\trapscan.ps1 [-Frames 400] [-TimeoutSec 120] [-Build]
param([int]$Frames = 400, [int]$TimeoutSec = 120, [switch]$Build)
$args2 = @{ Frames = $Frames; TimeoutSec = $TimeoutSec; Env = @('XENON_HEADLESS=1', 'XENON_WATCHDOG=1', 'XENON_TRAP_TRACE=1') }
if (-not $Build) { $args2.NoBuild = $true }
& (Join-Path $PSScriptRoot 'iter.ps1') @args2 | Out-Null
$o = @(Get-Content "$env:TEMP\iter_run.txt")
$i = [Array]::FindIndex($o, [Predicate[string]] { param($l) $l -like '*TRAP TRACE*' })
if ($i -ge 0) {
    Write-Host ("TRAP: " + $o[$i].Trim())
    $o[($i + 1)..([Math]::Min($i + 25000, $o.Count - 1))] | Where-Object { $_ -match 'pc=0x' } | Select-Object -Last 6 |
        ForEach-Object { Write-Host ("   " + (($_.Trim() -replace '^\[r31 changed\] ', '') -replace ' r1=\w+ ', '  ')) }
} else {
    $t = $o | Where-Object { $_ -match 'THREAD TRAP|THREAD EXIT|WATCHDOG' } | Select-Object -First 3
    if ($t) { $t | ForEach-Object { Write-Host $_.Trim() } } else { Write-Host "NO TRAP: primary thread did not stop within $Frames frames." }
}
$e = Get-Content "$env:TEMP\iter_run.txt.err" -ErrorAction SilentlyContinue | Where-Object { $_ -match 'WATCHDOG' } | Select-Object -First 1
if ($e) { Write-Host $e }
$p = $o | Where-Object { $_ -match '\[THREAD\].*Primary' } | Select-Object -Last 1
if ($p) { Write-Host ("Primary at end: " + $p.Trim()) }
$g = $o | Where-Object { $_ -match 'GPU CP' } | Select-Object -Last 1
if ($g) { Write-Host ("GPU: " + $g.Trim()) }
