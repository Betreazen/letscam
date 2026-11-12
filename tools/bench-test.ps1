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
    [int]$RenderTimeoutSec = 900
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
$configBackup = Set-TestConfig

# The set: a 7x7 plank house with a door on its south side, a stone floor around it.
# The flat check world stands at y -60 (the bench's check world is superflat). Built once the
# bot stands there: the server only edits loaded chunks.
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

$bot = Start-LcBot $Scratch 1920 1080
$started = Get-Date
try {
    Invoke-Console "gamemode creative lc_a" | Out-Null
    Invoke-Console "tp lc_a $x0 $y0 $($z0+12) 180 0" | Out-Null
    Start-Sleep -Seconds 5
    Build-Set
    Invoke-Console "kill @e[type=slime,x=$($x0-30),y=$($y0-5),z=$($z0-30),dx=60,dy=30,dz=60]" | Out-Null
    Start-Sleep -Seconds 2

    Invoke-Bot $bot 'chat /letscam start' | Out-Null
    Wait-Message $bot @('Запись началась', 'не начал') 20 | Write-Host
    Start-Sleep -Seconds 3
    # Up to the door, through it, into the middle of the house.
    Invoke-Bot $bot "goto $x0 $y0 $($z0+5)" 60 | Out-Null
    Invoke-Bot $bot "goto $x0 $y0 $z0" 60 | Out-Null
    Start-Sleep -Seconds 2
    Invoke-Bot $bot 'key e tap' | Out-Null
    Start-Sleep -Seconds 4
    Invoke-Bot $bot 'key esc tap' | Out-Null
    Start-Sleep -Seconds 2
    # Back out and away.
    Invoke-Bot $bot "goto $x0 $y0 $($z0+6)" 60 | Out-Null
    Invoke-Bot $bot "goto $($x0+6) $y0 $($z0+12)" 60 | Out-Null
    Start-Sleep -Seconds 3
    Invoke-Bot $bot 'chat /letscam stop' | Out-Null
    Wait-Message $bot @('Запись сохранена[^\n]*', 'остановлена с ошибкой[^\n]*') 30 | Write-Host

    # One recording, four angles in a row, each with its contact sheet.
    Invoke-Bot $bot 'chat /letscam render auto+orbit+side+top sheet' | Out-Null
    $done = Wait-Renders $bot 4 ($RenderTimeoutSec * 4)
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
$angles = [ordered]@{}
foreach ($angle in 'orbit', 'side', 'top') {
    $angles["video $angle"] = [bool](Get-ChildItem $dir.FullName -File | Where-Object { $_.BaseName -eq "${stem}_$angle" })
}
foreach ($s in '', '_orbit', '_side', '_top') {
    $angles["sheet$s"] = Test-Path (Join-Path $dir.FullName "$stem${s}_sheet_01.png")
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
foreach ($k in $angles.Keys) { $checks[$k] = $angles[$k] }
Write-Host "video: $($video.FullName)"
Write-Host ("streams: " + (($probe.streams | ForEach-Object { "$($_.codec_type)/$($_.codec_name) $($_.tags.title)$($_.tags.handler_name)" }) -join '; '))
Write-Host "duration: $duration s, recording: $expected s"
$failed = 0
foreach ($k in $checks.Keys) {
    Write-Host ("{0,-22} {1}" -f $k, $(if ($checks[$k]) { 'ok' } else { $failed++; 'FAIL' }))
}
if ($failed) { exit 1 }
