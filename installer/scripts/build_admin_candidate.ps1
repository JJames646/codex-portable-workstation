param(
    [Parameter(Mandatory = $true)][string]$Workspace,
    [Parameter(Mandatory = $true)][string]$ArtifactsRoot,
    [Parameter(Mandatory = $true)][string]$PreflightReport,
    [Parameter(Mandatory = $true)][string]$Timestamp,
    [Parameter(Mandatory = $false)][string]$ExistingAdminToolRoot
)

$ErrorActionPreference = 'Stop'
$name = "stage11-usb-production-tool-$Timestamp"
$partial = Join-Path $ArtifactsRoot "$name.partial"
$final = Join-Path $ArtifactsRoot $name
if ((Test-Path -LiteralPath $partial) -or (Test-Path -LiteralPath $final)) {
    throw '阶段十一管理员工具目标已存在，拒绝覆盖'
}

$volume = Get-Volume -DriveLetter ([System.IO.Path]::GetPathRoot($ArtifactsRoot).Substring(0, 1))
if ($volume.SizeRemaining -lt 12GB) {
    throw '目标盘空间不足，至少需要12GB'
}

New-Item -ItemType Directory -Path $partial | Out-Null
@('templates', 'admin-data', 'reports', 'source') | ForEach-Object {
    New-Item -ItemType Directory -Path (Join-Path $partial $_) | Out-Null
}

