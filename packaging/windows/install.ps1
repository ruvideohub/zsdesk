<#
.SYNOPSIS
    Устанавливает AuraExplorer с панелью и включает автозапуск.

.DESCRIPTION
    Копирует программы AuraUI в папку пользователя, кладёт ярлыки в меню «Пуск» и
    прописывает автозапуск при входе в Windows. Каждый компонент (панель, сетки рабочего
    стола, виджеты, файловый менеджер) — отдельная программа, и запускает их одна служба:
    `auraservice.exe --run`. Автозапуск поэтому один — служба; что именно она поднимает,
    решается в её окне «Запуск и компоненты» (`auraservice.exe --settings`), и там же
    переключатели каждого компонента. Так у всех запусков один хозяин и нет двух панелей,
    которые дрались бы за место на экране.

    Администратор не нужен: всё ставится в профиль пользователя (PrivilegesRequired)
    и в HKCU.

.PARAMETER Dir
    Куда ставить. По умолчанию %LOCALAPPDATA%\Programs\AuraExplorer.

.PARAMETER NoAutostart
    Не прописывать автозапуск (и убрать прежний).

.PARAMETER PanelOnly
    Оставлен для совместимости: раньше включал автозапуск только панели. Теперь набор
    компонентов задаётся в окне службы, и скрипт просто пишет обычный автозапуск службы.

.PARAMETER DesktopShortcut
    Положить ярлык ещё и на рабочий стол.

.PARAMETER NoStart
    Не запускать AuraExplorer после установки.

.PARAMETER Remove
    Удалить: остановить процессы, убрать автозапуск, ярлыки и папку.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File install.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File install.ps1 -Remove
#>
[CmdletBinding()]
param(
    [string]$Dir = (Join-Path $env:LOCALAPPDATA 'Programs\AuraExplorer'),
    [switch]$NoAutostart,
    [switch]$PanelOnly,
    [switch]$DesktopShortcut,
    [switch]$NoStart,
    [switch]$Remove
)

$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

$RunKey    = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
# Папки оболочки могут быть пустыми в урезанной среде — тогда ярлыки просто не делаем.
$Programs   = [Environment]::GetFolderPath('Programs')
$DesktopDir = [Environment]::GetFolderPath('Desktop')
$StartMenu  = if ($Programs) { Join-Path $Programs 'AuraExplorer.lnk' } else { $null }
$ShellLink  = if ($Programs) { Join-Path $Programs 'AuraUI - запуск и компоненты.lnk' } else { $null }
# Так ярлык назывался до службы; при обновлении его надо убрать, иначе их станет два.
$OldShellLink = if ($Programs) { Join-Path $Programs 'AuraExplorer - сетки и панель.lnk' } else { $null }
$FencesLink = if ($Programs) { Join-Path $Programs 'Fences.lnk' } else { $null }
$Desktop    = if ($DesktopDir) { Join-Path $DesktopDir 'AuraExplorer.lnk' } else { $null }
$Source    = $PSScriptRoot
if (-not $Source) { $Source = (Get-Location).Path }
if (-not $Dir) { throw 'Не задана папка установки и пуст %LOCALAPPDATA%: запусти с -Dir "C:\AuraExplorer".' }
$Exe       = Join-Path $Dir 'AuraExplorer.exe'
$Dock      = Join-Path $Dir 'AuraDock.exe'
$Service   = Join-Path $Dir 'auraservice.exe'
$Fences    = Join-Path $Dir 'Fences.exe'

function Write-Head([string]$Text) { Write-Host ''; Write-Host $Text -ForegroundColor Cyan }
function Write-Note([string]$Text) { Write-Host "  $Text" -ForegroundColor DarkGray }
function Write-Ok([string]$Text) { Write-Host "  $Text" -ForegroundColor Green }

function Remove-Integration {
    # `--uninstall-integration` is the program's own way back: it restores the recorded
    # values and leaves alone anything the user changed afterwards. A missing or
    # already-removed installation is not an error — there is simply nothing to undo.
    $explorer = Join-Path $Dir 'AuraExplorer.exe'
    if (-not (Test-Path -LiteralPath $explorer -PathType Leaf)) {
        # The folder may already be gone but the registry entries are not: our own
        # uninstaller cannot run, so say so rather than pretend the job is done.
        Write-Note 'AuraExplorer.exe не найден: следы в реестре не убраны, проверь HKCU\Software\Classes\Directory\shell\AuraExplorer'
        return
    }
    try {
        Start-Process -FilePath $explorer -ArgumentList '--uninstall-integration' -Wait -ErrorAction Stop | Out-Null
        Write-Note 'перехват открытия папок возвращён Проводнику'
    } catch {
        Write-Note "не удалось вернуть открытие папок: $($_.Exception.Message)"
    }
}

