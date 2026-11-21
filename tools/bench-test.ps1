# LetsCam's in-game bench on Horizon's mod API (docs/development.md). It runs its own throwaway
# dedicated server from the plain nightly the mod pins (offline, 127.0.0.1 only) under
# $SdkDev\bench\server, and the offline bot lc_a plays the newest native build with LetsCam from
# the isolated launcher whose data lives in $SdkDev (horizon-sdk-dev). Build LetsCam there first:
#   & "$SdkDev\hl.ps1" native-build <repo>\mod\letscam --version $BaseVersion --trust-native-code
# The scenario: ~40 s walking up to a small house, through its door, the inventory open inside,
# back out; the key editor saves 3 keyframes; then `/letscam render auto+orbit+side+top+keys sheet`
# and ffprobe checks of the auto video. No SoulsCraft here (no mob phase labels). The bot is muted
# and never records a microphone (the owner's rules). Needs pwsh 7: Windows PowerShell 5.1
# misreads the Cyrillic in this file.
param(
    [string]$SdkDev = 'C:\Claude code projects\horizon-sdk-dev',
    [string]$BaseVersion = 'nightly-2026-11-21-8f83e5c',  # the plain nightly horizon-mod.toml pins
    [string]$OutputRoot = 'F:\letscam\modapi-bench',
    [string]$Version = '',          # native-<commit>-<key>; default: the newest one
    [int]$RenderTimeoutSec = 900,
    [string]$Angles = 'auto+orbit+side+top',
    [switch]$NoKeys,
    [switch]$KeepServer             # leave the bench server running at the end
)
$ErrorActionPreference = 'Stop'
# ffprobe prints UTF-8 (the track title «Игра»); the console's OEM code page garbled it.
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$S = $SdkDev
$env:HORIZON_LAUNCHER_HOME = "$S\launcher"; $env:HORIZON_HOME = "$S\game"
$env:HORIZON_LAUNCHER_NO_SELF_UPDATE = '1'
$Base = "$S\launcher\versions\$BaseVersion"
$Resources = "$Base\.local\horizon-client\26.3"
$ServerTool = "$Base\horizon-server.exe"
$ServerDir = "$S\bench\server"
$ServerPort = 25590
$GameDir = "$S\bench\client"
$Scratch = "$S\bench\player"
$ConfigFile = Join-Path $GameDir 'config\letscam-client.toml'
$Ffprobe = 'ffprobe'
# Another session's performance measurement holds this lock: a bench now would spoil it.
$HeavyLock = 'C:\ai-bridge\heavy.lock'
while (Test-Path $HeavyLock) {
    Write-Host "waiting: $HeavyLock is held"
    Start-Sleep -Seconds 30
}
if (-not $Version) {
    $commit = ($BaseVersion -split '-')[-1]
    $Version = (Get-ChildItem "$S\launcher\versions" -Directory -Filter "native-$commit-*" |
        Where-Object { Test-Path "$($_.FullName)\.installed" } | Sort-Object LastWriteTime | Select-Object -Last 1).Name
}
$Client = "$S\launcher\versions\$Version\horizon-client.exe"
$Ctl = "$S\launcher\versions\$Version\horizon-ctl.exe"
Write-Host "client: $Client"
Write-Host "mods: $((Get-Content -Raw "$S\launcher\versions\$Version\native-mods.json" | ConvertFrom-Json).mods | ForEach-Object { "$($_.id) $($_.version)" })"

# The owner's 87% CPU cap for everything this bench starts (same job object as hl.ps1).
. ([scriptblock]::Create(((Get-Content 'C:\Claude code projects\horizon-joint-dev\joint.ps1')[273..326] -join "`n")))
$CpuJob = [JointCpuCap]::Create(87)
function Add-ToJob([int]$ProcessId) {
    try {
        $p = Get-Process -Id $ProcessId -ErrorAction Stop
        if (-not [JointCpuCap]::Contains($CpuJob, $p.Handle)) { [JointCpuCap]::Assign($CpuJob, $p.Handle) }
        Add-Descendants $CpuJob $ProcessId
    } catch { Write-Warning "CPU cap not applied to ${ProcessId}: $_" }
}

function Invoke-Bot($Bot, [string]$Line, [int]$Timeout = 60) {
    $words = $Line -split ' '
    $out = & $Ctl --socket $Bot.Socket --timeout $Timeout @words 2>&1
    if ($LASTEXITCODE -ne 0) { Write-Warning "$($Bot.Name): $Line -> $out" }
    $out
}

