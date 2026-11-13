# perfgate.ps1 - the standard performance scenarios, one or more builds in turn (docs/PERFORMANCE.md "Perf gate").
#
#   powershell -File tools/perfgate.ps1 -Exe target\release\mcla.exe [-Exe2 old.exe] [-Scen roam,night,race,garage,hangout]
#              [-Backend vulkan|dx12] [-Seconds 150] [-Repeat 1] [-Out work\<session>\perfgate] [-Affinity 0] [-NoQuiet]
#
# Each run: a user folder per exe and backend under -Out (its shader cache kept between runs), a copy of -Save and a
# fixed settings file (1920x1080 windowed, no vsync, no cap) put back before the run, MCLA_FRAMETIMES / MCLA_FRAMELOG /
# MCLA_SLOWTICK on, and the process's CPU, GPU 3D engine, dedicated VRAM and private memory sampled every 2 s.
# Before each run it waits (at most 5 min) for the machine to be quiet: no other exe running from the repo folder and
# the CPU under -QuietPct for 10 s. Then tools/perfgate.py prints the table (fps, 1 % low, worst frame, worst tick,
# phase means, GPU time per pass, VRAM) and, with two builds, the change of each and the regressions.
# At most one game runs at a time (the 2-instance rule of the machine counts other sessions' games too).
param(
  [Parameter(Mandatory)][string]$Exe,
  [string]$Exe2 = "",
  [string[]]$Scen = @('roam', 'night', 'race', 'garage', 'hangout'),
  [string]$Backend = "vulkan",
  [int]$Seconds = 150,
  [int]$Repeat = 1,
  [string]$Out = "",
  [string]$Data = "D:/MCLA/extracted/cache",
  [string]$Save = "$env:LOCALAPPDATA\RustyMCLA\saves\mcla_save.json",
  [int]$Affinity = 0,
  [double]$QuietPct = 12,
  [switch]$NoQuiet
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot\..\..").Path
if (-not $Out) { $Out = Join-Path $repo "work\perfgate" }
New-Item -ItemType Directory "$Out\runs" -Force | Out-Null
$scenArgs = @{
  roam    = "--drive --auto --traffic 110 --time 13";
  night   = "--drive --auto --traffic 110 --time 23";
  race    = "--race-name hl_ordered_ts_01 --auto";
  garage  = "--garage";
  hangout = "--drive --pos 728.0,8.0,-300.0 --time 21 --traffic 110";
  old     = "--auto --traffic 20"
}
$settings = "schema = 1`n`n[display]`nfps_cap = 0`nfps_monitor = `"frame_time`"`nresolution = `"1920x1080`"`nvsync = false`nwindow_mode = `"windowed`"`n`n[game]`nintro = `"fast`"`n"
$exes = @($Exe) + @(if ($Exe2) { $Exe2 })
$log = "$Out\perfgate.log"

function Wait-Quiet {
  if ($NoQuiet) { return "not checked" }
  $t0 = Get-Date
  while (@(Get-Process | Where-Object { $_.Path -like "$repo\*" -and $_.Path -notlike "$Out\*" -and $_.MainWindowHandle -ne 0 }).Count -gt 0 -and ((Get-Date) - $t0).TotalMinutes -lt 5) { Start-Sleep 10 }
  $ok = 0; $t1 = Get-Date
  while ($ok -lt 5 -and ((Get-Date) - $t1).TotalMinutes -lt 5) {
    $v = (Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 2 -MaxSamples 1).CounterSamples[0].CookedValue
    if ($v -lt $QuietPct) { $ok++ } else { $ok = 0 }
  }
  return $(if ($ok -ge 5) { "quiet" } else { "NOT QUIET" })
}

function Invoke-Run([string]$exe, [int]$k, [string]$scen, [int]$rep) {
  $tag = "b$k-$scen-$Backend-r$rep"
  $ud = "$Out\user_b${k}_$Backend"
  New-Item -ItemType Directory "$ud\saves" -Force | Out-Null
  if (Test-Path $Save) { Copy-Item $Save "$ud\saves\mcla_save.json" -Force }
  Set-Content -Path "$ud\settings.toml" -Value $settings -Encoding ascii -NoNewline
  $q = Wait-Quiet
  # (never a third game on the machine: wait while two run)
  while (@(Get-Process | Where-Object { $_.Path -like "*\mcla*.exe" -or $_.Path -like "$repo\work\*.exe" } | Where-Object { $_.MainWindowHandle -ne 0 }).Count -ge 2) { Start-Sleep 10 }
  $psi = New-Object System.Diagnostics.ProcessStartInfo $exe, "--data $Data $($scenArgs[$scen])"
  $psi.WorkingDirectory = $repo; $psi.UseShellExecute = $false; $psi.RedirectStandardError = $true; $psi.RedirectStandardOutput = $true
  $env1 = @{ MCLA_SETTINGS = "$ud\settings.toml"; RUSTY_MCLA_USER_DIR = $ud; WGPU_BACKEND = $Backend; MCLA_FRAMELOG = "50";
             MCLA_FRAMETIMES = "$Out\runs\$tag.ft.csv"; MCLA_SLOWTICK = "30" }
  foreach ($kv in $env1.GetEnumerator()) { $psi.EnvironmentVariables[$kv.Key] = $kv.Value }
  $p = [System.Diagnostics.Process]::Start($psi)
  if ($Affinity) { try { $p.ProcessorAffinity = [IntPtr]$Affinity } catch {} }
  $errTask = $p.StandardError.ReadToEndAsync(); $outTask = $p.StandardOutput.ReadToEndAsync()
  $csv = "$Out\runs\$tag.res.csv"
  "sec,cpu_pct,gpu3d_pct,dedicated_mb,private_mb,ws_mb,machine_cpu_pct" | Out-File $csv -Encoding ascii
  $n = [Environment]::ProcessorCount; $name = $p.ProcessName; $t0 = Get-Date
  while (-not $p.HasExited -and ((Get-Date) - $t0).TotalSeconds -lt $Seconds) {
    Start-Sleep 2
    $s = [int]((Get-Date) - $t0).TotalSeconds; $cpu = 0; $g = 0; $d = 0; $m = 0
    try {
      $cs = (Get-Counter -Counter @("\Process($name)\% Processor Time", "\GPU Engine(pid_$($p.Id)_*engtype_3D)\Utilization Percentage",
        "\GPU Process Memory(pid_$($p.Id)_*)\Dedicated Usage", "\Processor(_Total)\% Processor Time") -ErrorAction SilentlyContinue).CounterSamples
      foreach ($x in $cs) {
        if ($x.Path -like '*\process(*') { $cpu = $x.CookedValue / $n } elseif ($x.Path -like '*engtype_3d*') { $g += $x.CookedValue }
        elseif ($x.Path -like '*dedicated usage') { $d += $x.CookedValue } elseif ($x.Path -like '*processor(_total)*') { $m = $x.CookedValue }
      }
    } catch {}
    $p.Refresh()
    "$s,$([math]::Round($cpu,1)),$([math]::Round($g,1)),$([int]($d/1MB)),$([int]($p.PrivateMemorySize64/1MB)),$([int]($p.WorkingSet64/1MB)),$([math]::Round($m,1))" | Out-File $csv -Append -Encoding ascii
  }
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
  $p.WaitForExit(10000) | Out-Null
  $errTask.Result | Out-File "$Out\runs\$tag.err" -Encoding utf8
  "$(Get-Date -Format HH:mm:ss) $tag $q $exe" | Out-File $log -Append
  return $tag
}

$tags = @(); $si = 0
foreach ($r in 1..$Repeat) {
  foreach ($s in $Scen) {
    if (-not $scenArgs.ContainsKey($s)) { throw "unknown scenario $s" }
    # (two builds: the order swaps each scenario so a drift of the machine does not favour one)
    $order = if ($si % 2) { $exes[($exes.Count - 1)..0] } else { $exes }; $si++
    foreach ($e in $order) { $k = [array]::IndexOf($exes, $e) + 1; $tags += Invoke-Run $e $k $s $r }
  }
}
& python -I "$PSScriptRoot\perfgate.py" "$Out\runs" @tags