function Stop-AuraExplorer {
    # Сначала останавливаем владельца запуска, иначе он немедленно поднимет Fences снова.
    Get-Process -Name 'auraservice' -ErrorAction SilentlyContinue | ForEach-Object {
        try { $_.Kill(); [void]$_.WaitForExit(2000) } catch { }
    }
    # Fences — отдельная программа со своей shutdown-командой.
    if (Test-Path -LiteralPath $Fences) {
        try { Start-Process -FilePath $Fences -ArgumentList '--quit' -Wait -ErrorAction Stop | Out-Null } catch { }
    }
    # AuraExplorer предоставляет ограниченное кооперативное закрытие для обновления.
    if (Test-Path -LiteralPath $Exe) {
        try { Start-Process -FilePath $Exe -ArgumentList @('--shutdown-for-update', ('"{0}"' -f $Exe)) -Wait -ErrorAction Stop | Out-Null } catch { }
    }
    Get-Process -Name 'AuraExplorer', 'aura-dock', 'Fences', 'widgets' -ErrorAction SilentlyContinue | ForEach-Object {
        try { $_.Kill(); [void]$_.WaitForExit(2000) } catch { }
    }
}

function Set-Autostart([string]$Name, [string]$Value) {
    New-ItemProperty -Path $RunKey -Name $Name -Value $Value -PropertyType String -Force | Out-Null
}

function Remove-Autostart([string]$Name) {
    $existing = Get-ItemProperty -Path $RunKey -Name $Name -ErrorAction SilentlyContinue
    if ($existing) { Remove-ItemProperty -Path $RunKey -Name $Name -Force; Write-Note "автозапуск «$Name» выключен" }
}

# Прежние записи автозапуска компонентов: старый AuraExplorer desktop-mode, отдельный
# Fences и AuraDock. Их работу теперь делает AuraService, поэтому записи удаляются.
# а оставленная запись подняла бы второй экземпляр — или вообще старую программу.
function Remove-StaleAutostart {
    $key = Get-Item -LiteralPath $RunKey -ErrorAction SilentlyContinue
    if (-not $key) { return }
    $fencesPrefix = '"' + [IO.Path]::GetFullPath($Fences) + '" '
    foreach ($name in @($key.GetValueNames())) {
        if ($name -eq 'AuraExplorer') { continue }
        $value = [string]$key.GetValue($name)
        $oldExplorerDesktop = ($value -match 'AuraExplorer(\.exe)?["\s]') -and ($value -match '--desktop')
        # Standalone Fences uses a profile-scoped Run value. Retire it only when it points
        # to this exact suite copy; a different standalone Fences installation survives.
        $thisFencesInstall = $value.StartsWith($fencesPrefix, [StringComparison]::OrdinalIgnoreCase)
        if ($oldExplorerDesktop -or $thisFencesInstall) {
            Remove-ItemProperty -Path $RunKey -Name $name -Force -ErrorAction SilentlyContinue
            Write-Note "снята прежняя запись автозапуска «$name»"
        }
    }
    Remove-Autostart 'AuraDock'
}

function Remove-Shortcut([string]$Path) {
    if (-not $Path) { return }
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force; Write-Note "удалён ярлык $Path" }
}

function New-Shortcut([string]$Path, [string]$Target, [string]$Arguments, [string]$Description) {
    if (-not $Path) { return }
    $link = $null
    $shell = New-Object -ComObject WScript.Shell
    try {
        $link = $shell.CreateShortcut($Path)
        $link.TargetPath = $Target
        if ($Arguments) { $link.Arguments = $Arguments }
        $link.WorkingDirectory = Split-Path -Parent $Target
        $link.IconLocation = "$Target,0"
        $link.Description = $Description
        $link.Save()
    } finally {
        if ($link) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($link) }
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell)
    }
}

