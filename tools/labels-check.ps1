# SoulsCraft's attack phases on LetsCam's contact sheets, in game (docs/development.md). The bot
# lc_a plays a client built with both mods (letscam + soulscraft, one native-build of a folder
# holding both packages) on SoulsCraft's Linux server in the horizon-linux-dev box. It records a
# revenant's forced attacks with `/sc debug on`, renders `auto sheet`, and checks that the replay
# holds `soulscraft:state` messages; the sheets themselves are looked at by eye. The bot is muted
# and never records a microphone (the owner's rules). The server is SoulsCraft's: started only
# when no other /hz/servers/* runs, stopped at the end. Needs pwsh 7.
# Not run yet: the launcher can't build the two mods into one game (horizon-rust/horizon#19616).
param(
    [Parameter(Mandatory)][string]$Version,   # native-8f83e5c-<key> with letscam and soulscraft
    [string]$SdkDev = 'C:\Claude code projects\horizon-sdk-dev',
    [string]$BaseVersion = 'nightly-2026-11-21-8f83e5c',
    [string]$Hlx = 'C:\Claude code projects\horizon-linux-dev\hlx.ps1',
    [string]$ServerExe = '/hz/exports/soulscraft-2/horizon-server',
    [string]$ServerDir = '/hz/servers/soulscraft',
    [int]$ServerPort = 25591,
    [string]$OutputRoot = 'F:\letscam\labels-check',
    [string[]]$Attacks = @('overhead', 'thrust', 'slash'),
    [int]$RenderTimeoutSec = 900
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$S = $SdkDev
$env:HORIZON_LAUNCHER_HOME = "$S\launcher"; $env:HORIZON_HOME = "$S\game"
$env:HORIZON_LAUNCHER_NO_SELF_UPDATE = '1'
$Resources = "$S\launcher\versions\$BaseVersion\.local\horizon-client\26.3"
$Client = "$S\launcher\versions\$Version\horizon-client.exe"
$Ctl = "$S\launcher\versions\$Version\horizon-ctl.exe"
$GameDir = "$S\bench\labels-client"
$Scratch = "$S\bench\labels-player"
while (Test-Path 'C:\ai-bridge\heavy.lock') { Write-Host 'waiting: heavy.lock is held'; Start-Sleep -Seconds 30 }
$mods = (Get-Content -Raw "$S\launcher\versions\$Version\native-mods.json" | ConvertFrom-Json).mods | ForEach-Object { "$($_.id) $($_.version)" }
Write-Host "client: $Version ($($mods -join ', '))"
if (-not ($mods -match '^letscam ') -or -not ($mods -match '^soulscraft ')) { throw "$Version lacks letscam or soulscraft" }

# The owner's 87% CPU cap, as in bench-test.ps1.
. ([scriptblock]::Create(((Get-Content 'C:\Claude code projects\horizon-joint-dev\joint.ps1')[273..326] -join "`n")))
$CpuJob = [JointCpuCap]::Create(87)

function Invoke-Box([string]$Line) { & pwsh -NoProfile -File $Hlx -Sh $Line 2>&1 }
function Invoke-Console([string]$Line) {
    $out = Invoke-Box "$ServerExe --dir $ServerDir cmd '$Line'"
    Write-Host "console: $Line -> $(($out | Select-Object -Last 1))"
}
function Invoke-Bot([string]$Line, [int]$Timeout = 60) {
    $out = & $Ctl --socket $Socket --timeout $Timeout @($Line -split ' ') 2>&1
    if ($LASTEXITCODE -ne 0) { Write-Warning "lc_a: $Line -> $out" }
    $out
}
function Wait-Log([string]$Pattern, [int]$TimeoutSec) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path $Log) {
            $hit = Select-String -Path $Log -Pattern $Pattern | Select-Object -Last 1
            if ($hit) { return $hit.Matches[0].Value }
        }
        Start-Sleep -Seconds 2
    }
    throw "no '$Pattern' in the client log after $TimeoutSec s"
}

$running = Invoke-Box "pgrep -af 'horizon-server' | grep -v pgrep || true"
$others = @($running | Where-Object { $_ -match '/hz/servers/' -and $_ -notmatch [regex]::Escape($ServerDir) })
if ($others) { throw "another server runs in the box, not starting: $($others -join '; ')" }
Write-Host "server start: $((Invoke-Box "$ServerExe --dir $ServerDir start") | Select-Object -Last 1)"