function Invoke-Console([string]$Line) {
    $out = & $ServerTool --dir $ServerDir cmd $Line 2>&1
    if ($LASTEXITCODE -ne 0) { Write-Warning "console: $Line -> $out" }
    $out
}

function Test-ServerUp {
    & $ServerTool --dir $ServerDir cmd 'list' 2>$null | Out-Null
    $LASTEXITCODE -eq 0
}

function Start-BenchServer {
    if (Test-ServerUp) { Write-Host 'bench server: already up'; return }
    $logs = "$S\bench\logs"; New-Item -ItemType Directory -Force $logs | Out-Null
    $starter = Start-Process $ServerTool -ArgumentList '--dir', "`"$ServerDir`"", 'start' -PassThru -NoNewWindow `
        -RedirectStandardOutput "$logs\server-start.out" -RedirectStandardError "$logs\server-start.err"
    $starter.WaitForExit()
    if ($starter.ExitCode -ne 0) { throw "bench server did not start: $(Get-Content -Raw "$logs\server-start.err")" }
    Get-BenchServerProcesses | ForEach-Object { Add-ToJob $_.ProcessId }
    Write-Host 'bench server: started'
}

function Get-BenchServerProcesses {
    Get-CimInstance Win32_Process -Filter "Name = 'horizon-server.exe' OR Name = 'horizon-client.exe'" |
        Where-Object { $_.CommandLine -like "*$ServerDir*" }
}

function Stop-BenchServer {
    & $ServerTool --dir $ServerDir stop 2>&1 | Write-Host
}

# lc_a offscreen at the given window size, waited for until it is in the world. The game reads
# LetsCam's config at `config/letscam-client.toml` relative to its working folder, so it starts
# from the game folder.
function Start-LcBot([int]$Width = 1920, [int]$Height = 1080) {
    New-Item -ItemType Directory -Force $Scratch, $GameDir | Out-Null
    Set-BotSilent $Scratch
    Set-BotSilent $GameDir
    $socket = Join-Path $Scratch 'lc_a.sock'
    Push-Location $GameDir
    $oldCwd = [Environment]::CurrentDirectory
    [Environment]::CurrentDirectory = $GameDir
    try {
        $reply = & $Ctl --socket $socket launch --game $Client --offscreen --resources $Resources --home $GameDir `
            --game-dir $GameDir --player-dir $Scratch --name lc_a --server "127.0.0.1:$ServerPort" `
            --width $Width --height $Height 2>&1
        if ($LASTEXITCODE -ne 0) { throw "launch failed: $reply" }
    } finally {
        [Environment]::CurrentDirectory = $oldCwd
        Pop-Location
    }
    $launched = ($reply -join '') | ConvertFrom-Json
    Add-ToJob $launched.pid
    $bot = [pscustomobject]@{ Name = 'lc_a'; Socket = $socket; Pid = $launched.pid }
    Invoke-Bot $bot 'wait-ready 120' 130 | Out-Null
    $bot
}

# Owner's rule: bots make no sound on the PC's speakers; master volume 0 before every launch.
function Set-BotSilent([string]$PlayerDir) {
    $options = Join-Path $PlayerDir 'options.txt'
    $lines = if (Test-Path $options) { @(Get-Content $options | Where-Object { $_ -notmatch '^(soundCategory_master|tutorialStep):' }) } else { @() }
    Set-Content -Encoding utf8 $options ($lines + 'soundCategory_master:0.0' + 'tutorialStep:none')
}

function Stop-LcBot($Bot) {
    if (-not $Bot) { return }
    & $Ctl --socket $Bot.Socket quit 2>&1 | Out-Null
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-Process -Id $Bot.Pid -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    if (Get-Process -Id $Bot.Pid -ErrorAction SilentlyContinue) { Stop-Process -Id $Bot.Pid -Force }
}

# The test's config: never the microphone (owner's rule), output on F:.
function Set-TestConfig {
    New-Item -ItemType Directory -Force (Split-Path $ConfigFile) | Out-Null
    $tomlRoot = $OutputRoot -replace '\\', '/'
    Set-Content -Encoding utf8 $ConfigFile "record_mic = false`nkeep_sources = true`noutput_dir = `"$tomlRoot`"`n"
}

