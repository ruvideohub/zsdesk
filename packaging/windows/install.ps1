<# Install the verified standalone AuraDock bundle for the current user. No profiles are deleted. #>
[CmdletBinding()]
param(
    [string]$Dir = (Join-Path $env:LOCALAPPDATA 'Programs\AuraDock'),
    [switch]$NoAutostart,
    [switch]$PanelOnly, # compatibility: only Dock is automatic in this version
    [switch]$DesktopShortcut,
    [switch]$NoStart,
    [switch]$Remove
)
$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }
$Dir = [IO.Path]::GetFullPath($Dir).TrimEnd('\')
$Source = [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\')
$Dock = Join-Path $Dir 'AuraDock.exe'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$RunName = 'AuraDock'
$Command = '"{0}"' -f $Dock
$Programs = [Environment]::GetFolderPath('Programs')
$Desktop = [Environment]::GetFolderPath('Desktop')
$Group = if ($Programs) { Join-Path $Programs 'AuraDock' } else { $null }

function Get-DockStartup {
    $key = Get-Item -LiteralPath $RunKey -ErrorAction SilentlyContinue
    if (-not $key -or $key.GetValueNames() -notcontains $RunName) { return $null }
    if ($key.GetValueKind($RunName) -ne [Microsoft.Win32.RegistryValueKind]::String) { throw 'Запись AuraDock имеет неизвестный тип и не изменена.' }
    return [string]$key.GetValue($RunName)
}
function Test-OwnStartup([string]$value) {
    return $null -ne $value -and $value.Trim().Equals($Command, [StringComparison]::OrdinalIgnoreCase)
}
function Remove-OwnShortcut([string]$path, [string]$target) {
    if (-not $path -or -not (Test-Path -LiteralPath $path -PathType Leaf)) { return }
    $shell = New-Object -ComObject WScript.Shell
    try {
        $link = $shell.CreateShortcut($path)
        try {
            if ([string]::Equals($link.TargetPath, $target, [StringComparison]::OrdinalIgnoreCase)) { Remove-Item -LiteralPath $path -Force }
        } finally { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($link) }
    } finally { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell) }
}
function New-OwnShortcut([string]$path, [string]$target, [string]$arguments) {
    if (-not $path) { return }
    $shell = New-Object -ComObject WScript.Shell
    try {
        $link = $shell.CreateShortcut($path)
        try {
            if ((Test-Path -LiteralPath $path) -and -not [string]::Equals($link.TargetPath, $target, [StringComparison]::OrdinalIgnoreCase)) {
                throw "Чужой ярлык не заменён: $path"
            }
            $link.TargetPath = $target
            $link.Arguments = $arguments
            $link.WorkingDirectory = $Dir
            $link.Save()
        } finally { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($link) }
    } finally { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell) }
}
function Assert-NotRunning {
    foreach ($name in @('AuraDock', 'aura-dock', 'widgets')) {
        foreach ($process in @(Get-Process -Name $name -ErrorAction SilentlyContinue)) {
            try { $path = $process.Path } catch { $path = $null }
            if ($path -and [IO.Path]::GetDirectoryName($path).Equals($Dir, [StringComparison]::OrdinalIgnoreCase)) {
                throw "Закройте $name из этой установки и повторите операцию; процессы принудительно не завершаются."
            }
        }
    }
}
function Assert-NoLegacyInstall {
    $legacyRun = (Get-ItemProperty -LiteralPath $RunKey -Name 'AuraExplorer' -ErrorAction SilentlyContinue).AuraExplorer
    if ((Test-Path -LiteralPath (Join-Path $Dir 'AuraExplorer.exe')) -or
        (Test-Path -LiteralPath (Join-Path $Dir 'auraservice.exe')) -or
        ($legacyRun -and $legacyRun -match '(?i)auraservice\.exe') -or
        (Test-Path -LiteralPath 'HKCU:\Software\Classes\Directory\shell\AuraExplorer') -or
        (Test-Path -LiteralPath 'HKCU:\Software\Classes\Drive\shell\AuraExplorer') -or
        (Test-Path -LiteralPath 'HKCU:\Software\Classes\Directory\Background\shell\AuraExplorer') -or
        (Test-Path -LiteralPath 'HKCU:\Software\Classes\CLSID\{20D04FE0-3AEA-1069-A2D8-08002B30309D}\shell\AuraExplorer')) {
        throw 'Найдена старая AuraExplorer/AuraService. Сначала удалите её прежним деинсталлятором, восстановив системный Проводник, затем повторите установку. Профили не удаляйте.'
    }
}
$shortcuts = @()
if ($Group) {
    $shortcuts += @{Path=(Join-Path $Group 'AuraDock.lnk'); Target=$Dock; Args=''}
    $shortcuts += @{Path=(Join-Path $Group 'AuraDock Settings.lnk'); Target=$Dock; Args='--settings'}
}
if ($Desktop) { $desktopLink = Join-Path $Desktop 'AuraDock.lnk' } else { $desktopLink = $null }

