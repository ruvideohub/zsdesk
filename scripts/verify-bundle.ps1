<#
.SYNOPSIS
    Проверяет собранный набор AuraUI: всё ли на месте, нет ли лишнего, сходятся ли хеши.

.DESCRIPTION
    Одно описание набора — packaging/bundle.json. Этот скрипт сверяет с ним папку:

      1. каждый обязательный файл набора есть в папке;
      2. в папке нет файлов, которых нет в наборе (лишний файл — это либо забытая
         копия, либо забытое описание: и то и другое надо решать, а не возить с собой);
      3. manifest.json совпадает с набором: та же версия, каждый файл посчитан и хеш
         сходится — то есть файлы не подменили после упаковки;
      4. служба не называет программы, которой в наборе нет: иначе она будет искать
         файл, который не установлен, и молча не запускать компонент;
      5. install.ps1 знает про каждый exe набора — иначе установка его не скопирует.

    Скрипт ничего не меняет. Код возврата 1 — набор негоден, и в CI это красный шаг.

.PARAMETER Dir
    Папка набора (то, что распаковывается пользователю).

.PARAMETER Bundle
    Описание набора; по умолчанию packaging/bundle.json.

.PARAMETER SkipManifest
    Не требовать manifest.json и не сверять хеши (для проверки промежуточной папки).

.EXAMPLE
    pwsh -NoProfile -File scripts/verify-bundle.ps1 -Dir dist
#>
param(
    [Parameter(Mandatory = $true)][string]$Dir,
    [string]$Bundle,
    [switch]$SkipManifest
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $Bundle) { $Bundle = Join-Path $root 'packaging/bundle.json' }

$problems = New-Object System.Collections.Generic.List[string]
$notes = New-Object System.Collections.Generic.List[string]

if (-not (Test-Path -LiteralPath $Dir -PathType Container)) { throw "Нет папки набора: $Dir" }
if (-not (Test-Path -LiteralPath $Bundle -PathType Leaf)) { throw "Нет описания набора: $Bundle" }
$Dir = [IO.Path]::GetFullPath($Dir)
$description = Get-Content -LiteralPath $Bundle -Raw -Encoding utf8 | ConvertFrom-Json
$entries = @($description.entries)

# Версия — из Cargo.toml, единственного места, где она написана.
$version = [regex]::Match((Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if ($version -notmatch '^\d+\.\d+\.\d+([+-][A-Za-z0-9.-]+)?$') { throw 'Версия воркспейса не читается' }

# Что реально лежит в папке: пути через прямой слэш, как в manifest.json.
$present = @{}
foreach ($file in (Get-ChildItem -LiteralPath $Dir -Recurse -File)) {
    $relative = $file.FullName.Substring($Dir.Length + 1).Replace('\', '/')
    $present[$relative] = $file
}

function Test-Entry([string]$Name) {
    foreach ($key in $present.Keys) { if ($key -like $Name) { return $true } }
    return $false
}

# 1. Всё обязательное на месте.
foreach ($entry in $entries) {
    if (-not $entry.required) { continue }
    if (-not (Test-Entry $entry.name)) { $problems.Add("в наборе нет $($entry.name) ($($entry.title))") }
}

# 2. Лишнего нет: каждый файл папки описан набором.
foreach ($key in $present.Keys) {
    $described = $false
    foreach ($entry in $entries) { if ($key -like $entry.name) { $described = $true; break } }
    if (-not $described) { $problems.Add("файл $key не описан в bundle.json") }
}

# 3. manifest.json: версия и хеши.
$manifestPath = Join-Path $Dir 'manifest.json'
if (-not $SkipManifest) {
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        $problems.Add('нет manifest.json — сборка не опечатана, установка не сможет сверить файлы')
    } else {
        $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json
        if ($manifest.product -ne $description.product) { $problems.Add("manifest.json: продукт $($manifest.product), ожидался $($description.product)") }
        if ($manifest.version -ne $version) { $problems.Add("manifest.json: версия $($manifest.version), в Cargo.toml $version") }
        foreach ($key in $present.Keys) {
            if ($key -eq 'manifest.json') { continue }
            $recorded = $manifest.files.PSObject.Properties[$key]
            if (-not $recorded) { $problems.Add("manifest.json не знает про $key"); continue }
            $recorded = $recorded.Value
            $file = $present[$key]
            if ([int64]$recorded.bytes -ne $file.Length) { $problems.Add("размер $key не совпал с manifest.json"); continue }
            $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($hash -ne [string]$recorded.sha256) { $problems.Add("хеш $key не совпал с manifest.json — файл подменили после упаковки") }
        }
        # Манифест не описывает сам себя — иначе его «хеш» всегда устаревший, и установщик,
        # сверяющий файлы по манифесту, откажется от собственного архива.
        if ($manifest.files.PSObject.Properties['manifest.json']) {
            $problems.Add('manifest.json описывает сам себя: такой хеш не может быть верным')
        }
        foreach ($recordedName in $manifest.files.PSObject.Properties.Name) {
            if ($recordedName -eq 'manifest.json') { continue }
            if (-not $present.ContainsKey($recordedName)) { $problems.Add("manifest.json обещает $recordedName, а файла нет") }
        }
    }
}

# 4. Служба не ищет того, чего нет в наборе.
$launch = Join-Path $root 'crates/aura-service/src/launch.rs'
if (Test-Path -LiteralPath $launch -PathType Leaf) {
    $named = [regex]::Matches((Get-Content -LiteralPath $launch -Raw -Encoding utf8), 'exe:\s*"([^"]+)"') | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique
    foreach ($exe in $named) {
        $known = $false
        foreach ($key in $present.Keys) { if ($key -eq $exe) { $known = $true; break } }
        if (-not $known) { $problems.Add("служба запускает $exe, а в наборе такого файла нет") }
    }
    $notes.Add("служба называет: $($named -join ', ')")
}

# 5. Установщик знает про каждый exe.
$installer = Join-Path $root 'packaging/windows/install.ps1'
if (Test-Path -LiteralPath $installer -PathType Leaf) {
    $text = Get-Content -LiteralPath $installer -Raw -Encoding utf8
    foreach ($key in $present.Keys) {
        if ($key -notlike '*.exe') { continue }
        if ($text -notlike "*$key*") { $problems.Add("install.ps1 не копирует $key") }
    }
}

$exeCount = @($present.Keys | Where-Object { $_ -like '*.exe' }).Count
Write-Host "Набор $($description.product) $version ($($description.platform)): $($present.Count) файлов, $exeCount exe"
foreach ($note in $notes) { Write-Host "  $note" -ForegroundColor DarkGray }
if ($problems.Count -gt 0) {
    Write-Host 'Набор негоден:' -ForegroundColor Red
    foreach ($problem in $problems) { Write-Host "  - $problem" -ForegroundColor Red }
    exit 1
}
Write-Host 'Набор сходится с bundle.json: всё на месте, лишнего нет, хеши совпадают.' -ForegroundColor Green
exit 0