function Wait-Message($Bot, [string[]]$Patterns, [int]$TimeoutSec) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $text = (Invoke-Bot $Bot 'messages' 10) -join "`n"
        $log = [IO.Path]::ChangeExtension($Bot.Socket, 'log')
        if (Test-Path $log) { $text += "`n" + ((Get-Content -Tail 200 $log) -join "`n") }
        foreach ($p in $Patterns) {
            if ($text -match $p) { return $Matches[0] }
        }
        Start-Sleep -Seconds 2
    }
    throw "none of [$($Patterns -join ', ')] in chat after $TimeoutSec s"
}

function Wait-Renders($Bot, [int]$Count, [int]$TimeoutSec) {
    $log = [IO.Path]::ChangeExtension($Bot.Socket, 'log')
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $lines = if (Test-Path $log) { Get-Content $log | Where-Object { $_ -match 'letscam: ' } } else { @() }
        $failure = $lines | Where-Object { $_ -match 'Рендер остановлен|не удался|не открылся|не начался' } | Select-Object -First 1
        if ($failure) { return "FAIL: $failure" }
        $done = @($lines | Where-Object { $_ -match 'Ролик готов' })
        if ($done.Count -ge $Count) { return ($done -join "`n") }
        Start-Sleep -Seconds 3
    }
    "FAIL: $Count renders not done after $TimeoutSec s"
}

function Get-NewRecording([datetime]$Since) {
    Get-ChildItem $OutputRoot -Directory | Where-Object { $_.LastWriteTime -ge $Since } |
        Sort-Object Name | Select-Object -Last 1
}

$AngleList = @($Angles -split '\+' | Where-Object { $_ })
Set-TestConfig
$recordLog = Join-Path $Scratch 'lc_a-record.log'
Remove-Item -LiteralPath $recordLog -Force -ErrorAction SilentlyContinue

$x0 = -500; $y0 = -60; $z0 = -500
function Build-Set {
    $lines = @(
        "fill $($x0-12) $y0 $($z0-12) $($x0+12) $($y0+8) $($z0+16) air",
        "fill $($x0-12) $($y0-1) $($z0-12) $($x0+12) $($y0-1) $($z0+16) stone",
        "fill $($x0-3) $y0 $($z0-3) $($x0+3) $($y0+3) $($z0+3) oak_planks hollow",
        "setblock $x0 $y0 $($z0+3) oak_door[half=lower,facing=south,open=true]",
        "setblock $x0 $($y0+1) $($z0+3) oak_door[half=upper,facing=south,open=true]",
        "setblock $($x0+2) $($y0+2) $($z0-2) wall_torch[facing=south]"
    )
    foreach ($line in $lines) { Write-Host "console: $line -> $(Invoke-Console $line)" }
}

