param(
    [Parameter(Mandatory = $true)][string]$Workspace,
    [Parameter(Mandatory = $true)][string]$ArtifactsRoot,
    [Parameter(Mandatory = $true)][string]$ReportsRoot,
    [Parameter(Mandatory = $true)][string]$AdminToolRoot,
    [Parameter(Mandatory = $true)][string]$UserInstallerRoot,
    [Parameter(Mandatory = $true)][string]$PreflightReport,
    [Parameter(Mandatory = $true)][string]$ModeComparisonReport,
    [Parameter(Mandatory = $true)][string]$LargeInstallerReport,
    [Parameter(Mandatory = $true)][string]$TestEvidenceReport,
    [Parameter(Mandatory = $true)][string]$Timestamp
)

$ErrorActionPreference = 'Stop'
$sourceName = "stage11-source-release-$Timestamp"
$partial = Join-Path $ArtifactsRoot "$sourceName.partial"
$final = Join-Path $ArtifactsRoot $sourceName
if ((Test-Path -LiteralPath $partial) -or (Test-Path -LiteralPath $final)) {
    throw 'Stage 11 source release path already exists'
}

New-Item -ItemType Directory -Path $partial | Out-Null
$sourceRoot = Join-Path $partial 'source'
& robocopy $Workspace $sourceRoot /E /COPY:DAT /DCOPY:DAT /R:1 /W:1 /XJ /NP /XD target target-native node_modules dist .git /XF .DS_Store '._*' windows-schema.json 'formal-master-*' | Out-Null
if ($LASTEXITCODE -gt 7) {
    throw "Source copy failed with Robocopy exit code $LASTEXITCODE"
}

$forbiddenFiles = @(Get-ChildItem -LiteralPath $sourceRoot -File -Recurse -Force | Where-Object {
    $_.Name -match '^(\.env($|\.)|production-signing\.(key|pub)$)' -or
    $_.Extension -in @('.pfx', '.p12', '.pem', '.key')
})
if ($forbiddenFiles.Count -ne 0) {
    throw 'Source release contains forbidden credential files'
}

$sourceFiles = @(Get-ChildItem -LiteralPath $sourceRoot -File -Recurse -Force | Sort-Object FullName)
$sourcePrefixLength = $sourceRoot.TrimEnd([IO.Path]::DirectorySeparatorChar).Length + 1
$sourceEntries = foreach ($file in $sourceFiles) {
    [ordered]@{
        path = $file.FullName.Substring($sourcePrefixLength).Replace([IO.Path]::DirectorySeparatorChar, '/')
        bytes = [uint64]$file.Length
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLowerInvariant()
    }
}
$sourceManifestPath = Join-Path $partial 'source-manifest.json'
[ordered]@{
    schemaVersion = 1
    generatedAt = (Get-Date).ToUniversalTime().ToString('o')
    fileCount = $sourceFiles.Count
    files = @($sourceEntries)
} | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 $sourceManifestPath

$adminExe = @(Get-ChildItem -LiteralPath $AdminToolRoot -Filter '*.exe' -File)
$template = @(Get-ChildItem -LiteralPath (Join-Path $AdminToolRoot 'templates') -Filter '*.exe' -File)
if ($adminExe.Count -ne 1 -or $template.Count -ne 1) {
    throw 'Admin candidate must contain exactly one admin EXE and one template EXE'
}
$adminExe = $adminExe[0].FullName
$template = $template[0].FullName
$publicKey = Join-Path $AdminToolRoot 'admin-data\production-signing.pub'
$userExe = @(Get-ChildItem -LiteralPath $UserInstallerRoot -Filter '*.exe' -File)
if ($userExe.Count -ne 1) {
    throw 'User installer candidate must contain exactly one EXE'
}
$adminExeSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $adminExe).Hash.ToLowerInvariant()
$templateSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $template).Hash.ToLowerInvariant()
$userInstallerSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $userExe[0].FullName).Hash.ToLowerInvariant()

