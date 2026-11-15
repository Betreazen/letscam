# RustCraft's monorail capsule for LetsCam, a scene the RustCraft session asked for (2026-11-12):
# the route of its slice 7 agent (stone deck at y 149, tier-4 rails at y 150 from x 98 to 172 along
# z 110 with a one-block climb at x 130..139, stations «Дом» at 97 150 110 and «Шахта» at
# 173 150 110), in RustCraft's area x 96..178, z 96..124. The bot lc_a records the trip «Дом» →
# «Шахта» and renders it side, orbit and top on the seat it rides (focus=ride), plus the
# automatic camera on the seat, each with a contact sheet. Cleans the route up after itself.
#
# Needs the joint client with LetsCam and RustCraft 0.6+, and the joint server in offline mode:
#   .\joint.ps1 open; .\joint.ps1 serve -Owner "letscam agent"   (and .\joint.ps1 close after)
param(
    [string]$Scratch = (Join-Path $env:TEMP 'letscam-capsule'),
    # The route's console lines, as the RustCraft session wrote them (comments and blanks skipped);
    # kept next to this script.
    [string]$Route = (Join-Path $PSScriptRoot 'capsule-route.txt'),
    [int]$RenderTimeoutSec = 900,
    # The render's angles, `+`-joined; `fp` is the rider's own view, where the cabin must hide.
    [string]$Angles = 'auto+side+orbit+top'
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
$AngleCount = @($Angles -split '\+' | Where-Object { $_ }).Count
# The station window's buttons in GUI pixels at 1280x720, GUI scale 3 (from the route's notes):
# the first row of the list («Шахта») and «Ехать», which stays grey until a row is picked.
$Row = '206 90'; $Go = '290 122'

$configBackup = Set-TestConfig
# The GUI clicks are for 1280x720; ffmpeg scales the video to the config's 1920x1080.
$bot = Start-LcBot $Scratch 1280 720
$started = Get-Date
try {
    Invoke-Console 'gamemode creative lc_a' | Out-Null
    Invoke-Console 'tp lc_a 99 150 108 -90 0' | Out-Null
    Start-Sleep -Seconds 5
    foreach ($line in Get-Content $Route) {
        $line = $line.Trim()
        if ($line -eq '' -or $line.StartsWith('#')) { continue }
        Write-Host "console: $line -> $(Invoke-Console $line)"
    }
    # The server picks up new rails and stations on its rescan, within a second. The first tp only
    # loaded the chunks (the bot fell through where the deck now is): onto the deck now.
    Start-Sleep -Seconds 3
    Invoke-Console 'tp lc_a 99 150 108 -90 0' | Out-Null
    Start-Sleep -Seconds 2
    Write-Host "climb up:   $(Invoke-Console 'data get block 129 150 110')"
    Write-Host "climb down: $(Invoke-Console 'data get block 140 150 110')"

    Invoke-Bot $bot 'chat /letscam start' | Out-Null
    Wait-Message $bot @('Запись началась', 'не начал') 20 | Write-Host
    Start-Sleep -Seconds 2
    Invoke-Bot $bot 'interact 97 150 110 east' | Write-Host
    Start-Sleep -Seconds 1
    Invoke-Bot $bot "click-at $Row" | Out-Null
    Start-Sleep -Milliseconds 500
    Invoke-Bot $bot "click-at $Go" | Out-Null
    # About 2.5 s of travel and the easing at both ends; then a moment at «Шахта».
    Start-Sleep -Seconds 7
    Invoke-Bot $bot 'state' | Write-Host
    Invoke-Bot $bot 'key shift tap' | Out-Null
    Start-Sleep -Seconds 2
    Invoke-Bot $bot 'chat /letscam stop' | Out-Null
    Wait-Message $bot @('Запись сохранена[^\n]*', 'остановлена с ошибкой[^\n]*') 30 | Write-Host

    Invoke-Bot $bot "chat /letscam render $Angles sheet focus=ride" | Out-Null
    $done = Wait-Renders $bot $AngleCount ($RenderTimeoutSec * $AngleCount)
    Write-Host $done
} finally {
    Stop-LcBot $bot
    Invoke-Console 'fill 96 149 106 178 153 114 minecraft:air' | Out-Null
    Restore-TestConfig $configBackup
}

if ($done -like 'FAIL:*') { throw "render failed: $done" }
$dir = Get-NewRecording $started
Write-Host "recording: $($dir.FullName)"
Get-ChildItem $dir.FullName -File | Where-Object { $_.Extension -in '.mp4', '.mkv', '.png' } |
    ForEach-Object { Write-Host ("  {0,-60} {1,8:N0} KB" -f $_.Name, ($_.Length / 1KB)) }