Start-BenchServer
$bot = $null
$started = Get-Date
try {
    $bot = Start-LcBot 1920 1080
    Invoke-Console "gamemode creative lc_a" | Out-Null
    # Daylight, as on the joint bench (the throwaway world's clock runs on between runs).
    Invoke-Console "time set 1000" | Out-Null
    Invoke-Console "tp lc_a $x0 -40 $($z0+12) 180 0" | Out-Null
    Start-Sleep -Seconds 6
    $feet = ((Invoke-Bot $bot 'state') | ConvertFrom-Json).player.position[1]
    $script:y0 = [int][math]::Floor($feet + 0.01)
    Write-Host "ground: the bot's feet at y $feet, the set at y $y0"
    Build-Set
    Invoke-Console "kill @e[type=slime,x=$($x0-30),y=$($y0-5),z=$($z0-30),dx=60,dy=30,dz=60]" | Out-Null
    Start-Sleep -Seconds 2

    Invoke-Bot $bot 'chat /letscam' | Out-Null
    $buttons = Wait-Message $bot @('letscam: window open, buttons [^\n]*', 'открывается только[^\n]*') 20
    Write-Host $buttons
    Start-Sleep -Milliseconds 800
    Invoke-Bot $bot "screenshot $(Join-Path $Scratch 'window.png')" | Out-Null
    $script:windowStarted = $buttons -match '"Начать запись", \(([\d.]+), ([\d.]+)\)'
    if ($windowStarted) {
        Invoke-Bot $bot "click-at $([int][double]$Matches[1]) $([int][double]$Matches[2])" | Out-Null
    } else {
        Invoke-Bot $bot 'key esc tap' | Out-Null
        Invoke-Bot $bot 'chat /letscam start' | Out-Null
    }
    Wait-Message $bot @('Запись началась', 'не начал') 20 | Write-Host
    Start-Sleep -Seconds 3
    Invoke-Bot $bot 'look 180 0' | Out-Null
    Invoke-Bot $bot 'move forward 2.9' 30 | Write-Host
    Start-Sleep -Seconds 2
    Invoke-Bot $bot 'key e tap' | Out-Null
    Start-Sleep -Seconds 4
    Invoke-Bot $bot 'key esc tap' | Out-Null
    Start-Sleep -Seconds 2
    Invoke-Bot $bot 'look 0 0' | Out-Null
    Invoke-Bot $bot 'move forward 2.5' 30 | Write-Host
    Invoke-Bot $bot 'look -90 0' | Out-Null
    Invoke-Bot $bot 'move forward 1.5' 30 | Write-Host
    Start-Sleep -Seconds 9
    # Stopped with the command: horizon-ctl's `key` knows F1-F6 and F11 only, not LetsCam's F9.
    Invoke-Bot $bot 'chat /letscam stop' | Out-Null
    $script:stopLine = Wait-Message $bot @('Запись сохранена[^\n]*', 'остановлена с ошибкой[^\n]*') 30
    Write-Host $stopLine

    $renderAngles = $Angles
    if (-not $NoKeys) {
        Invoke-Bot $bot 'chat /letscam edit' | Out-Null
        Wait-Message $bot @('letscam: key editor open[^\n]*', 'не открылся[^\n]*') 120 | Write-Host
        Start-Sleep -Seconds 5
        foreach ($i in 1..3) {
            Invoke-Bot $bot 'key n tap' | Out-Null
            if ($i -lt 3) {
                foreach ($s in 1..5) { Invoke-Bot $bot 'key right tap' | Out-Null; Start-Sleep -Milliseconds 700 }
                Invoke-Bot $bot 'key d down' | Out-Null
                Start-Sleep -Seconds 1
                Invoke-Bot $bot 'key d up' | Out-Null
            }
        }
        Invoke-Bot $bot 'key enter tap' | Out-Null
        $script:keysLine = Wait-Message $bot @('letscam: key editor closed, \d+ keyframes') 60
        Write-Host $keysLine
        Copy-Item -LiteralPath ([IO.Path]::ChangeExtension($bot.Socket, 'log')) -Destination $recordLog -Force
        Stop-LcBot $bot
        $bot = $null
        Start-Sleep -Seconds 3
        $bot = Start-LcBot 1920 1080
        $renderAngles = "$Angles+keys"
        $script:AngleList += 'keys'
    }

    Invoke-Bot $bot "chat /letscam render $renderAngles sheet" | Out-Null
    $done = Wait-Renders $bot $AngleList.Count ($RenderTimeoutSec * $AngleList.Count)
    Write-Host $done
    Invoke-Bot $bot "screenshot $(Join-Path $Scratch 'after-render.png')" | Out-Null
} finally {
    $finalLog = if ($bot) { [IO.Path]::ChangeExtension($bot.Socket, 'log') } else { $null }
    Stop-LcBot $bot
    if ($finalLog -and (Test-Path $finalLog)) { Copy-Item -LiteralPath $finalLog -Destination (Join-Path $Scratch 'lc_a-render.log') -Force }
    if (-not $KeepServer) { Stop-BenchServer }
}