if ($Remove) {
    Assert-NoLegacyInstall
    Assert-NotRunning
    $installedManifest = Join-Path $Dir 'manifest.json'
    if (-not (Test-Path -LiteralPath $installedManifest -PathType Leaf)) { throw 'Нет манифеста установленного AuraUI; файлы и ярлыки не удалены.' }
    $receipt = Get-Content -LiteralPath $installedManifest -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($receipt.product -ne 'AuraDock' -or $null -eq $receipt.files) { throw 'Манифест установленного AuraUI неверен; файлы и ярлыки не удалены.' }
    $current = Get-DockStartup
    if (Test-OwnStartup $current) { Remove-ItemProperty -LiteralPath $RunKey -Name $RunName }
    foreach ($shortcut in $shortcuts) { Remove-OwnShortcut $shortcut.Path $shortcut.Target }
    Remove-OwnShortcut $desktopLink $Dock
    # Only delete files exactly matching this installation receipt. Keep modified files,
    # scripts, licenses, the manifest and all unknown/user-owned files.
    foreach ($name in @('AuraDock.exe','widgets.exe')) {
        $path = Join-Path $Dir $name
        $record = $receipt.files.PSObject.Properties[$name]
        if ($record -and (Test-Path -LiteralPath $path -PathType Leaf)) {
            if ((Get-Item -LiteralPath $path).Length -eq [int64]$record.Value.bytes -and
                (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -eq [string]$record.Value.sha256) {
                Remove-Item -LiteralPath $path -Force
            } else { Write-Warning "Изменённый файл сохранён: $path" }
        }
    }
    Write-Host 'Удаление завершено; пользовательские настройки и прочие файлы сохранены.' -ForegroundColor Green
    return
}

Assert-NoLegacyInstall
Assert-NotRunning
# A separate directory keeps old combined uninstall logs and Fences untouched.
$legacyDir = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs\AuraUI')).TrimEnd('\')
if ($Dir.Equals($legacyDir, [StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath (Join-Path $Dir 'Fences.exe'))) {
    throw 'Выберите отдельную папку AuraDock; старая совместная установка и Fences не изменены.'
}
$current = Get-DockStartup
if ($null -ne $current -and -not (Test-OwnStartup $current)) { throw 'Запись AuraDock принадлежит другой установке и не изменена.' }
$required = @('AuraDock.exe','widgets.exe')
$manifestPath = Join-Path $Source 'manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw 'Нет manifest.json: установка неполного набора запрещена.' }
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($manifest.product -ne 'AuraDock' -or $null -eq $manifest.files) { throw 'Неверный манифест набора.' }
$records = @($manifest.files.PSObject.Properties)
foreach ($name in $required) {
    if ($records.Name -notcontains $name) { throw "Нет $name в манифесте." }
}
foreach ($entry in $records) {
    $relative = [string]$entry.Name
    if ($relative -match '(?i)\.exe$' -and $relative -notin $required) { throw "Посторонний исполняемый файл в манифесте: $relative" }
    if ([IO.Path]::IsPathRooted($relative) -or $relative -match '(^|[\\/])\.\.([\\/]|$)' -or $relative -match ':') { throw "Недопустимый путь в манифесте: $relative" }
    $path = [IO.Path]::GetFullPath((Join-Path $Source $relative))
    if (-not $path.StartsWith($Source + '\', [StringComparison]::OrdinalIgnoreCase) -or -not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Файл отсутствует: $relative" }
    if ((Get-Item -LiteralPath $path).Length -ne [int64]$entry.Value.bytes -or
        (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne [string]$entry.Value.sha256) { throw "Файл повреждён: $relative" }
}
New-Item -ItemType Directory -Force -Path $Dir | Out-Null
if (-not $Source.Equals($Dir, [StringComparison]::OrdinalIgnoreCase)) {
    foreach ($entry in $records) {
        $from = Join-Path $Source $entry.Name
        $to = Join-Path $Dir $entry.Name
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $to) | Out-Null
        Copy-Item -LiteralPath $from -Destination $to -Force
    }
    Copy-Item -LiteralPath $manifestPath -Destination $Dir -Force
}
# Recheck ownership after copying; never overwrite a competing installation's entry.
if ((Get-DockStartup) -ne $current) { throw 'Запись автозапуска изменилась; установка остановлена.' }
if ($NoAutostart) {
    if (Test-OwnStartup $current) { Remove-ItemProperty -LiteralPath $RunKey -Name $RunName }
} else {
    New-Item -Path $RunKey -Force | Out-Null
    New-ItemProperty -LiteralPath $RunKey -Name $RunName -PropertyType String -Value $Command -Force | Out-Null
}
if ($Group) {
    New-Item -ItemType Directory -Force -Path $Group | Out-Null
    foreach ($shortcut in $shortcuts) { New-OwnShortcut $shortcut.Path $shortcut.Target $shortcut.Args }
}
if ($DesktopShortcut -and $desktopLink) { New-OwnShortcut $desktopLink $Dock '' }
if (-not $NoStart) { Start-Process -FilePath $Dock -WorkingDirectory $Dir }
Write-Host 'AuraDock установлен; Fences устанавливается и запускается отдельно.' -ForegroundColor Green