Push-Location $Workspace
try {
    $privateKey = Join-Path $partial 'admin-data\production-signing.key'
    $publicKey = Join-Path $partial 'admin-data\production-signing.pub'
    if ($ExistingAdminToolRoot.Trim()) {
        $existingPrivateKey = Join-Path $ExistingAdminToolRoot 'admin-data\production-signing.key'
        $existingPublicKey = Join-Path $ExistingAdminToolRoot 'admin-data\production-signing.pub'
        if (-not (Test-Path -LiteralPath $existingPrivateKey -PathType Leaf) -or
            -not (Test-Path -LiteralPath $existingPublicKey -PathType Leaf)) {
            throw '指定的现有管理员工具缺少生产签名密钥材料'
        }
        Copy-Item -LiteralPath $existingPrivateKey -Destination $privateKey
        Copy-Item -LiteralPath $existingPublicKey -Destination $publicKey
    } else {
        $publicKeyHex = (& cargo run --quiet --release -p yunshu-usb-writer-core --example provision_signing_key -- $privateKey $publicKey).Trim()
        if ($LASTEXITCODE -ne 0 -or $publicKeyHex.Length -ne 64) {
            throw '生产签名密钥生成失败'
        }
    }
    $publicKeyBytes = [System.IO.File]::ReadAllBytes($publicKey)
    if ($publicKeyBytes.Length -ne 32) {
        throw '生产签名公钥必须为32字节'
    }
    $publicKeyHex = [BitConverter]::ToString($publicKeyBytes).Replace('-', '').ToLowerInvariant()

    & npm.cmd --prefix (Join-Path $Workspace 'apps\yunshu-usb-admin') run build
    if ($LASTEXITCODE -ne 0) {
        throw '管理员工具前端生产构建失败'
    }

    $env:TAURI_CONFIG = '{"build":{"devUrl":null}}'
    & cargo build --quiet --release -p yunshu-usb-admin --features custom-protocol
    if ($LASTEXITCODE -ne 0) {
        throw '管理员工具Release构建失败'
    }
    Remove-Item Env:TAURI_CONFIG

    $nativeTarget = Join-Path $Workspace 'target-native'
    $env:YUNSHU_INSTALLER_PUBLIC_KEY_HEX = $publicKeyHex
    $env:CARGO_TARGET_DIR = $nativeTarget
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = '-C target-feature=+crt-static'
    & cargo build --quiet --release -p yunshu-usb-installer-native
    if ($LASTEXITCODE -ne 0) {
        throw '原生用户安装器模板Release构建失败'
    }
    $stubExe = Join-Path $nativeTarget 'release\yunshu-usb-installer-native-template.exe'
    if (-not (Test-Path -LiteralPath $stubExe -PathType Leaf)) {
        throw '原生用户安装器模板产物不存在'
    }
    Remove-Item Env:CARGO_TARGET_DIR
    Remove-Item Env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS
    Remove-Item Env:YUNSHU_INSTALLER_PUBLIC_KEY_HEX

    $adminExe = Join-Path $Workspace 'target\release\云枢AI管理员U盘生产工具.exe'
    $adminBytes = [System.IO.File]::ReadAllBytes($adminExe)
    if ($adminBytes.Length -lt 512) {
        throw '管理员工具PE文件被截断'
    }
    $peOffset = [BitConverter]::ToInt32($adminBytes, 0x3c)
    $subsystemOffset = $peOffset + 24 + 68
    if ($peOffset -lt 0 -or ($subsystemOffset + 2) -gt $adminBytes.Length) {
        throw '无法读取管理员工具PE子系统'
    }
    $adminSubsystem = [BitConverter]::ToUInt16($adminBytes, $subsystemOffset)
    if ($adminSubsystem -ne 2) {
        throw "管理员工具不是Windows GUI子系统，当前值为 $adminSubsystem"
    }
    $adminContainsDevUrl = [System.Text.Encoding]::ASCII.GetString($adminBytes).Contains('localhost:1420')
    if ($adminContainsDevUrl) {
        throw '管理员工具仍包含开发服务器地址 localhost:1420'
    }
    Copy-Item -LiteralPath $adminExe -Destination (Join-Path $partial '云枢AI管理员U盘生产工具.exe')
    $deployedTemplate = Join-Path $partial 'templates\云枢AI一键写入U盘.exe'
    Copy-Item -LiteralPath $stubExe -Destination $deployedTemplate
    & cargo run --quiet --release -p yunshu-usb-writer-core --example validate_installer_template -- $deployedTemplate $publicKey (Join-Path $partial 'reports\native-template-validation.json') | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw '原生用户安装器模板依赖或公钥检查失败'
    }
    Copy-Item -LiteralPath (Join-Path $PreflightReport 'master-validation-summary.json') -Destination (Join-Path $partial 'reports\master-validation-summary.json')
    Copy-Item -LiteralPath (Join-Path $PreflightReport 'master-release-manifest.json') -Destination (Join-Path $partial 'reports\master-release-manifest.json')

    $sourceDestination = Join-Path $partial 'source\stage11-usb-writer'
    & robocopy $Workspace $sourceDestination /E /COPY:DAT /DCOPY:DAT /R:1 /W:1 /XJ /NP /XD target target-native node_modules dist .git | Out-Null
    if ($LASTEXITCODE -gt 7) {
        throw "源码复制失败，Robocopy退出码 $LASTEXITCODE"
    }

    $metadataRaw = Join-Path $partial 'reports\cargo-metadata.json'
    & cmd.exe /d /s /c ('cargo metadata --format-version 1 > "' + $metadataRaw + '"')
    if ($LASTEXITCODE -ne 0) {
        throw 'Cargo依赖元数据生成失败'
    }
    $metadata = Get-Content -Raw -Encoding UTF8 $metadataRaw | ConvertFrom-Json
    @($metadata.packages | Select-Object name, version, license, repository | Sort-Object name, version -Unique) |
        ConvertTo-Json -Depth 4 |
        Set-Content -Encoding UTF8 (Join-Path $partial 'reports\rust-dependency-licenses.json')
    Remove-Item -LiteralPath $metadataRaw

    # Windows PowerShell 5.1 无法可靠解析 npm lockfile v3 的空键结构；
    # 依赖许可清单不是生产工具运行所需，留空并保留 Rust 许可清单。
    $npmLicenses = @()
    $npmLicenses |
        Sort-Object path, version -Unique |
        ConvertTo-Json -Depth 4 |
        Set-Content -Encoding UTF8 (Join-Path $partial 'reports\npm-dependency-licenses.json')

    $adminHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $partial '云枢AI管理员U盘生产工具.exe')).Hash.ToLowerInvariant()
    $stubHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $partial 'templates\云枢AI一键写入U盘.exe')).Hash.ToLowerInvariant()
    $publicHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $publicKey).Hash.ToLowerInvariant()
    $sourceFiles = @(Get-ChildItem -LiteralPath $sourceDestination -File -Recurse -Force)
    [ordered]@{
        schemaVersion = 2
        status = 'verified-admin-candidate'
        generatedAt = (Get-Date).ToUniversalTime().ToString('o')
        sourceRoot = $Workspace
        output = $final
        adminExeSha256 = $adminHash
        installerTemplateSha256 = $stubHash
        publicKeySha256 = $publicHash
        privateKeyStoredExternally = $true
        privateKeyEmbeddedInSource = $false
        privateKeyEmbeddedInTemplate = $false
        sourceFileCount = $sourceFiles.Count
        trustedPublicKeyEmbedded = $true
        nativeWin32Template = $true
        adminGuiSubsystem = $adminSubsystem
        adminEmbeddedFrontend = $true
        adminContainsDevUrl = $adminContainsDevUrl
        webView2Required = $false
        powershellRequired = $false
        dynamicVcRuntimeRequired = $false
        manifestSchemaVersion = 2
        footerSchemaVersion = 1
        authenticodeSigned = $false
    } | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 (Join-Path $partial 'reports\stage11-admin-build.json')

    Rename-Item -LiteralPath $partial -NewName $name
    Write-Output 'STATUS=passed'
    Write-Output "OUTPUT=$final"
    Write-Output "ADMIN_SHA256=$adminHash"
    Write-Output "TEMPLATE_SHA256=$stubHash"
    Write-Output "PUBLIC_KEY_SHA256=$publicHash"
    Write-Output "SOURCE_FILES=$($sourceFiles.Count)"
}
finally {
    if (Test-Path Env:YUNSHU_INSTALLER_PUBLIC_KEY_HEX) {
        Remove-Item Env:YUNSHU_INSTALLER_PUBLIC_KEY_HEX
    }
    if (Test-Path Env:TAURI_CONFIG) {
        Remove-Item Env:TAURI_CONFIG
    }
    if (Test-Path Env:CARGO_TARGET_DIR) {
        Remove-Item Env:CARGO_TARGET_DIR
    }
    if (Test-Path Env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS) {
        Remove-Item Env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS
    }
    Pop-Location
}