$renderLog = Join-Path $Scratch 'lc_a-render.log'
if ($done -like 'FAIL:*') { throw "render failed: $done" }
$dir = Get-NewRecording $started
$stem = "letscam_$($dir.Name)"
$video = Get-ChildItem $dir.FullName -File | Where-Object { $_.BaseName -eq $stem } | Select-Object -First 1
if (-not $video) { throw "no $stem video in $($dir.FullName)" }
$angleChecks = [ordered]@{}
foreach ($angle in $AngleList) {
    $word = ($angle -split ':')[0]
    $suffix = if ($word -eq 'auto') { '' } else { "_$word" }
    if ($suffix) {
        $angleChecks["video $word"] = [bool](Get-ChildItem $dir.FullName -File | Where-Object { $_.BaseName -eq "$stem$suffix" })
    }
    $angleChecks["sheet$suffix"] = Test-Path (Join-Path $dir.FullName "$stem${suffix}_sheet_01.png")
}
$probe = & $Ffprobe -v error -show_entries 'stream=index,codec_type,codec_name,width,height,r_frame_rate:stream_tags=title,handler_name:format=duration' `
    -of json $video.FullName | ConvertFrom-Json
$session = Get-Content -Raw (Join-Path $dir.FullName 'session.json') | ConvertFrom-Json
$v = $probe.streams | Where-Object codec_type -eq 'video'
$a = @($probe.streams | Where-Object codec_type -eq 'audio')
$duration = [double]$probe.format.duration
$expected = $session.duration_ms / 1000.0
$checks = [ordered]@{
    'video 1920x1080'      = ($v.width -eq 1920 -and $v.height -eq 1080)
    'fps 60 or 30'         = ($v.r_frame_rate -in '60/1', '30/1')
    'audio track «Игра»'   = [bool]($a | Where-Object { $_.tags.title -eq 'Игра' -or $_.tags.handler_name -eq 'Игра' })
    'no mic track'         = -not ($a | Where-Object { $_.tags.title -eq 'Микрофон' -or $_.tags.handler_name -eq 'Микрофон' })
    'length ≈ recording'   = ([math]::Abs($duration - $expected) -le 0.1)
}
foreach ($k in $angleChecks.Keys) { $checks[$k] = $angleChecks[$k] }
$checks['window starts recording'] = [bool]$windowStarted
$checks['command stops recording'] = [bool]($stopLine -match 'Запись сохранена')
if (-not $NoKeys) {
    $checks['key editor saved 3 keys'] = [bool]($keysLine -match 'closed, 3 keyframes')
    $checks['render follows 3 keys'] = [bool](Select-String -Path $renderLog -Pattern 'keys path with 3 keyframes' -Quiet)
}
Select-String -Path $renderLog -Pattern 'letscam: render (stats|loop): [^"]*' | ForEach-Object { Write-Host $_.Matches[0].Value }
$drifts = @(Select-String -Path $renderLog -Pattern 'replay drift -?\d+ ms \(max (-?\d+)\)' |
    ForEach-Object { [math]::Abs([int]$_.Matches[0].Groups[1].Value) })
$driftOk = $drifts.Count -ge $AngleList.Count -and ($drifts | Measure-Object -Maximum).Maximum -le 60
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead((Join-Path $dir.FullName 'replay.mcpr'))
try {
    $reader = [IO.StreamReader]::new($zip.GetEntry('metaData.json').Open())
    $replayMs = ($reader.ReadToEnd() | ConvertFrom-Json).duration
    $reader.Dispose()
} finally { $zip.Dispose() }
Write-Host "replay: $replayMs ms of $($session.duration_ms) ms recorded"
$logs = @($renderLog) + @($recordLog | Where-Object { Test-Path $_ })
$stopped = Select-String -Path $logs -Pattern 'Реплей перестал записываться[^"]*' | Select-Object -First 1
if ($stopped) { Write-Host "recorder: $($stopped.Matches[0].Value)" }
if ($stopped) { Write-Host "replay keeps time: not judged, the recorder stopped" } else { $checks['replay keeps time'] = $driftOk }
$lastMove = 0
for ($i = 1; $i -lt $session.track.Count; $i++) {
    $p = $session.track[$i - 1]; $q = $session.track[$i]
    $d = [math]::Abs($q.eye[0] - $p.eye[0]) + [math]::Abs($q.eye[2] - $p.eye[2]) + [math]::Abs($q.yaw - $p.yaw)
    if ($d -gt 0.001) { $lastMove = $q.t_ms }
}
Write-Host "player last moved at $lastMove ms"
$checks['replay covers the action'] = ($replayMs -ge $lastMove - 500) -and -not $stopped
$errors = @(Select-String -Path $logs -Pattern 'ERROR|panicked|letscam: .*(ошибк|error)' | ForEach-Object { $_.Line })
Write-Host "client log errors: $($errors.Count)"; $errors | Select-Object -First 10 | ForEach-Object { Write-Host "  $_" }
Write-Host "folder: $($dir.FullName)"
Get-ChildItem $dir.FullName -File | ForEach-Object { Write-Host ("  {0,-48} {1,12:N0}" -f $_.Name, $_.Length) }
Write-Host "video: $($video.FullName)"
Write-Host ("streams: " + (($probe.streams | ForEach-Object { "$($_.codec_type)/$($_.codec_name) $($_.width)x$($_.height) $($_.r_frame_rate) $($_.tags.title)$($_.tags.handler_name)" }) -join '; '))
Write-Host "duration: $duration s, recording: $expected s"
$failed = 0
foreach ($k in $checks.Keys) {
    Write-Host ("{0,-26} {1}" -f $k, $(if ($checks[$k]) { 'ok' } else { $failed++; 'FAIL' }))
}
if ($failed) { exit 1 }
