# Builds xenon-emu at a git ref in a temporary worktree and prints the path of its xenon-emu.exe.
# Used as the "old" side of cmp_diag.ps1 / time_runs.ps1 when checking that a refactor does not change behavior.
# Usage: $old = .\xenon-emu\scripts\build_ref.ps1 [-Ref HEAD]      (remove with -Clean when done)
param([string]$Ref = 'HEAD', [switch]$Clean)
$repo = Split-Path -Parent $PSScriptRoot
$wt = Join-Path $env:TEMP 'xenon_ref_build'
if ($Clean) {
    git -C $repo worktree remove --force $wt 2>$null | Out-Null
    return
}
if (Test-Path $wt) { git -C $repo worktree remove --force $wt 2>$null | Out-Null }
git -C $repo worktree add -q --detach $wt $Ref | Out-Null
Push-Location $wt
$b = cargo build --release -p xenon-emu --bin xenon-emu 2>&1 | Out-String
Pop-Location
if ($b -notmatch 'Finished') { Write-Host $b; throw "build of $Ref failed" }
Join-Path $wt 'target\release\xenon-emu.exe'
