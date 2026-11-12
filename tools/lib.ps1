# Shared helpers of LetsCam's bench scenarios (bench-test.ps1, capsule-scene.ps1). Dot-source it.
# The bot is lc_a, an offline client on the joint server (horizon-joint-dev/README.md); the world
# changes go through the server's console, so the bot needs no operator rights.
$Ctl = "$env:LOCALAPPDATA\Horizon\current\horizon-ctl.exe"
$ServerTool = "$env:LOCALAPPDATA\Horizon\current\horizon-server.exe"
$JointServer = "$env:LOCALAPPDATA\Horizon\joint-dev\server"
$Instance = "$env:APPDATA\horizon\instances\joint-dev"
$Ffprobe = 'ffprobe'
# Output on the Dev Drive: Defender scans every file written on C: (RustCraft's request).
$OutputRoot = 'F:\letscam\joint-dev'
$ConfigFile = Join-Path $Instance 'config\letscam-client.toml'

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

# Starts lc_a offscreen at the given window size and waits until it is in the world.
function Start-LcBot([string]$Scratch, [int]$Width = 1920, [int]$Height = 1080) {
    New-Item -ItemType Directory -Force $Scratch | Out-Null
    $socket = Join-Path $Scratch 'lc_a.sock'
    $reply = & $Ctl --socket $socket launch --game (Get-JointClient) --offscreen --home $Instance `
        --game-dir $Instance --player-dir $Scratch --name lc_a --server 127.0.0.1:25570 `
        --width $Width --height $Height 2>&1
    if ($LASTEXITCODE -ne 0) { throw "launch failed: $reply" }
    $bot = [pscustomobject]@{ Name = 'lc_a'; Socket = $socket }
    Invoke-Bot $bot 'wait-ready 120' 130 | Out-Null
    $bot
}

function Stop-LcBot($Bot) {
    & $Ctl --socket $Bot.Socket quit 2>&1 | Out-Null
}

# The tests' config: no microphone (never the owner's), output on F:. Returns the old file's text.
function Set-TestConfig {
    New-Item -ItemType Directory -Force (Split-Path $ConfigFile) | Out-Null
    $backup = if (Test-Path $ConfigFile) { Get-Content -Raw $ConfigFile } else { $null }
    $tomlRoot = $OutputRoot -replace '\\', '/'
    Set-Content -Encoding utf8 $ConfigFile "record_mic = false`nkeep_sources = true`noutput_dir = `"$tomlRoot`"`n"
    $backup
}

function Restore-TestConfig($Backup) {
    if ($null -ne $Backup) { Set-Content -Encoding utf8 -NoNewline $ConfigFile $Backup }
    else { Remove-Item -LiteralPath $ConfigFile -Force -ErrorAction SilentlyContinue }
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

# Waits for $Count «Ролик готов» lines in the game's log (one per angle), or the first failure.
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

# The newest recording folder written since $Since.
function Get-NewRecording([datetime]$Since) {
    Get-ChildItem $OutputRoot -Directory | Where-Object { $_.LastWriteTime -ge $Since } |
        Sort-Object Name | Select-Object -Last 1
}