$publicKeyBytes = [IO.File]::ReadAllBytes($publicKey)
if ($publicKeyBytes.Length -ne 32) {
    throw 'Production public key must be 32 bytes'
}
$publicKeyHex = [BitConverter]::ToString($publicKeyBytes).Replace('-', '').ToLowerInvariant()
$templateText = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($template)).ToLowerInvariant()
if (-not $templateText.Contains($publicKeyHex)) {
    throw 'Installer template does not embed the trusted production public key'
}
if (-not $templateText.Contains('yunshu_native_installer_v1')) {
    throw 'Installer template is not the native Win32 template'
}
$templateValidation = Get-Content -Raw -Encoding UTF8 (Join-Path $AdminToolRoot 'reports\native-template-validation.json') | ConvertFrom-Json
if ($templateValidation.status -ne 'passed') {
    throw 'Native installer template validation did not pass'
}
$adminBuild = Get-Content -Raw -Encoding UTF8 (Join-Path $AdminToolRoot 'reports\stage11-admin-build.json') | ConvertFrom-Json
if ($adminBuild.status -ne 'verified-admin-candidate' -or
    $adminBuild.installerTemplateSha256 -ne $templateSha256 -or
    $adminBuild.adminExeSha256 -ne $adminExeSha256) {
    throw 'Admin build report is not bound to the current administrator EXE and native template'
}

$preflight = Get-Content -Raw -Encoding UTF8 (Join-Path $PreflightReport 'master-validation-summary.json') | ConvertFrom-Json
$modeComparison = Get-Content -Raw -Encoding UTF8 $ModeComparisonReport | ConvertFrom-Json
$largeInstaller = Get-Content -Raw -Encoding UTF8 $LargeInstallerReport | ConvertFrom-Json
$userBuild = Get-Content -Raw -Encoding UTF8 (Join-Path $UserInstallerRoot 'reports\stage11-user-installer-build.json') | ConvertFrom-Json
if ($preflight.status -ne 'passed') {
    throw 'Master preflight report did not pass'
}
if ($modeComparison.status -ne 'passed' -or $modeComparison.manifestFilesIdentical -ne $true) {
    throw 'Direct and installer write comparison did not pass'
}
if ($largeInstaller.status -ne 'passed' -or
    $largeInstaller.largerThan4GiB -ne $true -or
    $largeInstaller.copiedInstallerVerified -ne $true) {
    throw 'Large installer verification did not pass'
}
if ($userBuild.status -ne 'verified-installer-candidate' -or
    $userBuild.installerSha256 -ne $userInstallerSha256 -or
    [int]$userBuild.manifestSchemaVersion -ne 2 -or
    [int]$userBuild.footerSchemaVersion -ne 1 -or
    $userBuild.strictEd25519Verified -ne $true) {
    throw 'User installer build report is not bound to the current verified installer'
}
$testEvidence = Get-Content -Raw -Encoding UTF8 $TestEvidenceReport | ConvertFrom-Json
if ([int]$testEvidence.schemaVersion -ne 1 -or
    $testEvidence.status -ne 'passed' -or
    $testEvidence.artifacts.installerTemplateSha256 -ne $templateSha256 -or
    $testEvidence.artifacts.adminExeSha256 -ne $adminExeSha256 -or
    $testEvidence.artifacts.userInstallerSha256 -ne $userInstallerSha256) {
    throw 'Test evidence is not passed or is not bound to the current Stage 11 artifacts'
}

function Get-PassedTestCount {
    param([object]$Checks, [string]$Name)
    $property = $Checks.PSObject.Properties[$Name]
    if ($null -eq $property -or $property.Value.status -ne 'passed' -or [int]$property.Value.passedCount -lt 1) {
        throw "Required test evidence is missing or failed: $Name"
    }
    return [int]$property.Value.passedCount
}

function Assert-PassedCheck {
    param([object]$Checks, [string]$Name)
    $property = $Checks.PSObject.Properties[$Name]
    if ($null -eq $property -or $property.Value.status -ne 'passed') {
        throw "Required verification evidence is missing or failed: $Name"
    }
}

