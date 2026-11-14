# LetsCam on the joint bench (horizon-joint-dev/README.md): the offline bot lc_a records ~40 s in
# LetsCam's area (x −1000..0, z −1000..0): walking up to a small house, through its door, the
# inventory open inside, back out. Then `/letscam render auto+orbit+side+top sheet` makes four
# videos with contact sheets, and ffprobe checks the auto one:
# 1920x1080, the fps from the config, two audio tracks named «Игра» and «Микрофон» (here only
# «Игра»: the test never records the owner's microphone), the length of the recording ±1 frame.
#
# Needs a client build with LetsCam and the joint server running in offline mode:
#   cd "C:\Claude code projects\horizon-joint-dev"
#   .\joint.ps1 open; .\joint.ps1 serve -Owner "letscam agent"     (and .\joint.ps1 close after)
#   & "C:\Claude code projects\letscam\tools\bench-test.ps1"
param(
    [string]$Scratch = (Join-Path $env:TEMP 'letscam-bench'),
    [int]$RenderTimeoutSec = 900,
    # An empty doorway instead of the open oak door (to tell whether the door matters).
    [switch]$NoDoor,
    # The render command's angles, `+`-joined; `fixed:` shots look at the doorway from the south.
    [string]$Angles = 'auto+orbit+side+top'
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
$AngleList = @($Angles -split '\+' | Where-Object { $_ })
$configBackup = Set-TestConfig

# The set: a 7x7 plank house with a door on its south side, a stone floor around it.
# Built once the bot stands there: the server only edits loaded chunks. The height of the ground
# is taken from where the bot lands: `check` makes the joint world afresh, and its superflat
# layers have not always been the same (feet at y -60 one day, -61 the next).
$x0 = -500; $y0 = -60; $z0 = -500
function Build-Set {
    $lines = @(
        "fill $($x0-12) $y0 $($z0-12) $($x0+12) $($y0+8) $($z0+16) air",
        "fill $($x0-12) $($y0-1) $($z0-12) $($x0+12) $($y0-1) $($z0+16) stone",
        "fill $($x0-3) $y0 $($z0-3) $($x0+3) $($y0+3) $($z0+3) oak_planks hollow",
        $(if ($NoDoor) { "setblock $x0 $y0 $($z0+3) air" } else { "setblock $x0 $y0 $($z0+3) oak_door[half=lower,facing=south,open=true]" }),
        $(if ($NoDoor) { "setblock $x0 $($y0+1) $($z0+3) air" } else { "setblock $x0 $($y0+1) $($z0+3) oak_door[half=upper,facing=south,open=true]" }),
        "setblock $($x0+2) $($y0+2) $($z0-2) wall_torch[facing=south]"
    )
    foreach ($line in $lines) { Write-Host "console: $line -> $(Invoke-Console $line)" }
}

$bot = Start-LcBot $Scratch 1920 1080
$started = Get-Date
try {
    Invoke-Console "gamemode creative lc_a" | Out-Null
    # Dropped from above, so it lands on whatever the ground is.
    Invoke-Console "tp lc_a $x0 -40 $($z0+12) 180 0" | Out-Null
    Start-Sleep -Seconds 6
    $feet = ((Invoke-Bot $bot 'state') | ConvertFrom-Json).player.position[1]
    $script:y0 = [int][math]::Floor($feet + 0.01)
    Write-Host "ground: the bot's feet at y $feet, the set at y $y0"
    Build-Set
    Invoke-Console "kill @e[type=slime,x=$($x0-30),y=$($y0-5),z=$($z0-30),dx=60,dy=30,dz=60]" | Out-Null
    Start-Sleep -Seconds 2

    # Started from the window (a bare `/letscam` opens it): its record button, clicked where the
    # window logged it.
    Invoke-Bot $bot 'chat /letscam' | Out-Null
    $buttons = Wait-Message $bot @('letscam: window open, buttons [^\n]*', 'открывается только[^\n]*') 20
    Write-Host $buttons
    $script:windowStarted = $buttons -match '"Начать запись", \(([\d.]+), ([\d.]+)\)'
    if ($windowStarted) {
        Invoke-Bot $bot "click-at $([int][double]$Matches[1]) $([int][double]$Matches[2])" | Out-Null
    } else {
        Invoke-Bot $bot 'key esc tap' | Out-Null
        Invoke-Bot $bot 'chat /letscam start' | Out-Null
    }
    Wait-Message $bot @('Запись началась', 'не начал') 20 | Write-Host
    Start-Sleep -Seconds 3
    # Up to the door, through it, into the middle of the house: straight north along x of the door
    # (goto's path finder won't take the thin open door). ~4.3 blocks/s: 12.5 blocks to the middle.
    Invoke-Bot $bot 'look 180 0' | Out-Null
    Invoke-Bot $bot 'move forward 2.9' 30 | Write-Host
    Start-Sleep -Seconds 2
    Invoke-Bot $bot 'key e tap' | Out-Null
    Start-Sleep -Seconds 4
    Invoke-Bot $bot 'key esc tap' | Out-Null
    Start-Sleep -Seconds 2
    # Back out through the door and away to the east.
    Invoke-Bot $bot 'look 0 0' | Out-Null
    Invoke-Bot $bot 'move forward 2.5' 30 | Write-Host
    Invoke-Bot $bot 'look -90 0' | Out-Null
    Invoke-Bot $bot 'move forward 1.5' 30 | Write-Host
    Start-Sleep -Seconds 3
    Invoke-Bot $bot 'chat /letscam stop' | Out-Null
    Wait-Message $bot @('Запись сохранена[^\n]*', 'остановлена с ошибкой[^\n]*') 30 | Write-Host

    # One recording, four angles in a row, each with its contact sheet.
    Invoke-Bot $bot "chat /letscam render $Angles sheet" | Out-Null
    $done = Wait-Renders $bot $AngleList.Count ($RenderTimeoutSec * $AngleList.Count)
    Write-Host $done
    Invoke-Bot $bot "screenshot $(Join-Path $Scratch 'after-render.png')" | Out-Null
} finally {
    Stop-LcBot $bot
    Invoke-Console "fill $($x0-12) $y0 $($z0-12) $($x0+12) $($y0+8) $($z0+16) air" | Out-Null
    Invoke-Console "fill $($x0-12) $($y0-1) $($z0-12) $($x0+12) $($y0-1) $($z0+16) grass_block" | Out-Null
    Restore-TestConfig $configBackup
}

if ($done -like 'FAIL:*') { throw "render failed: $done" }
$dir = Get-NewRecording $started
$stem = "letscam_$($dir.Name)"
$video = Get-ChildItem $dir.FullName -File | Where-Object { $_.BaseName -eq $stem } | Select-Object -First 1
if (-not $video) { throw "no $stem video in $($dir.FullName)" }
$angleChecks = [ordered]@{}
# Each angle's file suffix is its preset word (`fixed:x,y,z` → `_fixed`), none for `auto`.
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
    'length ≈ recording'   = ([math]::Abs($duration - $expected) -le 0.1)
}
foreach ($k in $angleChecks.Keys) { $checks[$k] = $angleChecks[$k] }
$checks['window starts recording'] = [bool]$windowStarted
# How fast each angle rendered (the encoder's own timing, one line per finished render).
Select-String -Path ([IO.Path]::ChangeExtension($bot.Socket, 'log')) -Pattern 'LetsCam: \d+ frames in[^"]*' |
    ForEach-Object { Write-Host $_.Matches[0].Value }
