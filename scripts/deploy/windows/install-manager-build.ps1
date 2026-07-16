[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourceExe,

    [Parameter(Mandatory = $true)]
    [string]$DestinationExe,

    [string]$BackupRoot
)

$ErrorActionPreference = "Stop"

$source = (Resolve-Path -LiteralPath $SourceExe).Path
$destination = [System.IO.Path]::GetFullPath($DestinationExe)
$destinationDirectory = Split-Path -Parent $destination

if ([System.IO.Path]::GetExtension($source) -ne ".exe") {
    throw "SourceExe must be a Windows EXE: $source"
}

if (-not (Test-Path -LiteralPath $destinationDirectory)) {
    New-Item -ItemType Directory -Force -Path $destinationDirectory | Out-Null
}

if (-not $BackupRoot) {
    $targetDirectory = Split-Path -Parent $destinationDirectory
    $projectRoot = Split-Path -Parent $targetDirectory
    $BackupRoot = Join-Path $projectRoot "Backups\manager-builds"
}

$destinationProcesses = Get-CimInstance Win32_Process -Filter "Name LIKE 'codex-plus-plus-manager%.exe'" |
    Where-Object {
        $_.ExecutablePath -and
        [System.IO.Path]::GetFullPath($_.ExecutablePath) -eq $destination
    }

if ($destinationProcesses) {
    $processIds = ($destinationProcesses.ProcessId -join ", ")
    throw "The destination manager is still running (PID: $processIds). Exit it before deployment."
}

$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$backupDirectory = Join-Path $BackupRoot $timestamp
$temporaryDestination = "$destination.new"

if ($PSCmdlet.ShouldProcess($destination, "Install Codex Portable Workstation manager")) {
    try {
        if (Test-Path -LiteralPath $destination) {
            New-Item -ItemType Directory -Force -Path $backupDirectory | Out-Null
            $disabledBackup = Join-Path $backupDirectory "codex-plus-plus-manager.exe.disabled"
            Copy-Item -LiteralPath $destination -Destination $disabledBackup -Force

            $metadata = [ordered]@{
                source = $destination
                backup = $disabledBackup
                createdAt = (Get-Date).ToString("o")
                sha256 = (Get-FileHash -LiteralPath $disabledBackup -Algorithm SHA256).Hash
            }
            $metadata |
                ConvertTo-Json |
                Set-Content -LiteralPath (Join-Path $backupDirectory "metadata.json") -Encoding UTF8
        }

        Copy-Item -LiteralPath $source -Destination $temporaryDestination -Force
        $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
        $temporaryHash = (Get-FileHash -LiteralPath $temporaryDestination -Algorithm SHA256).Hash
        if ($sourceHash -ne $temporaryHash) {
            throw "The temporary manager failed SHA-256 verification."
        }

        if (Test-Path -LiteralPath $destination) {
            Remove-Item -LiteralPath $destination -Force
        }
        Move-Item -LiteralPath $temporaryDestination -Destination $destination -Force

        $installedHash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
        if ($sourceHash -ne $installedHash) {
            throw "The installed manager failed SHA-256 verification."
        }

        Write-Output "MANAGER_INSTALLED=$destination"
        Write-Output "SHA256=$installedHash"
        if (Test-Path -LiteralPath $backupDirectory) {
            Write-Output "BACKUP=$backupDirectory"
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryDestination) {
            Remove-Item -LiteralPath $temporaryDestination -Force
        }
    }
}
