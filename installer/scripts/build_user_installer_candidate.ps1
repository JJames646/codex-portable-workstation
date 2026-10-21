param(
    [Parameter(Mandatory = $true)][string]$Workspace,
    [Parameter(Mandatory = $true)][string]$AdminToolRoot,
    [Parameter(Mandatory = $true)][string]$MasterRoot,
    [Parameter(Mandatory = $true)][string]$ArtifactsRoot,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Timestamp
)

$ErrorActionPreference = 'Stop'
$name = "stage11-user-installer-$Timestamp"
$partial = Join-Path $ArtifactsRoot "$name.partial"
$final = Join-Path $ArtifactsRoot $name
if ((Test-Path -LiteralPath $partial) -or (Test-Path -LiteralPath $final)) {
    throw '阶段十一用户安装包目标已存在，拒绝覆盖'
}

$volume = Get-Volume -DriveLetter ([System.IO.Path]::GetPathRoot($ArtifactsRoot).Substring(0, 1))
if ($volume.SizeRemaining -lt 16GB) {
    throw '目标盘空间不足，至少需要16GB'
}

New-Item -ItemType Directory -Path $partial | Out-Null
New-Item -ItemType Directory -Path (Join-Path $partial 'reports') | Out-Null
$output = Join-Path $partial "云枢AI便携工作站一键安装包-$Version.exe"
$privateKey = Join-Path $AdminToolRoot 'admin-data\production-signing.key'
$publicKey = Join-Path $AdminToolRoot 'admin-data\production-signing.pub'
$template = Join-Path $AdminToolRoot 'templates\云枢AI一键写入U盘.exe'

Push-Location $Workspace
try {
    & cargo run --quiet --release -p yunshu-usb-writer-core --example validate_installer_template -- $template $publicKey (Join-Path $partial 'reports\native-template-validation.json') | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw '原生用户安装器模板检查失败'
    }
    & cargo run --quiet --release -p yunshu-usb-writer-core --example build_installer -- $MasterRoot $template $output $privateKey $Version | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw '用户单EXE生成失败'
    }

    $verificationReport = Join-Path $partial 'reports\independent-installer-verification.json'
    & cargo run --quiet --release -p yunshu-usb-writer-core --example verify_installer -- $output $publicKey $verificationReport | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw '用户单EXE独立验签失败'
    }

    $file = Get-Item -LiteralPath $output
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $output).Hash.ToLowerInvariant()
    [ordered]@{
        schemaVersion = 2
        status = 'verified-installer-candidate'
        generatedAt = (Get-Date).ToUniversalTime().ToString('o')
        productVersion = $Version
        masterRoot = $MasterRoot
        installerFileName = $file.Name
        installerBytes = [uint64]$file.Length
        installerSha256 = $hash
        largerThan4GiB = $file.Length -gt 4GB
        strictEd25519Verified = $true
        nativeWin32Installer = $true
        manifestSchemaVersion = 2
        footerSchemaVersion = 1
        payloadArchitecture = 'x64'
        webView2Required = $false
        powershellRequired = $false
        dynamicVcRuntimeRequired = $false
        trustedPublicKeySha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $publicKey).Hash.ToLowerInvariant()
        authenticodeSigned = $false
    } | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 (Join-Path $partial 'reports\stage11-user-installer-build.json')

    Copy-Item -LiteralPath $publicKey -Destination (Join-Path $partial 'reports\production-signing.pub')
    Rename-Item -LiteralPath $partial -NewName $name
    Write-Output 'STATUS=passed'
    Write-Output "OUTPUT=$final"
    Write-Output "INSTALLER=$($file.Name)"
    Write-Output "BYTES=$($file.Length)"
    Write-Output "LARGER_THAN_4GIB=$($file.Length -gt 4GB)"
    Write-Output "SHA256=$hash"
}
finally {
    Pop-Location
}