# The studio recorder can stop on its own (LetsCam then says so in chat): the replay must cover
# the whole recording, or the second half of every video has no player.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead((Join-Path $dir.FullName 'replay.mcpr'))
try {
    $reader = [IO.StreamReader]::new($zip.GetEntry('metaData.json').Open())
    $replayMs = ($reader.ReadToEnd() | ConvertFrom-Json).duration
    $reader.Dispose()
} finally { $zip.Dispose() }
Write-Host "replay: $replayMs ms of $($session.duration_ms) ms recorded"
$stopped = Select-String -Path ([IO.Path]::ChangeExtension($bot.Socket, 'log')) -Pattern 'Реплей перестал записываться[^"]*' |
    Select-Object -First 1
if ($stopped) { Write-Host "recorder: $($stopped.Matches[0].Value)" }
# A replay ends at its last packet, so a quiet tail (the player standing still in a flat world)
# is no loss: it must reach the last time the player moved or turned, not the recording's end.
$lastMove = 0
for ($i = 1; $i -lt $session.track.Count; $i++) {
    $a = $session.track[$i - 1]; $b = $session.track[$i]
    $d = [math]::Abs($b.eye[0] - $a.eye[0]) + [math]::Abs($b.eye[2] - $a.eye[2]) + [math]::Abs($b.yaw - $a.yaw)
    if ($d -gt 0.001) { $lastMove = $b.t_ms }
}
Write-Host "player last moved at $lastMove ms"
$checks['replay covers the action'] = ($replayMs -ge $lastMove - 500) -and -not $stopped
Write-Host "video: $($video.FullName)"
Write-Host ("streams: " + (($probe.streams | ForEach-Object { "$($_.codec_type)/$($_.codec_name) $($_.tags.title)$($_.tags.handler_name)" }) -join '; '))
Write-Host "duration: $duration s, recording: $expected s"
$failed = 0
foreach ($k in $checks.Keys) {
    Write-Host ("{0,-22} {1}" -f $k, $(if ($checks[$k]) { 'ok' } else { $failed++; 'FAIL' }))
}
if ($failed) { exit 1 }