$rustCoreTestsPassed = Get-PassedTestCount $testEvidence.checks 'rustCoreTests'
$rustAdminTestsPassed = Get-PassedTestCount $testEvidence.checks 'rustAdminTests'
$adminFrontendTestsPassed = Get-PassedTestCount $testEvidence.checks 'adminFrontendTests'
$nativeInstallerUnitTestsPassed = Get-PassedTestCount $testEvidence.checks 'nativeInstallerUnitTests'
@('windowsCargoCheck', 'windowsReleaseBuild', 'adminLaunchSmoke', 'userInstallerLaunchSmoke') | ForEach-Object {
    Assert-PassedCheck $testEvidence.checks $_
}

Rename-Item -LiteralPath $partial -NewName $sourceName
$sourceManifestFinal = Join-Path $final 'source-manifest.json'
$finalReportPath = Join-Path $ReportsRoot "stage11-final-$Timestamp.json"
[ordered]@{
    schemaVersion = 1
    status = 'stage11-candidate-ready-for-stage12'
    generatedAt = (Get-Date).ToUniversalTime().ToString('o')
    master = [ordered]@{
        buildId = $preflight.buildId
        fileCount = $preflight.fileCount
        totalBytes = $preflight.expandedBytes
        factoryFiles = $preflight.factoryFileCount
        rootMappings = $preflight.rootMappingCount
    }
    artifacts = [ordered]@{
        adminToolRoot = $AdminToolRoot
        adminExeSha256 = $adminExeSha256
        installerTemplateSha256 = $templateSha256
        userInstallerRoot = $UserInstallerRoot
        userInstallerFileName = $userExe[0].Name
        userInstallerBytes = [uint64]$userExe[0].Length
        userInstallerSha256 = $userInstallerSha256
        sourceReleaseRoot = $final
        sourceFileCount = $sourceFiles.Count
        sourceManifestSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $sourceManifestFinal).Hash.ToLowerInvariant()
    }
    verification = [ordered]@{
        evidenceReport = $TestEvidenceReport
        evidenceReportSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $TestEvidenceReport).Hash.ToLowerInvariant()
        rustCoreTestsPassed = $rustCoreTestsPassed
        rustAdminTestsPassed = $rustAdminTestsPassed
        adminFrontendTestsPassed = $adminFrontendTestsPassed
        nativeInstallerUnitTestsPassed = $nativeInstallerUnitTestsPassed
        nativeTemplateValidationPassed = $true
        windowsCargoCheckPassed = ($testEvidence.checks.windowsCargoCheck.status -eq 'passed')
        windowsReleaseBuildPassed = ($testEvidence.checks.windowsReleaseBuild.status -eq 'passed')
        adminLaunchSmokePassed = ($testEvidence.checks.adminLaunchSmoke.status -eq 'passed')
        userInstallerLaunchSmokePassed = ($testEvidence.checks.userInstallerLaunchSmoke.status -eq 'passed')
        trustedPublicKeyEmbedded = $true
        webView2Required = $false
        powershellRequired = $false
        dynamicVcRuntimeRequired = $false
        strictEd25519Verified = [bool]$userBuild.strictEd25519Verified
        largeInstallerOver4GiBVerified = [bool]$largeInstaller.largerThan4GiB
        largeInstallerCopiedAndVerified = [bool]$largeInstaller.copiedInstallerVerified
        directAndInstallerWritesIdentical = [bool]$modeComparison.manifestFilesIdentical
        comparedFileCount = $modeComparison.fileCount
    }
    boundaries = [ordered]@{
        physicalUsbWritePerformed = $false
        protectedUsbVolumesModified = $false
        workstationLauncherOrRuntimeModified = $false
        authenticodeSigned = $false
        stage12PhysicalUsbAcceptanceRequired = $true
        win11X64AcceptanceRequiredBeforeSalesRelease = $true
        salesRelease = $false
    }
} | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 $finalReportPath

Write-Output 'STATUS=passed'
Write-Output "SOURCE_RELEASE=$final"
Write-Output "FINAL_REPORT=$finalReportPath"
Write-Output "SOURCE_FILES=$($sourceFiles.Count)"
