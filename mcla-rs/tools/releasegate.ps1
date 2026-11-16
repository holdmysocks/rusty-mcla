# releasegate.ps1 - the frame-rate and soak check of a release candidate (docs/PERFORMANCE.md "Release gate").
#
#   powershell -File tools/releasegate.ps1 -Rc <candidate mcla.exe> -Old <last release mcla.exe> [-Out work\release\gate]
#              [-SoakMinutes 30] [-BikeMinutes 15] [-BikeSave work\physics2\soak\full_bike.json] [-NoSoak]
#
# 1. tools/perfgate.ps1, the candidate against the last release, all cores: roam, night, race, garage, hangout.
# 2. The same on 4 cores (affinity 0x55): roam, race.
# 3. A soak of the candidate and of the last release side by side (the machine's two game slots): free roam (--drive
#    --auto --traffic 110) for -SoakMinutes, then tools/soak.py on the candidate with the release's run as --ref (memory
#    growth at most 20 % over the release's and private memory under 7 GB, slow ticks, frame rate drift, panics).
# 4. The same soak on the ZX-14 (perfgate's `bike` scenario) for -BikeMinutes, from a copy of -BikeSave: a save whose
#    current car is the bike, since the save's car replaces --car (the v0.22.0 soak's "bike" legs drove the R8).
#    soak.py --vehicle fails the leg when the game's `driven:` lines name anything else. Every leg prints what it drove.
# PASS when neither perf gate prints REGRESSION, no run of theirs was left out (busy machine / GPU shared: run again),
# and both soaks pass. Needs a quiet machine (about 100 min with the soaks); the gates wait for it run by run.
param(
  [Parameter(Mandatory)][string]$Rc,
  [Parameter(Mandatory)][string]$Old,
  [string]$Out = "",
  [int]$SoakMinutes = 30,
  [int]$BikeMinutes = 15,
  [string]$BikeSave = "",
  [switch]$NoSoak
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot\..\..").Path
if (-not $Out) { $Out = Join-Path $repo "work\release\gate" }
if (-not $BikeSave) { $BikeSave = Join-Path $repo "work\physics2\soak\full_bike.json" }
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
# (the candidate and the last release side by side for $min minutes of perfgate scenario $scen, from save $save (none:
# perfgate's default), then soak.py; $veh: the vehicle the leg must have driven)
function Soak([string]$scen, [int]$min, [string]$save, [string]$veh) {
  "=== soak $scen, $min min ($(Get-Date -Format HH:mm))"
  $jobs = foreach ($x in @(@($Rc, 'rc'), @($Old, 'old'))) {
    $j = Start-Job -ArgumentList $gate, $x[0], ($min * 60), "$Out\soak_${scen}_$($x[1])", $scen, $save -ScriptBlock {
      param($g, $e, $s, $o, $sc, $sv)
      $a = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $g, '-Exe', $e, '-Scen', $sc, '-Seconds', $s, '-NoQuiet', '-Out', $o)
      if ($sv) { $a += @('-Save', $sv) }
      & powershell @a 2>&1 | Out-Null }
    Start-Sleep 20; $j
  }
  $jobs | Wait-Job | Remove-Job
  $a = @("$PSScriptRoot\soak.py", "$Out\soak_${scen}_rc\runs", "b1-$scen-vulkan-r1", '--minutes', $min,
         '--ref', "$Out\soak_${scen}_old\runs", "b1-$scen-vulkan-r1")
  if ($veh) { $a += @('--vehicle', $veh) }
  & python -I @a
  if ($LASTEXITCODE) { $script:fails += "soak $scen" }
}
if (-not $NoSoak) {
  Soak 'roam' $SoakMinutes '' ''
  if (Test-Path $BikeSave) {
    $bs = "$Out\bike_save.json"
    New-Item -ItemType Directory $Out -Force | Out-Null
    Copy-Item $BikeSave $bs -Force
    Soak 'bike' $BikeMinutes $bs 'vp_kaw_ninja_zx14_06'
  } else { $fails += "bike soak: no save $BikeSave (one whose car is vp_kaw_ninja_zx14_06)" }
}
"=== $(Get-Date -Format HH:mm)"
if ($fails.Count) { "RELEASE GATE FAIL: $($fails -join '; ')" } else { "RELEASE GATE PASS" }
