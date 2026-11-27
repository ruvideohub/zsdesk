<#
.SYNOPSIS
Installs or removes the standalone Fences app for the current Windows user.
.PARAMETER Dir
Install folder. Defaults to %LOCALAPPDATA%\Programs\Fences.
.PARAMETER Autostart
Explicitly enable Fences at sign-in. By default the existing setting is not changed.
.PARAMETER NoAutostart
Explicitly disable this install's Fences sign-in entry.
.PARAMETER DesktopShortcut
Create a desktop shortcut in addition to the Start menu shortcut.
.PARAMETER NoStart
Do not start Fences after installation.
.PARAMETER Remove
Stop Fences, remove its owned sign-in entry and shortcuts, then remove the app folder.
#>
[CmdletBinding()]
param(
    [string]$Dir = (Join-Path $env:LOCALAPPDATA 'Programs\Fences'),
    [switch]$NoAutostart,
    [switch]$Autostart,
    [switch]$DesktopShortcut,
    [switch]$NoStart,
    [switch]$Remove
)
$ErrorActionPreference = 'Stop'
if ($Autostart -and $NoAutostart) { throw 'Choose only one of -Autostart and -NoAutostart.' }
$Source = $PSScriptRoot
if (-not $Source) { $Source = (Get-Location).Path }
if (-not $Dir) { throw 'No install directory was supplied and LOCALAPPDATA is empty.' }
$Exe = Join-Path $Dir 'Fences.exe'
$SourceExe = Join-Path $Source 'Fences.exe'
# Never install over or remove the Dock, even when a custom directory is supplied.
if (Test-Path -LiteralPath (Join-Path $Dir 'AuraDock.exe') -PathType Leaf) {
    throw 'Choose a separate Fences directory; AuraDock will not be modified.'
}
$Receipt = Join-Path $Dir 'fences-manifest.json'

$Programs = [Environment]::GetFolderPath('Programs')
$DesktopDir = [Environment]::GetFolderPath('Desktop')
$StartMenu = if ($Programs) { Join-Path $Programs 'Fences.lnk' } else { $null }
$DesktopLink = if ($DesktopDir) { Join-Path $DesktopDir 'Fences.lnk' } else { $null }

function Get-InstalledFencesProcesses {
    $expected = [IO.Path]::GetFullPath($Exe)
    foreach ($process in @(Get-Process -Name 'Fences' -ErrorAction SilentlyContinue)) {
        try { $actual = $process.Path } catch { throw "Cannot verify the executable path for running Fences process $($process.Id)." }
        if (-not $actual) { throw "Cannot verify the executable path for running Fences process $($process.Id)." }
        if ([string]::Equals([IO.Path]::GetFullPath($actual), $expected, [StringComparison]::OrdinalIgnoreCase)) { $process }
    }
}

function Stop-Fences {
    if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { return }
    try { Start-Process -FilePath $Exe -ArgumentList '--quit' -Wait -ErrorAction Stop | Out-Null } catch { }
    for ($i = 0; $i -lt 40; $i++) {
        if (@(Get-InstalledFencesProcesses).Count -eq 0) { return }
        Start-Sleep -Milliseconds 250
    }
    if (@(Get-InstalledFencesProcesses).Count -gt 0) {
        throw "Fences is still running from $Exe. Exit it and retry."
    }
}

function Invoke-Fences([string]$Argument) {
    if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { return }
    $process = Start-Process -FilePath $Exe -ArgumentList $Argument -Wait -PassThru -ErrorAction Stop
    if ($process.ExitCode -ne 0) { throw "Fences $Argument failed with exit code $($process.ExitCode)." }
}

function Remove-Shortcut([string]$Path) {
    if (-not $Path -or -not (Test-Path -LiteralPath $Path)) { return }
    $shell = New-Object -ComObject WScript.Shell
    try {
        $link = $shell.CreateShortcut($Path)
        try {
            if ([string]::Equals($link.TargetPath, $Exe, [StringComparison]::OrdinalIgnoreCase)) { Remove-Item -LiteralPath $Path -Force }
        } finally { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($link) }
    } finally { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell) }
}

function New-Shortcut([string]$Path, [string]$Target) {
    if (-not $Path) { return }
    $shell = New-Object -ComObject WScript.Shell
    $link = $null
    try {
        $link = $shell.CreateShortcut($Path)
        if ((Test-Path -LiteralPath $Path) -and -not [string]::Equals($link.TargetPath, $Target, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Another application's shortcut was not replaced: $Path"
        }
        $link.TargetPath = $Target
        $link.WorkingDirectory = Split-Path -Parent $Target
        $link.IconLocation = "$Target,0"
        $link.Description = 'Standalone desktop and folder grids'
        $link.Save()
    } finally {
        if ($link) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($link) }
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell)
    }
}

