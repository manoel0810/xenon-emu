# Runs two xenon-emu builds with identical XENON_* sets (headless) and diffs their stdout/stderr.
# Expected differences: only tie order in HashMap-based top-N tables (see docs/BASELINE.md).
# Usage: .\xenon-emu\scripts\cmp_diag.ps1 -Old <exe> [-New <exe>] [-TimeoutSec 150]
param([Parameter(Mandatory)][string]$Old, [string]$New, [int]$TimeoutSec = 150)
$repo = Split-Path -Parent $PSScriptRoot
$root = Split-Path -Parent $repo
if (-not $New) { $New = Join-Path $repo 'target\release\xenon-emu.exe' }
$out = Join-Path $env:TEMP 'xenon_cmp'
New-Item -ItemType Directory -Force $out | Out-Null
$sets = [ordered]@{
  logs   = @('XENON_MAX_FRAMES=400','XENON_PM4_LOG=1','XENON_SYSCALL_SUMMARY=1','XENON_TRAP_TRACE=1','XENON_WATCHDOG=1','XENON_GPU_MMIO_LOG=1')
  hooks  = @('XENON_MAX_FRAMES=150','XENON_PC_HOOK=0x921AFFF8,0x92198CD8','XENON_PC_HOOK_ALL=1','XENON_PC_HOOK_LIMIT=6','XENON_PC_HOOK_MIN_FRAME=10','XENON_PC_HOOK_MEM=31:10:8','XENON_PC_HOOK_STRREGS=3,4','XENON_PC_HOOK_R3STR=1','XENON_PC_HOOK_LIST=31','XENON_PC_PROFILE=1')
  watch  = @('XENON_MAX_FRAMES=40','XENON_WATCH=0x90102000','XENON_WATCH_MEM=1','XENON_WATCH_DUMP_HIT=2','XENON_TRAP_TRACE=1','XENON_TRAP_FULL=1','XENON_DUMP_RING_FRAME=30','XENON_DUMP_RING_N=20','XENON_HEARTBEAT=1','XENON_LOG_SYSCALLS=1','XENON_SYSCALL_LOG=1','XENON_FB_SOURCE=reference','XENON_NO_RESTART=1')
}
foreach ($name in $sets.Keys) {
  foreach ($which in @('old','new')) {
    $exe = if ($which -eq 'old') { $Old } else { $New }
    Get-ChildItem Env: | Where-Object { $_.Name -like 'XENON_*' } | ForEach-Object { Remove-Item "Env:$($_.Name)" }
    $env:XENON_HEADLESS = '1'; $env:XENON_SKIP_BOOTANIM = '1'
    foreach ($e in $sets[$name]) { $k, $v = $e -split '=', 2; Set-Item "Env:$k" $v }
    $f = Join-Path $out "$name.$which.txt"
    Push-Location $root
    $p = Start-Process -FilePath $exe -NoNewWindow -PassThru -RedirectStandardOutput $f -RedirectStandardError "$f.err"
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; "TIMEOUT $name $which" }
    Pop-Location
  }
  $a = Get-Content (Join-Path $out "$name.old.txt"); $b = Get-Content (Join-Path $out "$name.new.txt")
  $c = @(Compare-Object $a $b)
  $errSame = (Get-FileHash (Join-Path $out "$name.old.txt.err")).Hash -eq (Get-FileHash (Join-Path $out "$name.new.txt.err")).Hash
  "${name}: stdout lines $($a.Count) vs $($b.Count), diffs=$($c.Count); stderr identical=$errSame"
  $c | Select-Object -First 6 | ForEach-Object { "  $($_.SideIndicator) $($_.InputObject)" }
}
Get-ChildItem Env: | Where-Object { $_.Name -like 'XENON_*' } | ForEach-Object { Remove-Item "Env:$($_.Name)" }