New-Item -ItemType Directory -Force $GameDir, $Scratch, $OutputRoot, "$GameDir\config" | Out-Null
foreach ($dir in $GameDir, $Scratch) {
    $options = Join-Path $dir 'options.txt'
    $lines = @(if (Test-Path $options) { Get-Content $options | Where-Object { $_ -notmatch '^(soundCategory_master|tutorialStep):' } })
    Set-Content -Encoding utf8 $options ($lines + 'soundCategory_master:0.0' + 'tutorialStep:none')
}
$tomlRoot = $OutputRoot -replace '\\', '/'
Set-Content -Encoding utf8 "$GameDir\config\letscam-client.toml" "record_mic = false`nkeep_sources = true`noutput_dir = `"$tomlRoot`"`n"
$Socket = Join-Path $Scratch 'lc_a.sock'
$Log = [IO.Path]::ChangeExtension($Socket, 'log')
$started = Get-Date
$botPid = $null
try {
    Push-Location $GameDir
    [Environment]::CurrentDirectory = $GameDir
    $reply = & $Ctl --socket $Socket launch --game $Client --offscreen --resources $Resources --home $GameDir `
        --game-dir $GameDir --player-dir $Scratch --name lc_a --server "127.0.0.1:$ServerPort" --width 1920 --height 1080 2>&1
    Pop-Location
    if ($LASTEXITCODE -ne 0) { throw "launch failed: $reply" }
    $botPid = (($reply -join '') | ConvertFrom-Json).pid
    $p = Get-Process -Id $botPid
    [JointCpuCap]::Assign($CpuJob, $p.Handle)
    Invoke-Bot 'wait-ready 120' 130 | Out-Null

    Invoke-Console 'op lc_a'
    Invoke-Console 'gamemode survival lc_a'
    Invoke-Console 'time set 1000'
    Invoke-Console 'effect give lc_a resistance 600 4 true'
    Invoke-Console 'effect give lc_a regeneration 600 4 true'
    Invoke-Console 'effect give lc_a saturation 600 4 true'
    Start-Sleep -Seconds 2
    Invoke-Bot 'chat /sc debug on' | Out-Null
    Invoke-Bot 'chat /sc spawn revenant 1 1' | Out-Null
    Start-Sleep -Seconds 3
    $seen = (Invoke-Bot 'entities 24') -join '' | ConvertFrom-Json
    $list = if ($seen.PSObject.Properties['entities']) { $seen.entities } else { $seen }
    $mob = $list | Where-Object { "$($_.type) $($_.kind) $($_.name)" -match 'revenant' } | Select-Object -First 1
    Write-Host "revenant: $($mob | ConvertTo-Json -Compress -Depth 3)"

    Invoke-Bot 'chat /letscam start' | Out-Null
    Wait-Log 'Запись началась[^"]*' 20 | Write-Host
    foreach ($attack in $Attacks) {
        if ($mob) { Invoke-Bot "look-at entity $($mob.id)" | Out-Null }
        Invoke-Bot "chat /sc force $attack" | Out-Null
        Start-Sleep -Seconds 5
    }
    Invoke-Bot 'chat /letscam stop' | Out-Null
    Wait-Log 'Запись сохранена[^"]*' 30 | Write-Host
    Invoke-Bot 'chat /letscam render auto sheet' | Out-Null
    Wait-Log 'Ролик готов[^"]*|Рендер остановлен[^"]*|не удался[^"]*' $RenderTimeoutSec | Write-Host
} finally {
    if ($botPid) {
        & $Ctl --socket $Socket quit 2>&1 | Out-Null
        Start-Sleep -Seconds 5
        if (Get-Process -Id $botPid -ErrorAction SilentlyContinue) { Stop-Process -Id $botPid -Force }
    }
    Write-Host "server stop: $((Invoke-Box "$ServerExe --dir $ServerDir stop") | Select-Object -Last 1)"
}

$dir = Get-ChildItem $OutputRoot -Directory | Where-Object { $_.LastWriteTime -ge $started } | Sort-Object Name | Select-Object -Last 1
if (-not $dir) { throw "no recording in $OutputRoot" }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead((Join-Path $dir.FullName 'replay.mcpr'))
try {
    $entry = $zip.Entries | Where-Object { $_.Name -like '*.tmcpr' } | Select-Object -First 1
    $ms = [IO.MemoryStream]::new(); $s = $entry.Open(); $s.CopyTo($ms); $s.Dispose()
    $text = [Text.Encoding]::Latin1.GetString($ms.ToArray())
    $payloads = [regex]::Matches($text, 'soulscraft:state').Count
} finally { $zip.Dispose() }
$sheets = @(Get-ChildItem $dir.FullName -Filter '*_sheet_*.png')
Write-Host "folder: $($dir.FullName)"
Write-Host "soulscraft:state in the replay: $payloads"
Write-Host "sheets: $($sheets.Count)"
$errors = @(Select-String -Path $Log -Pattern 'ERROR|panicked|can''t read' | ForEach-Object { $_.Line })
Write-Host "client log errors: $($errors.Count)"; $errors | Select-Object -First 10 | ForEach-Object { Write-Host "  $_" }
if ($payloads -eq 0 -or $sheets.Count -eq 0) { exit 1 }
