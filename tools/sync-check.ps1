# Compares where the replay shows the player (the render's `letscam: sync t_ms=… replay_eye=[…]`
# log lines) with where the recorded track says he was at the same time (session.json), and
# prints the gap per sample plus the time shift that fits best. A good recording is within a
# fraction of a block at shift 0.
#   & tools\sync-check.ps1 -Log <client .log> -Recording <recording folder>
param(
    [Parameter(Mandatory)][string]$Log,
    [Parameter(Mandatory)][string]$Recording
)
$ErrorActionPreference = 'Stop'

$track = (Get-Content -Raw (Join-Path $Recording 'session.json') | ConvertFrom-Json).track
$samples = foreach ($line in Get-Content $Log) {
    if ($line -match 'letscam: sync t_ms=(\d+) replay_eye=\[([-\d.e]+), ([-\d.e]+), ([-\d.e]+)\]') {
        [pscustomobject]@{ T = [double]$Matches[1]; P = @([double]$Matches[2], [double]$Matches[3], [double]$Matches[4]) }
    }
}
if (-not $samples) { throw "no 'letscam: sync' lines in $Log" }

# The track's eye at time $t (ms), linear between its 20 Hz samples.
function Get-TrackEye([double]$t) {
    if ($t -le $track[0].t_ms) { return $track[0].eye }
    for ($i = 1; $i -lt $track.Count; $i++) {
        if ($track[$i].t_ms -ge $t) {
            $a = $track[$i - 1]; $b = $track[$i]
            $f = ($t - $a.t_ms) / [math]::Max(1, $b.t_ms - $a.t_ms)
            return @(0..2 | ForEach-Object { $a.eye[$_] + ($b.eye[$_] - $a.eye[$_]) * $f })
        }
    }
    $track[-1].eye
}

function Get-Gap($s, [double]$shift) {
    $e = Get-TrackEye ($s.T + $shift)
    [math]::Sqrt((0..2 | ForEach-Object { [math]::Pow($s.P[$_] - $e[$_], 2) } | Measure-Object -Sum).Sum)
}

Write-Host ('{0,8} {1,28} {2,28} {3,6}' -f 't_ms', 'replay eye', 'track eye', 'gap')
foreach ($s in $samples) {
    $e = Get-TrackEye $s.T
    $fmt = { param($p) '{0,8:F2} {1,7:F2} {2,8:F2}' -f $p[0], $p[1], $p[2] }
    Write-Host ('{0,8} {1,28} {2,28} {3,6:F2}' -f $s.T, (& $fmt $s.P), (& $fmt $e), (Get-Gap $s 0))
}
$best = -5000..5000 | Where-Object { $_ % 100 -eq 0 } | ForEach-Object {
    $shift = $_
    [pscustomobject]@{ Shift = $shift; Mean = ($samples | ForEach-Object { Get-Gap $_ $shift } | Measure-Object -Average).Average }
} | Sort-Object Mean | Select-Object -First 1
$mean0 = ($samples | ForEach-Object { Get-Gap $_ 0 } | Measure-Object -Average).Average
Write-Host ("mean gap at shift 0: {0:F2} blocks; best shift {1} ms with mean gap {2:F2}" -f $mean0, $best.Shift, $best.Mean)
