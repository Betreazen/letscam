# A/B render timing of LetsCam builds (docs/development.md): the bot lc_a renders the same
# recording with each build in turn, A B A B…, on the bench server of bench-test.ps1, and the
# client log's `letscam: render loop` / `render stats` lines are tabled per run. Alternating in one
# sitting is the only fair comparison on this PC (timings drift between hours). Each round (one
# render per build) waits its turn in the PC's queue (heavy-queue.ps1) and gives the lock back
# after it. Close OBS, other servers and GPU jobs first. The bot is muted and never records
# (render only). Needs pwsh 7.
#   pwsh -File tools\render-ab.ps1 -Versions native-8f83e5c-aaaa,native-8f83e5c-bbbb -Recording 2026-11-22_01-55-42
param(
    [Parameter(Mandatory)][string[]]$Versions,
    [Parameter(Mandatory)][string]$Recording,     # folder name under $OutputRoot
    [int]$Rounds = 3,
    # With sheets: the end of a render (video and contact sheets) is timed too.
    [string]$Angle = 'auto sheet',
    [string]$SdkDev = 'C:\Claude code projects\horizon-sdk-dev',
    [string]$BaseVersion = 'nightly-2026-11-21-8f83e5c',
    [string]$OutputRoot = 'F:\letscam\modapi-bench',
    [int]$RenderTimeoutSec = 900
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
# `pwsh -File` hands a comma list over as one string.
$Versions = @($Versions | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$S = $SdkDev
$env:HORIZON_LAUNCHER_HOME = "$S\launcher"; $env:HORIZON_HOME = "$S\game"
$env:HORIZON_LAUNCHER_NO_SELF_UPDATE = '1'
$Resources = "$S\launcher\versions\$BaseVersion\.local\horizon-client\26.3"
$ServerTool = "$S\launcher\versions\$BaseVersion\horizon-server.exe"
$ServerDir = "$S\bench\server"
$GameDir = "$S\bench\client"
$Scratch = "$S\bench\player"

if (-not (Test-Path (Join-Path $OutputRoot "$Recording\replay.mcpr"))) { throw "no recording $Recording in $OutputRoot" }
foreach ($v in $Versions) { if (-not (Test-Path "$S\launcher\versions\$v\horizon-client.exe")) { throw "no build $v" } }

. ([scriptblock]::Create(((Get-Content 'C:\Claude code projects\horizon-joint-dev\joint.ps1')[273..326] -join "`n")))
$CpuJob = [JointCpuCap]::Create(87)

. (Join-Path $PSScriptRoot 'heavy-queue.ps1')

function Invoke-Bot([string]$Ctl, [string]$Socket, [string]$Line, [int]$Timeout = 60) {
    & $Ctl --socket $Socket --timeout $Timeout @($Line -split ' ') 2>&1
}

function Measure-Render([string]$Version) {
    $client = "$S\launcher\versions\$Version\horizon-client.exe"
    $ctl = "$S\launcher\versions\$Version\horizon-ctl.exe"
    $socket = Join-Path $Scratch 'lc_a.sock'
    $log = [IO.Path]::ChangeExtension($socket, 'log')
    Push-Location $GameDir
    [Environment]::CurrentDirectory = $GameDir
    $reply = & $ctl --socket $socket launch --game $client --offscreen --resources $Resources --home $GameDir `
        --game-dir $GameDir --player-dir $Scratch --name lc_a --server '127.0.0.1:25590' --width 1920 --height 1080 2>&1
    Pop-Location
    if ($LASTEXITCODE -ne 0) { throw "launch failed: $reply" }
    $botPid = (($reply -join '') | ConvertFrom-Json).pid
    try {
        [JointCpuCap]::Assign($CpuJob, (Get-Process -Id $botPid).Handle)
        Invoke-Bot $ctl $socket 'wait-ready 120' 130 | Out-Null
        Start-Sleep -Seconds 3
        $clock = [Diagnostics.Stopwatch]::StartNew()
        Invoke-Bot $ctl $socket "chat /letscam render $Recording $Angle" | Out-Null
        $deadline = (Get-Date).AddSeconds($RenderTimeoutSec)
        while ((Get-Date) -lt $deadline) {
            $done = Select-String -Path $log -Pattern 'Ролик готов|Рендер остановлен|не удался' -ErrorAction SilentlyContinue | Select-Object -Last 1
            if ($done) { break }
            Start-Sleep -Seconds 2
        }
        $wall = $clock.Elapsed.TotalSeconds
        if (-not $done -or $done.Line -notmatch 'Ролик готов') { throw "$Version render: $($done.Line)" }
        $loop = (Select-String -Path $log -Pattern 'letscam: render loop: .*' | Select-Object -Last 1).Matches[0].Value
        $stats = (Select-String -Path $log -Pattern 'letscam: render stats: .*' | Select-Object -Last 1).Matches[0].Value
        [pscustomobject]@{
            Version = $Version
            GameFrameMs = [double]([regex]::Match($loop, 'game frame ([\d.]+) ms').Groups[1].Value)
            SubmitMs = [double]([regex]::Match($stats, 'submit ([\d.]+) ms').Groups[1].Value)
            PipMs = [double]([regex]::Match($stats, 'PiP ([\d.]+) ms').Groups[1].Value)
            Fps = [double]([regex]::Match($stats, '\(([\d.]+) fps\)').Groups[1].Value)
            Retries = [int]([regex]::Match($loop, '(\d+) retries').Groups[1].Value)
            # From the command to «Ролик готов»: start, frames, video and sheets (±2 s of polling).
            WallSec = [math]::Round($wall, 1)
        }
    } finally {
        & $ctl --socket $socket quit 2>&1 | Out-Null
        Start-Sleep -Seconds 5
        if (Get-Process -Id $botPid -ErrorAction SilentlyContinue) { Stop-Process -Id $botPid -Force }
    }
}

$results = @()
$serverUp = $false
$sitting = Get-Date
function Start-AbServer {
    & $ServerTool --dir $ServerDir cmd 'list' 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) {
        # Through files, as bench-test.ps1 does: the server keeps a captured pipe open forever,
        # and waiting for its end held the heavy lock for hours (2026-11-22).
        $logs = "$S\bench\logs"; New-Item -ItemType Directory -Force $logs | Out-Null
        $starter = Start-Process $ServerTool -ArgumentList '--dir', "`"$ServerDir`"", 'start' -PassThru -NoNewWindow `
            -RedirectStandardOutput "$logs\server-start.out" -RedirectStandardError "$logs\server-start.err"
        if (-not $starter.WaitForExit(120000)) { $starter.Kill(); throw 'bench server: start did not return in 120 s' }
        if ($starter.ExitCode -ne 0) { throw "bench server did not start: $(Get-Content -Raw "$logs\server-start.err")" }
        $script:serverUp = $true
    }
}

try {
    foreach ($round in 1..$Rounds) {
        Enter-HeavyQueue "render A/B round $round ($($Versions -join ' vs '))" 30
        try {
            $busy = Get-Process obs64 -ErrorAction SilentlyContinue
            if ($busy) { Write-Warning "OBS is running: the numbers of this round are suspect" }
            Start-AbServer
            foreach ($v in $Versions) {
                $r = Measure-Render $v
                $r | Add-Member Round $round
                Write-Host ("round {0} {1}: game frame {2} ms, submit {3} ms (PiP {4}), {5} fps, {6} retries, {7} s in all" -f $round, $v, $r.GameFrameMs, $r.SubmitMs, $r.PipMs, $r.Fps, $r.Retries, $r.WallSec)
                $results += $r
            }
        } finally {
            Exit-HeavyQueue
        }
    }
} finally {
    if ($serverUp) { & $ServerTool --dir $ServerDir stop 2>&1 | Out-Null }
    # The renders of this sitting are measurements, not keepers; older files stay.
    Get-ChildItem (Join-Path $OutputRoot $Recording) -File -Filter "letscam_${Recording}*" |
        Where-Object { $_.LastWriteTime -ge $sitting } | Remove-Item -Force
}
$results | Group-Object Version | ForEach-Object {
    $g = $_.Group
    "{0}: game frame {1:N1} ms, submit {2:N2} ms, PiP {3:N2} ms, {4:N1} fps, {6:N1} s in all (median of {5})" -f $_.Name,
        (($g.GameFrameMs | Sort-Object)[[math]::Floor($g.Count / 2)]), (($g.SubmitMs | Sort-Object)[[math]::Floor($g.Count / 2)]),
        (($g.PipMs | Sort-Object)[[math]::Floor($g.Count / 2)]), (($g.Fps | Sort-Object)[[math]::Floor($g.Count / 2)]), $g.Count,
        (($g.WallSec | Sort-Object)[[math]::Floor($g.Count / 2)])
}