# ------------------------------------------------------------------ удаление
if ($Remove) {
    Write-Head 'Удаление AuraExplorer'
    Stop-AuraExplorer
    # Registry first, while the program is still on disk: --uninstall-integration is what
    # puts folders, drives and This PC back to Explorer. Without it the folders keep
    # opening in a program that is no longer installed.
    Remove-Integration
    Remove-Autostart 'AuraExplorer'
    Remove-Autostart 'AuraDock'
    Remove-StaleAutostart
    Remove-Shortcut $StartMenu
    Remove-Shortcut $ShellLink
    Remove-Shortcut $OldShellLink
    Remove-Shortcut $FencesLink
    Remove-Shortcut $Desktop
    if (Test-Path -LiteralPath $Dir) {
        Remove-Item -LiteralPath $Dir -Recurse -Force
        Write-Note "папка $Dir удалена"
    }
    Write-Host ''
    Write-Host 'Готово. Настройки остались в %APPDATA%\AuraExplorer — удали папку вручную, если они больше не нужны.' -ForegroundColor Green
    Write-Host 'Запись о прежних настройках папок лежит отдельно, в %LOCALAPPDATA%\AuraUI — по ней открытие папок возвращается к Проводнику.' -ForegroundColor DarkGray
    return
}

# ------------------------------------------------------------------ установка
Write-Head 'Установка AuraExplorer'

# AuraService owns the suite startup. Every required executable in bundle.json must be
# present before install, so an incomplete desktop bundle is rejected rather than half-installed.
$needed = @('AuraExplorer.exe', 'AuraDock.exe', 'Fences.exe', 'widgets.exe', 'auraservice.exe')
$components = @()
$missing = @($needed | Where-Object { -not (Test-Path -LiteralPath (Join-Path $Source $_) -PathType Leaf) })
if ($missing.Count -gt 0) {
    Write-Host ("Рядом со скриптом нет: {0}." -f ($missing -join ', ')) -ForegroundColor Red
    Write-Host 'Положи install.ps1 рядом с AuraExplorer.exe, AuraDock.exe, Fences.exe и auraservice.exe (как в архиве сборки).' -ForegroundColor Red
    exit 1
}

