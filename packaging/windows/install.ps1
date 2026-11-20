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
$Desktop    = if ($DesktopDir) { Join-Path $DesktopDir 'AuraExplorer.lnk' } else { $null }
$Source    = $PSScriptRoot
if (-not $Source) { $Source = (Get-Location).Path }
if (-not $Dir) { throw 'Не задана папка установки и пуст %LOCALAPPDATA%: запусти с -Dir "C:\AuraExplorer".' }
$Exe       = Join-Path $Dir 'AuraExplorer.exe'
$Dock      = Join-Path $Dir 'AuraDock.exe'
$Service   = Join-Path $Dir 'auraservice.exe'

function Write-Head([string]$Text) { Write-Host ''; Write-Host $Text -ForegroundColor Cyan }
function Write-Note([string]$Text) { Write-Host "  $Text" -ForegroundColor DarkGray }
function Write-Ok([string]$Text) { Write-Host "  $Text" -ForegroundColor Green }

function Stop-AuraExplorer {
    # Вежливо: просим запущенный AuraExplorer закрыться (он сам погасит панель и вернёт
    # панель задач Windows), потом ждём, и только затем добиваем остатки.
    if (Test-Path -LiteralPath $Exe) {
        try { Start-Process -FilePath $Exe -ArgumentList '--desktop', '--quit' -Wait -ErrorAction Stop | Out-Null } catch { }
        for ($i = 0; $i -lt 40; $i++) {
            $alive = Get-Process -Name 'AuraExplorer' -ErrorAction SilentlyContinue
            if (-not $alive) { break }
            Start-Sleep -Milliseconds 250
        }
    }
    # Служба идёт первой: иначе она поднимет то, что мы только что закрыли.
    Get-Process -Name 'auraservice', 'AuraExplorer', 'aura-dock', 'fences', 'widgets' -ErrorAction SilentlyContinue | ForEach-Object {
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

# Прежние записи автозапуска компонентов: сетки (`AuraExplorer.exe --desktop --profile-dir …`
# или `fences.exe --desktop …`) и панель (`AuraDock.exe`). Их работу теперь делает служба,
# а оставленная запись подняла бы второй экземпляр — или вообще старую программу.
function Remove-StaleAutostart {
    $key = Get-Item -LiteralPath $RunKey -ErrorAction SilentlyContinue
    if (-not $key) { return }
    foreach ($name in @($key.GetValueNames())) {
        if ($name -eq 'AuraExplorer') { continue }
        $value = [string]$key.GetValue($name)
        $ours = $value -match 'AuraExplorer(\.exe)?["\s]' -or $value -match 'fences\.exe' -or $value -match 'AuraDock\.exe'
        if ($ours -and $value -match '--desktop') {
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
    Remove-Autostart 'AuraExplorer'
    Remove-Autostart 'AuraDock'
    Remove-Shortcut $StartMenu
    Remove-Shortcut $ShellLink
    Remove-Shortcut $OldShellLink
    Remove-Shortcut $Desktop
    if (Test-Path -LiteralPath $Dir) {
        Remove-Item -LiteralPath $Dir -Recurse -Force
        Write-Note "папка $Dir удалена"
    }
    Write-Host ''
    Write-Host 'Готово. Настройки остались в %APPDATA%\AuraExplorer — удали папку вручную, если они больше не нужны.' -ForegroundColor Green
    return
}

# ------------------------------------------------------------------ установка
Write-Head 'Установка AuraExplorer'

# auraservice.exe — служба запуска: единственный владелец автозапуска и всех компонентов.
# fences.exe — сетки рабочего стола, widgets.exe — виджеты, AuraArc.exe — архиватор
# (работает и сам по себе). Весь набор описан в packaging/bundle.json, и если файла рядом
# нет, архив неполный: скажем об этом прямо, но установку из-за этого не рвём.
$needed = @('AuraExplorer.exe', 'AuraDock.exe', 'auraservice.exe')
$components = @('fences.exe', 'widgets.exe', 'AuraArc.exe')
$missing = @($needed | Where-Object { -not (Test-Path -LiteralPath (Join-Path $Source $_) -PathType Leaf) })
if ($missing.Count -gt 0) {
    Write-Host ("Рядом со скриптом нет: {0}." -f ($missing -join ', ')) -ForegroundColor Red
    Write-Host 'Положи install.ps1 в ту же папку, что AuraExplorer.exe, AuraDock.exe и auraservice.exe (как в архиве сборки).' -ForegroundColor Red
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
    if (Test-Path -LiteralPath $Service) {
        Start-Process -FilePath $Service -ArgumentList '--run' -WorkingDirectory $Dir
        Write-Ok 'служба запущена: панель, сетки и виджеты поднимаются ею'
    } else {
        Start-Process -FilePath $Exe -ArgumentList '--desktop', '--tray' -WorkingDirectory $Dir
        Write-Ok 'AuraExplorer запущен (служба не установлена)'
    }
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
