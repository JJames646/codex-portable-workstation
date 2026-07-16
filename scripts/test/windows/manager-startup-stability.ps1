[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ExePath,

    [ValidateRange(1, 100)]
    [int]$Iterations = 20,

    [ValidateRange(100, 30000)]
    [int]$ExitAfterReadyMs = 900,

    [ValidateRange(5, 60)]
    [int]$TimeoutSeconds = 20,

    [ValidateRange(0, 20)]
    [int]$ImmediateRelaunchIterations = 3
)

$ErrorActionPreference = "Stop"

function Get-ManagerGuardPort {
    if ($env:CODEX_PLUS_GUARD_PORT) {
        return [int]$env:CODEX_PLUS_GUARD_PORT
    }
    if ($env:CODEX_PLUS_MANAGER_GUARD_PORT) {
        return [int]$env:CODEX_PLUS_MANAGER_GUARD_PORT
    }
    if ($env:CODEX_PLUS_GUARD_PORT_OFFSET) {
        return 57319 + [int]$env:CODEX_PLUS_GUARD_PORT_OFFSET
    }

    $offset = 0
    $normalizedUsername = $env:USERNAME.ToLowerInvariant()
    foreach ($character in [System.Text.Encoding]::ASCII.GetBytes($normalizedUsername)) {
        $offset = ($offset + $character) % 65536
    }
    return 57319 + ($offset % 1000)
}

function Test-ManagerGuardPortReleased {
    param([int]$Port)

    $listener = Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction SilentlyContinue
    return -not $listener
}

function Wait-TestManagerExit {
    param(
        [Parameter(Mandatory = $true)]
        [System.Diagnostics.Process]$Process,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedPath,

        [Parameter(Mandatory = $true)]
        [int]$TimeoutSeconds,

        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    $exited = $Process.WaitForExit($TimeoutSeconds * 1000)
    if (-not $exited) {
        $actual = Get-Process -Id $Process.Id -ErrorAction SilentlyContinue
        if ($actual -and $actual.Path -eq $ExpectedPath) {
            Stop-Process -Id $Process.Id -Force
        }
        throw "${Context}: manager did not exit before the timeout."
    }

    if ($Process.ExitCode -ne 0) {
        throw "${Context}: unexpected exit code $($Process.ExitCode)."
    }
}

$exe = (Resolve-Path -LiteralPath $ExePath).Path
$guardPort = Get-ManagerGuardPort
$previousExitDelay = $env:CODEX_PLUS_MANAGER_EXIT_AFTER_READY_MS
$env:CODEX_PLUS_MANAGER_EXIT_AFTER_READY_MS = [string]$ExitAfterReadyMs

try {
    $existing = Get-CimInstance Win32_Process -Filter "Name LIKE 'codex-plus-plus-manager%.exe'"
    if ($existing) {
        $details = $existing |
            Select-Object ProcessId, Name, ExecutablePath |
            Format-Table -AutoSize |
            Out-String
        throw "Manager processes exist before the stability test:`n$details"
    }

    for ($iteration = 1; $iteration -le $Iterations; $iteration++) {
        if (-not (Test-ManagerGuardPortReleased -Port $guardPort)) {
            throw "Iteration ${iteration}: guard port $guardPort was not released before launch."
        }

        $process = Start-Process -FilePath $exe -PassThru
        Wait-TestManagerExit `
            -Process $process `
            -ExpectedPath $exe `
            -TimeoutSeconds $TimeoutSeconds `
            -Context "Iteration $iteration"

        Start-Sleep -Milliseconds 200
        if (-not (Test-ManagerGuardPortReleased -Port $guardPort)) {
            throw "Iteration ${iteration}: guard port $guardPort was not released after exit."
        }

        Write-Output "ITERATION=$iteration STATUS=OK PID=$($process.Id)"
    }

    for ($iteration = 1; $iteration -le $ImmediateRelaunchIterations; $iteration++) {
        if (-not (Test-ManagerGuardPortReleased -Port $guardPort)) {
            throw "Immediate relaunch ${iteration}: guard port $guardPort was not released before launch."
        }

        $closingProcess = Start-Process -FilePath $exe -PassThru
        Start-Sleep -Milliseconds ($ExitAfterReadyMs + 100)

        $relaunchTimer = [System.Diagnostics.Stopwatch]::StartNew()
        $relaunchedProcess = Start-Process -FilePath $exe -PassThru

        Wait-TestManagerExit `
            -Process $closingProcess `
            -ExpectedPath $exe `
            -TimeoutSeconds $TimeoutSeconds `
            -Context "Immediate relaunch $iteration closing process"
        Wait-TestManagerExit `
            -Process $relaunchedProcess `
            -ExpectedPath $exe `
            -TimeoutSeconds $TimeoutSeconds `
            -Context "Immediate relaunch $iteration new process"
        $relaunchTimer.Stop()

        if ($relaunchTimer.ElapsedMilliseconds -lt $ExitAfterReadyMs) {
            throw "Immediate relaunch ${iteration}: the new process exited before becoming the active manager."
        }

        Start-Sleep -Milliseconds 200
        if (-not (Test-ManagerGuardPortReleased -Port $guardPort)) {
            throw "Immediate relaunch ${iteration}: guard port $guardPort was not released."
        }

        Write-Output "IMMEDIATE_RELAUNCH=$iteration STATUS=OK OLD_PID=$($closingProcess.Id) NEW_PID=$($relaunchedProcess.Id)"
    }

    Write-Output "STARTUP_STABILITY_TEST=PASSED ITERATIONS=$Iterations PORT=$guardPort"
}
finally {
    if ($null -eq $previousExitDelay) {
        Remove-Item Env:CODEX_PLUS_MANAGER_EXIT_AFTER_READY_MS -ErrorAction SilentlyContinue
    }
    else {
        $env:CODEX_PLUS_MANAGER_EXIT_AFTER_READY_MS = $previousExitDelay
    }
}
