# The PC's shared queue for measurements and tests (C:\ai-bridge\heavy.ps1, README «Очередь и
# самопроверки», the owner's rule since 2026-11-22): a bench or a timing run takes the heavy lock
# for at most 60 minutes, gives it back in a finally, and a series gives it back between runs. The
# lock names this script's process, so a run that dies frees it. Dot-source this file.
$HeavyScript = 'C:\ai-bridge\heavy.ps1'

# Waits for LetsCam's turn (up to $WaitMinutes) and takes the lock for $Minutes; throws when the
# turn did not come.
function Enter-HeavyQueue([string]$What, [int]$Minutes = 30, [int]$WaitMinutes = 120) {
    if (-not (Test-Path $HeavyScript)) { throw "$HeavyScript is missing: no way to wait for a quiet PC" }
    & pwsh -NoProfile -File $HeavyScript take -Who letscam -What $What -Minutes $Minutes -HolderPid $PID -WaitMinutes $WaitMinutes
    if ($LASTEXITCODE -ne 0) { throw "heavy.ps1 take: no turn for «$What» (exit $LASTEXITCODE)" }
}

function Exit-HeavyQueue {
    if (Test-Path $HeavyScript) { & pwsh -NoProfile -File $HeavyScript release -Who letscam | Out-Null }
}
