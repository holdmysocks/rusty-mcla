# releasegate.ps1 - the frame-rate and soak check of a release candidate (docs/PERFORMANCE.md "Release gate").
#
#   powershell -File tools/releasegate.ps1 -Rc <candidate mcla.exe> -Old <last release mcla.exe> [-Out work\release\gate]
#              [-SoakMinutes 30] [-NoSoak]
#
# 1. tools/perfgate.ps1, the candidate against the last release, all cores: roam, night, race, garage, hangout.
# 2. The same on 4 cores (affinity 0x55): roam, race.
# 3. A soak of the candidate: free roam (--drive --auto --traffic 110) for -SoakMinutes, then tools/soak.py (memory growth,
#    slow ticks, frame rate drift, panics).
# PASS when neither perf gate prints REGRESSION, no run of theirs was left out (busy machine / GPU shared: run again),
# and the soak passes. Needs a quiet machine (about 85 min with the soak); the gates wait for it run by run.
param(
  [Parameter(Mandatory)][string]$Rc,
  [Parameter(Mandatory)][string]$Old,
  [string]$Out = "",
  [int]$SoakMinutes = 30,
  [switch]$NoSoak
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot\..\..").Path
if (-not $Out) { $Out = Join-Path $repo "work\release\gate" }
$gate = "$PSScriptRoot\perfgate.ps1"
$fails = @()
function Gate([string]$name, [string[]]$extra) {
  "=== $name ($(Get-Date -Format HH:mm))"
  $a = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $gate, '-Exe', $Old, '-Exe2', $Rc) + $extra
  $o = & powershell @a 2>&1 | ForEach-Object { "$_" }
  # (perfgate.py's table and verdict)
  $o | Where-Object { $_ -match '^scenario|^\w+ +b[12]|GPU passes|ticks:|^REGRESSION|^no regression|no result' }
  if ($o -match '^REGRESSION') { $script:fails += "$name regression" }
  if ($o -match 'no result') { $script:fails += "$name has runs left out (run again on a quiet machine)" }
}
Gate "all cores, last release (b1) vs candidate (b2)" @('-Scen', 'roam,night,race,garage,hangout', '-Out', "$Out\std")
Gate "4 cores" @('-Scen', 'roam,race', '-Affinity', '85', '-Out', "$Out\c4")
if (-not $NoSoak) {
  "=== soak, $SoakMinutes min ($(Get-Date -Format HH:mm))"
  & powershell -NoProfile -ExecutionPolicy Bypass -File $gate -Exe $Rc -Scen roam -Seconds ($SoakMinutes * 60) -Out "$Out\soak" 2>&1 | Out-Null
  & python -I "$PSScriptRoot\soak.py" "$Out\soak\runs" "b1-roam-vulkan-r1" --minutes $SoakMinutes
  if ($LASTEXITCODE) { $fails += "soak" }
}
"=== $(Get-Date -Format HH:mm)"
if ($fails.Count) { "RELEASE GATE FAIL: $($fails -join '; ')" } else { "RELEASE GATE PASS" }
