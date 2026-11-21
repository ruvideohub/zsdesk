<#
.SYNOPSIS
Builds and packages the standalone Fences Windows x64 app (ZIP and optional installer).
The package contains Fences.exe and runtime/support files only; it never builds or copies
AuraExplorer, AuraDock, AuraService, or another Aura application.
.EXAMPLE
./scripts/package-fences.ps1 -RuntimeDir 'C:\Build\VC\Microsoft.VC143.CRT' `
  -InnoCompiler 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe'
.EXAMPLE
./scripts/package-fences.ps1 -SkipBuild -BinaryPath C:\Build\Fences.exe -RuntimeDir C:\Build\VC\CRT
#>
param(
    [string]$WorkDir = (Join-Path ([IO.Path]::GetTempPath()) 'AuraFences-build'),
    [string]$TargetDir = $env:CARGO_TARGET_DIR,
    [string]$OutputDir,
    [string]$RuntimeDir,
    [string]$InnoCompiler,
    [switch]$SkipBuild,
    [switch]$ParentLockHeld,
    [string]$BinaryPath
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$WorkDir = [IO.Path]::GetFullPath($WorkDir)
if (-not $TargetDir) { $TargetDir = Join-Path $WorkDir 'target' }
$TargetDir = [IO.Path]::GetFullPath($TargetDir)
$version = [regex]::Match((Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if ($version -notmatch '^\d+\.\d+\.\d+([+-][A-Za-z0-9.-]+)?$') { throw 'Workspace version is invalid' }
if (-not $OutputDir) { $OutputDir = Join-Path $root "releases\$version" }
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
$target = 'x86_64-pc-windows-msvc'
if ($SkipBuild -and -not $BinaryPath) { throw '-SkipBuild requires -BinaryPath' }
if ($InnoCompiler -and -not (Test-Path -LiteralPath $InnoCompiler -PathType Leaf)) { throw "Inno Setup compiler not found: $InnoCompiler" }

if (-not $RuntimeDir) {
    $candidates = @()
    if ($env:VCToolsRedistDir) { $candidates += Join-Path $env:VCToolsRedistDir 'x64' }
    $vswhere = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path -LiteralPath $vswhere) {
        $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($installation) {
            $redist = Join-Path $installation 'VC\Redist\MSVC'
            if (Test-Path -LiteralPath $redist) {
                $candidates += @(Get-ChildItem -LiteralPath $redist -Directory | Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } |
                    Sort-Object { [version]$_.Name } -Descending | ForEach-Object { Join-Path $_.FullName 'x64' })
            }
        }
    }
    foreach ($candidate in $candidates) {
        if (-not (Test-Path -LiteralPath $candidate)) { continue }
        $crt = Get-ChildItem -LiteralPath $candidate -Directory -Filter 'Microsoft.VC*.CRT' |
            Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'msvcp140.dll') } | Select-Object -First 1
        if ($crt) { $RuntimeDir = $crt.FullName; break }
    }
}
if (-not $RuntimeDir) { throw 'Visual C++ x64 Redistributable files are required; supply -RuntimeDir' }
$RuntimeDir = [IO.Path]::GetFullPath($RuntimeDir)
$runtimeFiles = @('msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll')
foreach ($name in $runtimeFiles) {
    if (-not (Test-Path -LiteralPath (Join-Path $RuntimeDir $name) -PathType Leaf)) { throw "Redistributable file missing: $name" }
}

$packageLock = $null
if (-not $ParentLockHeld) {
    [IO.Directory]::CreateDirectory($TargetDir) | Out-Null
    $packageLock = [IO.File]::Open((Join-Path $TargetDir 'aura-package.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
}
try {
foreach ($path in @($WorkDir,$TargetDir,$OutputDir)) { [IO.Directory]::CreateDirectory($path) | Out-Null }
$env:CARGO_TARGET_DIR = $TargetDir
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_RELEASE_DEBUG = '0'
$zsui = Join-Path (Split-Path -Parent $root) 'zsui'
$env:CARGO_ENCODED_RUSTFLAGS = @("--remap-path-prefix=$root=/src/aura", "--remap-path-prefix=$zsui=/src/zsui",
    "--remap-path-prefix=$env:USERPROFILE=/user", "--remap-path-prefix=$TargetDir=/target", '-C', 'link-arg=/PDBALTPATH:fences.pdb') -join [char]31

if (-not $SkipBuild) {
    & cargo build --locked --release --target $target --package aura-fences --bin Fences --manifest-path (Join-Path $root 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'Cargo failed while building the standalone Fences package' }
}
if (-not $BinaryPath) { $BinaryPath = Join-Path $TargetDir "$target\release\Fences.exe" }
$BinaryPath = [IO.Path]::GetFullPath($BinaryPath)
if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) { throw "Fences binary not found: $BinaryPath" }
$resource = (Get-Item -LiteralPath $BinaryPath).VersionInfo
if ($resource.ProductVersion -ne $version -or $resource.FileDescription -ne 'Fences' -or $resource.OriginalFilename -ne 'Fences.exe') {
    throw 'Fences executable resource identity/version differs from the standalone product'
}

$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = $BinaryPath
$start.Arguments = '--build-info'
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$process = [Diagnostics.Process]::Start($start)
try {
    $output = $process.StandardOutput.ReadToEndAsync()
    $errorOutput = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(10000)) { $process.Kill(); $process.WaitForExit(); throw 'Fences build identity query timed out' }
    if ($process.ExitCode -ne 0) { throw "Fences build identity query failed: $($errorOutput.GetAwaiter().GetResult())" }
    $identity = $output.GetAwaiter().GetResult() | ConvertFrom-Json
} finally { $process.Dispose() }
if ($identity.product -ne 'Fences' -or $identity.version -ne $version -or -not $identity.standalone -or $identity.platform -ne 'windows-x64') {
    throw 'Fences --build-info identity/version mismatch'
}

$name = "Fences-$version-x64"
$stage = Join-Path $WorkDir $name
$zip = Join-Path $WorkDir "$name.zip"
$outputZip = Join-Path $OutputDir "$name.zip"
$installer = Join-Path $WorkDir "$name-setup.exe"
$outputInstaller = Join-Path $OutputDir "$name-setup.exe"
if ((Test-Path -LiteralPath $stage) -or (Test-Path -LiteralPath $zip) -or
    (Test-Path -LiteralPath $installer) -or (Test-Path -LiteralPath $outputZip) -or (Test-Path -LiteralPath $outputInstaller)) {
    throw "Existing Fences output retained; inspect before retrying: $name"
}
[IO.Directory]::CreateDirectory($stage) | Out-Null
Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $stage 'Fences.exe')
foreach ($dll in $runtimeFiles) { Copy-Item -LiteralPath (Join-Path $RuntimeDir $dll) -Destination (Join-Path $stage $dll) }
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $stage 'LICENSE.txt')
Copy-Item -LiteralPath (Join-Path $root 'packaging\fences\THIRD-PARTY-NOTICES.txt') -Destination (Join-Path $stage 'THIRD-PARTY-NOTICES.txt')
$supplementalLicenses=Join-Path $root 'packaging\licenses'
if (-not (Test-Path -LiteralPath $supplementalLicenses -PathType Container)) { throw 'Supplemental third-party license directory is missing' }
$licenseStage=Join-Path $stage 'licenses'
[IO.Directory]::CreateDirectory($licenseStage) | Out-Null
foreach ($license in @('MPL-2.0.txt','MinHook-LICENSE.txt','ExplorerColumnView-MIT.txt')) {
    Copy-Item -LiteralPath (Join-Path $supplementalLicenses $license) -Destination $licenseStage
}
Copy-Item -LiteralPath (Join-Path $root 'packaging\windows\install-fences.ps1') -Destination (Join-Path $stage 'install-fences.ps1')
Copy-Item -LiteralPath (Join-Path $root 'packaging\windows\install-fences.cmd') -Destination (Join-Path $stage 'install-fences.cmd')
@'
Fences — standalone desktop and folder grids

Run Fences.exe to start the desktop and folder grids. Fences keeps its layout and
preferences in %APPDATA%\AuraFences and can migrate an existing layout from an
older AuraExplorer profile. Files remain in their original locations.

Fences does not require or start AuraExplorer, AuraDock, or AuraService. Choosing
an Open action hands the selected path to the Windows system Explorer.

Install for the current user with install-fences.cmd, or use Fences-*-setup.exe.
Use portable.flag beside Fences.exe to keep data in a local data folder.
'@ | Set-Content -LiteralPath (Join-Path $stage 'README.txt') -Encoding utf8
@'
Fences — независимое приложение сеток рабочего стола и папок

Запустите Fences.exe. Раскладка и настройки хранятся в %APPDATA%\AuraFences;
при первом запуске можно перенести существующую раскладку из профиля старого AuraExplorer.
Файлы и папки остаются на своих местах.

Fences не требует и не запускает AuraExplorer, AuraDock или AuraService. Команда
«Открыть» передаёт выбранный путь системному Проводнику Windows.

Для установки текущему пользователю запустите install-fences.cmd либо Fences-*-setup.exe.
Файл portable.flag рядом с Fences.exe включает профиль data в папке приложения.
'@ | Set-Content -LiteralPath (Join-Path $stage '说明.txt') -Encoding utf8

$files = [ordered]@{}
foreach ($file in Get-ChildItem -LiteralPath $stage -File -Recurse | Sort-Object FullName) {
    $relative = $file.FullName.Substring($stage.Length + 1).Replace('\','/')
    $files[$relative] = [ordered]@{ bytes=$file.Length; sha256=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}
$manifest = [ordered]@{ product='Fences'; version=$version; architecture=$target; standalone=$true; files=$files }
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $stage 'manifest.json') -Encoding utf8

if ($InnoCompiler) {
    $arguments = @('/Qp', "/DSourceDir=$stage", "/DOutputDir=$WorkDir", "/DMyAppVersion=$version", (Join-Path $root 'packaging\windows\fences.iss'))
    & $InnoCompiler @arguments 1>(Join-Path $WorkDir 'installer.fences.current.log')
    if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed while building the Fences installer' }
    if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) { throw 'Inno Setup did not produce the expected Fences installer' }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip, [IO.Compression.CompressionLevel]::Optimal, $false)
$archive = [IO.Compression.ZipFile]::OpenRead($zip)
try {
    $entries = @($archive.Entries | ForEach-Object { $_.FullName })
    if ($entries.Count -ne $files.Count + 1) { throw 'Fences ZIP file count mismatch' }
    foreach ($forbidden in @('AuraExplorer.exe','AuraDock.exe','AuraService.exe','auraservice.exe','widgets.exe')) {
        if ($entries -contains $forbidden) { throw "Standalone Fences ZIP contains forbidden application: $forbidden" }
    }
    if (-not ($entries -contains 'Fences.exe') -or -not ($entries -contains 'manifest.json')) { throw 'Fences executable or manifest is missing from ZIP' }
    foreach ($entry in $archive.Entries) {
        if ($entry.FullName -eq 'manifest.json') { continue }
        if (-not $files.Contains($entry.FullName)) { throw "Unexpected file in Fences ZIP: $($entry.FullName)" }
        $expected = $files[$entry.FullName]
        if ($entry.Length -ne $expected.bytes) { throw "Fences ZIP length mismatch: $($entry.FullName)" }
        $stream = $entry.Open()
        $sha = [Security.Cryptography.SHA256]::Create()
        try { $hash = ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-','').ToLowerInvariant() }
        finally { $stream.Dispose(); $sha.Dispose() }
        if ($hash -ne $expected.sha256) { throw "Fences ZIP hash mismatch: $($entry.FullName)" }
    }
} finally { $archive.Dispose() }
Move-Item -LiteralPath $zip -Destination $outputZip
if ($InnoCompiler) { Move-Item -LiteralPath $installer -Destination $outputInstaller }
Write-Host "Created standalone Fences ZIP: $outputZip"
if ($InnoCompiler) { Write-Host "Created standalone Fences installer: $outputInstaller" }
} finally { if ($packageLock) { $packageLock.Dispose() } }
