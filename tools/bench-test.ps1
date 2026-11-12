# LetsCam on the joint bench (horizon-joint-dev/README.md): the offline bot lc_a records ~40 s in
# LetsCam's area (x −1000..0, z −1000..0): walking up to a small house, through its door, the
# inventory open inside, back out. Then `/letscam render` makes the video, and ffprobe checks it:
# 1920x1080, the fps from the config, two audio tracks named «Игра» and «Микрофон» (here only
# «Игра»: the test never records the owner's microphone), the length of the recording ±1 frame.
#
# Needs a client build with LetsCam and the joint server running in offline mode:
#   cd "C:\Claude code projects\horizon-joint-dev"
#   .\joint.ps1 open; .\joint.ps1 serve -Owner "letscam agent"     (and .\joint.ps1 close after)
#   & "C:\Claude code projects\letscam\tools\bench-test.ps1"
param(
    [string]$Scratch = (Join-Path $env:TEMP 'letscam-bench'),
    [int]$RenderTimeoutSec = 900
)
$ErrorActionPreference = 'Stop'

$Ctl = "$env:LOCALAPPDATA\Horizon\current\horizon-ctl.exe"
$ServerTool = "$env:LOCALAPPDATA\Horizon\current\horizon-server.exe"
$JointServer = "$env:LOCALAPPDATA\Horizon\joint-dev\server"
$Instance = "$env:APPDATA\horizon\instances\joint-dev"
$Ffprobe = 'ffprobe'

function Get-JointClient {
    $instances = Get-Content -Raw "$env:APPDATA\horizon-launcher\instances.json" | ConvertFrom-Json
    $version = $instances.instances.'joint-dev'.version
    if (-not $version) { throw 'joint-dev plays no native build yet: .\joint.ps1 client' }
    "$env:APPDATA\horizon-launcher\versions\$version\horizon-client.exe"
}

function Invoke-Bot($Bot, [string]$Line, [int]$Timeout = 60) {
    $words = $Line -split ' '
    $out = & $Ctl --socket $Bot.Socket --timeout $Timeout @words 2>&1
    if ($LASTEXITCODE -ne 0) { Write-Warning "$($Bot.Name): $Line -> $out" }
    $out
}

function Invoke-Console([string]$Line) {
    $out = & $ServerTool --dir $JointServer cmd $Line 2>&1
    if ($LASTEXITCODE -ne 0) { Write-Warning "console: $Line -> $out" }
    $out
}

function Wait-Message($Bot, [string[]]$Patterns, [int]$TimeoutSec) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        # Chat, and the game's log beside the socket: the render's last line comes after the
        # replay closed, when the chat of the replay world is gone.
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

# The test's config: no microphone (never the owner's), the rest as shipped.
$configDir = Join-Path $Instance 'config'
New-Item -ItemType Directory -Force $configDir | Out-Null
$configFile = Join-Path $configDir 'letscam-client.toml'
$configBackup = $null
if (Test-Path $configFile) { $configBackup = Get-Content -Raw $configFile }
Set-Content -Encoding utf8 $configFile "record_mic = false`nkeep_sources = true`n"

# The set: a 7x7 plank house with a door on its south side, a stone floor around, at y 100.
$x0 = -500; $y0 = 100; $z0 = -500
Invoke-Console "fill $($x0-12) $($y0-1) $($z0-12) $($x0+12) $($y0+8) $($z0+16) air" | Out-Null
Invoke-Console "fill $($x0-12) $($y0-1) $($z0-12) $($x0+12) $($y0-1) $($z0+16) stone" | Out-Null
Invoke-Console "fill $($x0-3) $y0 $($z0-3) $($x0+3) $($y0+3) $($z0+3) oak_planks hollow" | Out-Null
Invoke-Console "setblock $x0 $y0 $($z0+3) oak_door[half=lower,facing=south,open=true]" | Out-Null
Invoke-Console "setblock $x0 $($y0+1) $($z0+3) oak_door[half=upper,facing=south,open=true]" | Out-Null
Invoke-Console "setblock $($x0+2) $($y0+2) $($z0-2) torch" | Out-Null