# Опечатка сборки: manifest.json с версией и хешами всех файлов. Если он лежит рядом,
# файлы сверяются до копирования: подменённый или недокачанный архив не установится молча,
# а скажет, какой именно файл не тот.
$manifestPath = Join-Path $Source 'manifest.json'
if (Test-Path -LiteralPath $manifestPath -PathType Leaf) {
    Write-Head 'Проверка набора'
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json
    $bad = @()
    foreach ($entry in $manifest.files.PSObject.Properties) {
        $file = Join-Path $Source ($entry.Name.Replace('/', '\'))
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { $bad += "$($entry.Name): файла нет"; continue }
        $hash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ne ([string]$entry.Value.sha256)) { $bad += "$($entry.Name): файл изменён" }
    }
    if ($bad.Count -gt 0) {
        Write-Host 'Набор не сходится с manifest.json:' -ForegroundColor Red
        foreach ($line in $bad) { Write-Host "  $line" -ForegroundColor Red }
        Write-Host 'Скачай архив заново: файлы подменены или распакованы частично.' -ForegroundColor Red
        exit 1
    }
    Write-Ok ("версия {0}: сверены {1} файлов" -f $manifest.version, @($manifest.files.PSObject.Properties).Count)
} else {
    Write-Note 'manifest.json рядом нет — сверку файлов пропускаю (архив собран вручную)'
}

$sameFolder = ($Source.TrimEnd('\') -eq $Dir.TrimEnd('\'))
if (-not $sameFolder) { Stop-AuraExplorer }

New-Item -ItemType Directory -Force -Path $Dir | Out-Null
foreach ($file in $needed + $components) {
    $from = Join-Path $Source $file
    if (-not (Test-Path -LiteralPath $from -PathType Leaf)) {
        if ($needed -contains $file) { continue }
        Write-Note "$file нет в архиве — архив неполный: этот компонент не установится"
        continue
    }
    if (-not $sameFolder) {
        Copy-Item -LiteralPath $from -Destination $Dir -Force
    }
    Write-Note "$file -> $Dir"
}
# Скачанный архив помечен «из интернета»; снимаем метку, иначе SmartScreen ругается
# сильнее, чем на подпись.
try { Get-ChildItem -LiteralPath $Dir -Filter '*.exe' | Unblock-File -ErrorAction SilentlyContinue } catch { }

# Перехват окон проводника: DLL подгружается по имени из папки exe. Без неё перехват
# работает в упрощённом режиме (окна не прячутся до показа), поэтому копируем рядом.
foreach ($dll in Get-ChildItem -LiteralPath $Source -Filter 'AuraExplorerWindowHook-*.dll' -ErrorAction SilentlyContinue) {
    if (-not $sameFolder) {
        Copy-Item -LiteralPath $dll.FullName -Destination $Dir -Force
    }
    Write-Note ("{0} -> {1}" -f $dll.Name, $Dir)
}

# Visual C++ Runtime: AuraExplorer собран с динамическим CRT. Если рядом с exe лежат
# библиотеки — копируем их; если нет и в системе их тоже нет, предупреждаем.
$runtime = @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
foreach ($dll in $runtime) {
    if (Test-Path -LiteralPath (Join-Path $Source $dll)) {
        Copy-Item -LiteralPath (Join-Path $Source $dll) -Destination $Dir -Force
    }
}
if ($env:SystemRoot) {
    $system32 = Join-Path $env:SystemRoot 'System32'
    if (-not (Test-Path -LiteralPath (Join-Path $system32 'vcruntime140.dll'))) {
        Write-Host '  Внимание: в системе нет Visual C++ Runtime (vcruntime140.dll). Поставь «Microsoft Visual C++ 2015-2022 Redistributable (x64)».' -ForegroundColor Yellow
    }
}

# ------------------------------------------------------------------ автозапуск
Write-Head 'Автозапуск'
Remove-StaleAutostart
if ($NoAutostart) {
    Remove-Autostart 'AuraExplorer'
    Write-Note 'автозапуск не прописан'
} else {
    if ($PanelOnly) { Write-Note 'ключ -PanelOnly больше не нужен: набор компонентов задаётся в окне службы' }
    Set-Autostart 'AuraExplorer' ('"{0}" --run' -f $Service)
    Write-Ok "при входе в Windows запускается служба: $Service --run"
    Write-Note 'она поднимает панель, сетки и виджеты — по переключателям из окна «Запуск и компоненты»'
}

# ------------------------------------------------------------------ ярлыки
Write-Head 'Ярлыки'
Remove-Shortcut $OldShellLink
if ($StartMenu -or $Desktop) {
    New-Shortcut $StartMenu $Exe '' 'AuraExplorer — файловый менеджер с панелью и сетками'
    if ($StartMenu) { Write-Note "меню «Пуск»: $StartMenu" }
    New-Shortcut $ShellLink $Service '--settings' 'AuraUI — запуск и компоненты: что и когда запускать'
    if ($ShellLink) { Write-Note "меню «Пуск»: $ShellLink" }
    New-Shortcut $FencesLink $Fences '' 'Fences — самостоятельные сетки рабочего стола'
    if ($FencesLink) { Write-Note "меню «Пуск»: $FencesLink" }
    if ($DesktopShortcut) {
        New-Shortcut $Desktop $Exe '' 'AuraExplorer — файловый менеджер с панелью и сетками'
        if ($Desktop) { Write-Note "рабочий стол: $Desktop" }
    }
} else {
    Write-Note 'папки оболочки недоступны — ярлыки не создаются'
}

# ------------------------------------------------------------------ подпись
try {
    foreach ($file in Get-ChildItem -LiteralPath $Dir -Filter '*.exe') {
        $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
        $state = if ($signature.Status -eq 'Valid') { 'подпись проверена' } else { "подпись: $($signature.Status)" }
        Write-Note ("{0}: {1}" -f $file.Name, $state)
    }
} catch { Write-Note 'проверить подпись не удалось — на работу это не влияет' }

# ------------------------------------------------------------------ запуск
if (-not $NoStart) {
    Stop-AuraExplorer
    Start-Process -FilePath $Service -ArgumentList '--run' -WorkingDirectory $Dir
    Write-Ok 'служба запущена: Fences, панель и остальные компоненты запускаются по её настройкам'
}

Write-Host ''
Write-Host 'Установлено.' -ForegroundColor Green
Write-Host "  Папка:      $Dir"
Write-Host '  Панель:     внизу экрана, первая иконка — сам AuraExplorer'
Write-Host '  Сетки:      на рабочем столе, появляются сразу'
Write-Host '  Часы:       правый нижний угол, стеклянная карточка'
Write-Host '  Значок:     в трее; выход — правый клик по нему'
Write-Host '  Запуск:     «AuraUI — запуск и компоненты» в меню «Пуск» (auraservice.exe --settings)'
Write-Host ''
Write-Host 'Удалить: powershell -ExecutionPolicy Bypass -File install.ps1 -Remove'