if ($Remove) {
    # No recursive directory deletion: only remove files recorded by a Fences receipt,
    # and only if their hashes still match. Old installations without a receipt stay intact.
    $receiptPath = $Receipt
    if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) { $receiptPath = Join-Path $Dir 'manifest.json' }
    if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) { throw 'No Fences receipt; files and startup were not changed.' }
    $receipt = Get-Content -LiteralPath $receiptPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($receipt.product -ne 'Fences' -or -not $receipt.standalone -or $null -eq $receipt.files) { throw 'Invalid Fences receipt; files and startup were not changed.' }
    $exeRecord = $receipt.files.PSObject.Properties['Fences.exe']
    if ((Test-Path -LiteralPath $Exe -PathType Leaf) -and
        (-not $exeRecord -or (Get-Item -LiteralPath $Exe).Length -ne [int64]$exeRecord.Value.bytes -or
        (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash -ne [string]$exeRecord.Value.sha256)) {
        throw 'Fences executable differs from its receipt; startup and files were not changed.'
    }
    Stop-Fences
    Invoke-Fences '--uninstall-startup'
    Remove-Shortcut $StartMenu
    Remove-Shortcut $DesktopLink
    foreach ($name in @('Fences.exe','msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll')) {
        $path = Join-Path $Dir $name
        $record = $receipt.files.PSObject.Properties[$name]
        if ($record -and (Test-Path -LiteralPath $path -PathType Leaf)) {
            if ((Get-Item -LiteralPath $path).Length -eq [int64]$record.Value.bytes -and
                (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -eq [string]$record.Value.sha256) {
                Remove-Item -LiteralPath $path -Force
            } else { Write-Warning "Modified file preserved: $path" }
        }
    }
    Write-Host 'Fences was removed; profiles and other files were preserved.' -ForegroundColor Green
    return
}

if (-not (Test-Path -LiteralPath $SourceExe -PathType Leaf)) {
    throw 'Fences.exe is missing. Run this script from the extracted standalone Fences package.'
}
$sourceManifest = Join-Path $Source 'manifest.json'
if (-not (Test-Path -LiteralPath $sourceManifest -PathType Leaf)) { throw 'Missing Fences package manifest.' }
$manifest = Get-Content -LiteralPath $sourceManifest -Raw -Encoding UTF8 | ConvertFrom-Json
if ($manifest.product -ne 'Fences' -or -not $manifest.standalone -or $null -eq $manifest.files) { throw 'Invalid Fences package manifest.' }
foreach ($name in @('Fences.exe','msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll')) {
    $file = Join-Path $Source $name
    $entry = $manifest.files.PSObject.Properties[$name]
    if (-not $entry -or -not (Test-Path -LiteralPath $file -PathType Leaf) -or
        (Get-Item -LiteralPath $file).Length -ne [int64]$entry.Value.bytes -or
        (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne [string]$entry.Value.sha256) { throw "Fences package file missing or modified: $name" }
}
$SameFolder = [IO.Path]::GetFullPath($Source).TrimEnd('\') -ieq [IO.Path]::GetFullPath($Dir).TrimEnd('\')
if (-not $SameFolder) { Stop-Fences }
[IO.Directory]::CreateDirectory($Dir) | Out-Null
if (-not $SameFolder) { Copy-Item -LiteralPath $SourceExe -Destination $Exe -Force }
foreach ($name in @('msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll')) {
    $from = Join-Path $Source $name
    if ((Test-Path -LiteralPath $from -PathType Leaf) -and -not $SameFolder) { Copy-Item -LiteralPath $from -Destination $Dir -Force }
}
Copy-Item -LiteralPath $sourceManifest -Destination $Receipt -Force
try { Get-ChildItem -LiteralPath $Dir -Filter '*.exe' | Unblock-File -ErrorAction SilentlyContinue } catch { }
New-Shortcut $StartMenu $Exe
if ($DesktopShortcut) { New-Shortcut $DesktopLink $Exe }

if ($NoAutostart) { Invoke-Fences '--disable-startup' }
elseif ($Autostart) { Invoke-Fences '--enable-startup' }
if (-not $NoStart) { Start-Process -FilePath $Exe -WorkingDirectory $Dir }
Write-Host "Fences is installed in $Dir." -ForegroundColor Green
if ($NoAutostart) { Write-Host 'Sign-in startup is disabled.' }
elseif ($Autostart) { Write-Host 'Fences will start when you sign in to Windows.' }
else { Write-Host 'Fences sign-in startup was not changed.' }