New-Item -ItemType Directory -Force $Scratch | Out-Null
$socket = Join-Path $Scratch 'lc_a.sock'
$reply = & $Ctl --socket $socket launch --game (Get-JointClient) --offscreen --home $Instance `
    --game-dir $Instance --player-dir $Scratch --name lc_a --server 127.0.0.1:25570 --width 1920 --height 1080 2>&1
if ($LASTEXITCODE -ne 0) { throw "launch failed: $reply" }
$bot = [pscustomobject]@{ Name = 'lc_a'; Socket = $socket }
$started = Get-Date
try {
    Invoke-Bot $bot 'wait-ready 120' 130 | Out-Null
    Invoke-Console "gamemode creative lc_a" | Out-Null
    Invoke-Console "tp lc_a $x0 $y0 $($z0+12) 180 0" | Out-Null
    Start-Sleep -Seconds 3

    Invoke-Bot $bot 'chat /letscam start' | Out-Null
    Wait-Message $bot @('Запись началась', 'не начал') 20 | Write-Host
    Start-Sleep -Seconds 3
    # Up to the door, through it, into the middle of the house.
    Invoke-Bot $bot "goto $x0 $y0 $($z0+5)" 60 | Out-Null
    Invoke-Bot $bot "goto $x0 $y0 $z0" 60 | Out-Null
    Start-Sleep -Seconds 2
    Invoke-Bot $bot 'key e' | Out-Null
    Start-Sleep -Seconds 4
    Invoke-Bot $bot 'key esc' | Out-Null
    Start-Sleep -Seconds 2
    # Back out and away.
    Invoke-Bot $bot "goto $x0 $y0 $($z0+6)" 60 | Out-Null
    Invoke-Bot $bot "goto $($x0+6) $y0 $($z0+12)" 60 | Out-Null
    Start-Sleep -Seconds 3
    Invoke-Bot $bot 'chat /letscam stop' | Out-Null
    Wait-Message $bot @('Запись сохранена[^\n]*', 'остановлена с ошибкой[^\n]*') 30 | Write-Host

    Invoke-Bot $bot 'chat /letscam render' | Out-Null
    $done = Wait-Message $bot @('Ролик готов[^\n]*', 'Рендер остановлен[^\n]*', 'не удался[^\n]*', 'не открылся[^\n]*', 'не начался[^\n]*') $RenderTimeoutSec
    Write-Host $done
    Invoke-Bot $bot 'screenshot' | Out-Null
} finally {
    & $Ctl --socket $socket quit 2>&1 | Out-Null
    Invoke-Console "fill $($x0-12) $($y0-1) $($z0-12) $($x0+12) $($y0+8) $($z0+16) air" | Out-Null
    if ($null -ne $configBackup) { Set-Content -Encoding utf8 -NoNewline $configFile $configBackup }
    else { Remove-Item -LiteralPath $configFile -Force }
}

if ($done -notmatch 'Ролик готов') { throw "render failed: $done" }
$root = Join-Path $Instance 'letscam'
$dir = Get-ChildItem $root -Directory | Where-Object { $_.LastWriteTime -ge $started } |
    Sort-Object Name | Select-Object -Last 1
$video = Get-ChildItem $dir.FullName -File | Where-Object { $_.Extension -in '.mp4', '.mkv' } | Select-Object -First 1
if (-not $video) { throw "no video in $($dir.FullName)" }
$probe = & $Ffprobe -v error -show_entries 'stream=index,codec_type,codec_name,width,height,r_frame_rate:stream_tags=title:format=duration' `
    -of json $video.FullName | ConvertFrom-Json
$session = Get-Content -Raw (Join-Path $dir.FullName 'session.json') | ConvertFrom-Json

$v = $probe.streams | Where-Object codec_type -eq 'video'
$a = @($probe.streams | Where-Object codec_type -eq 'audio')
$duration = [double]$probe.format.duration
$expected = $session.duration_ms / 1000.0
$checks = [ordered]@{
    'video 1920x1080'      = ($v.width -eq 1920 -and $v.height -eq 1080)
    'fps 60 or 30'         = ($v.r_frame_rate -in '60/1', '30/1')
    'audio track «Игра»'   = [bool]($a | Where-Object { $_.tags.title -eq 'Игра' })
    'length ≈ recording'   = ([math]::Abs($duration - $expected) -le 0.1)
}
Write-Host "video: $($video.FullName)"
Write-Host ("streams: " + (($probe.streams | ForEach-Object { "$($_.codec_type)/$($_.codec_name) $($_.tags.title)" }) -join '; '))
Write-Host "duration: $duration s, recording: $expected s"
$failed = 0
foreach ($k in $checks.Keys) {
    Write-Host ("{0,-22} {1}" -f $k, $(if ($checks[$k]) { 'ok' } else { $failed++; 'FAIL' }))
}
if ($failed) { exit 1 }
