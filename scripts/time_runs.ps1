# Times two xenon-emu builds over N headless frames, alternating, to spot performance regressions.
# Usage: .\xenon-emu\scripts\time_runs.ps1 -Old <exe> [-New <exe>] [-Frames 400] [-Rounds 3]
param([Parameter(Mandatory)][string]$Old, [string]$New, [int]$Frames = 400, [int]$Rounds = 3)
$repo = Split-Path -Parent $PSScriptRoot
$root = Split-Path -Parent $repo
if (-not $New) { $New = Join-Path $repo 'target\release\xenon-emu.exe' }
foreach ($v in [Environment]::GetEnvironmentVariables().Keys) { if ($v -like 'XENON_*') { [Environment]::SetEnvironmentVariable($v, $null) } }
$env:XENON_HEADLESS = '1'; $env:XENON_SKIP_BOOTANIM = '1'; $env:XENON_MAX_FRAMES = "$Frames"
Push-Location $root
foreach ($r in 1..$Rounds) {
  foreach ($p in @(@('old', $Old), @('new', $New))) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $pr = Start-Process $p[1] -NoNewWindow -PassThru -RedirectStandardOutput "$env:TEMP\xenon_time.out" -RedirectStandardError "$env:TEMP\xenon_time.err"
    if (-not $pr.WaitForExit(150000)) { Stop-Process -Id $pr.Id -Force }
    "run$r $($p[0]): $([math]::Round($sw.Elapsed.TotalSeconds, 2)) s"
  }
}
Pop-Location
foreach ($v in [Environment]::GetEnvironmentVariables().Keys) { if ($v -like 'XENON_*') { [Environment]::SetEnvironmentVariable($v, $null) } }
