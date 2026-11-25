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
    if ($Path -and (Test-Path -LiteralPath $Path)) { Remove-Item -LiteralPath $Path -Force }
}

function New-Shortcut([string]$Path, [string]$Target) {
    if (-not $Path) { return }
    $shell = New-Object -ComObject WScript.Shell
    $link = $null
    try {
        $link = $shell.CreateShortcut($Path)
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
    Stop-Fences
    Invoke-Fences '--uninstall-startup'
    Remove-Shortcut $StartMenu
    Remove-Shortcut $DesktopLink
    if (Test-Path -LiteralPath $Dir) { Remove-Item -LiteralPath $Dir -Recurse -Force }
    Write-Host 'Fences was removed. User settings remain in %APPDATA%\AuraFences.' -ForegroundColor Green
    return
}

if (-not (Test-Path -LiteralPath $SourceExe -PathType Leaf)) {
    throw 'Fences.exe is missing. Run this script from the extracted standalone Fences package.'
}
$SameFolder = [IO.Path]::GetFullPath($Source).TrimEnd('\') -ieq [IO.Path]::GetFullPath($Dir).TrimEnd('\')
if (-not $SameFolder) { Stop-Fences }
[IO.Directory]::CreateDirectory($Dir) | Out-Null
if (-not $SameFolder) { Copy-Item -LiteralPath $SourceExe -Destination $Exe -Force }
foreach ($name in @('msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll')) {
    $from = Join-Path $Source $name
    if ((Test-Path -LiteralPath $from -PathType Leaf) -and -not $SameFolder) { Copy-Item -LiteralPath $from -Destination $Dir -Force }
}
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
