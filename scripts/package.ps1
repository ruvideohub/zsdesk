<#
.SYNOPSIS
    Собирает набор AuraUI: папку с программами, manifest.json, zip, при желании — установщик.

.DESCRIPTION
    Что именно входит в набор, решает не этот скрипт, а packaging/bundle.json — одно описание
    на весь проект. Скрипт только исполняет его:

      1. собирает нужные пакеты (`cargo build --release --bins`), если не сказано -SkipBuild;
      2. раскладывает файлы по папке набора и пишет manifest.json с версией и хешами;
      3. зовёт scripts/verify-bundle.ps1 — и падает, если набор не сошёлся с описанием;
      4. упаковывает папку в zip и печатает его размер и sha256;
      5. если есть Inno Setup (или задан -InnoCompiler) — собирает установщик из
         packaging/windows/aura.iss.

    Никаких «изданий» здесь нет: в AuraUI один набор. Прежний скрипт собирал шесть изданий
    файлового менеджера с функциями, которых в проекте уже нет, и молча выпускал неполный
    архив — именно поэтому он переписан.

.PARAMETER TargetDir
    Где лежат собранные exe. По умолчанию target/release.

.PARAMETER StageDir
    Куда раскладывать набор. По умолчанию releases/<версия>/AuraUI-<версия>-x64.

.PARAMETER OutputDir
    Куда положить zip и установщик. По умолчанию releases/<версия>.

.PARAMETER InnoCompiler
    Путь к ISCC.exe. Без него установщик не собирается; zip и install.ps1 собираются всегда.

.PARAMETER SkipBuild
    Не собирать cargo: взять то, что уже лежит в -TargetDir (так делает CI, который собрал
    пакеты отдельным шагом).

.PARAMETER NoZip
    Не упаковывать в zip (нужно CI: он загружает папку как артефакт).

.PARAMETER KeepStage
    Не удалять прежнюю папку набора, а обновлять её.

.PARAMETER ManifestOnly
    Ничего не собирать и не раскладывать: пересчитать manifest.json в готовой папке и
    проверить её. Нужно после подписи — она меняет байты exe, и прежние хеши перестают
    сходиться (иначе install.ps1 отверг бы собственный подписанный архив).

.EXAMPLE
    pwsh -NoProfile -File scripts/package.ps1

.EXAMPLE
    pwsh -NoProfile -File scripts/package.ps1 -SkipBuild -TargetDir target/release -StageDir dist -NoZip
#>
param(
    [string]$TargetDir,
    [string]$StageDir,
    [string]$OutputDir,
    [string]$InnoCompiler,
    [switch]$SkipBuild,
    [switch]$NoZip,
    [switch]$KeepStage,
    [switch]$ManifestOnly
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $TargetDir) { $TargetDir = Join-Path $root 'target\release' }
$TargetDir = [IO.Path]::GetFullPath($TargetDir)

# Версия и описание набора — единственные источники правды.
$version = [regex]::Match((Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if ($version -notmatch '^\d+\.\d+\.\d+([+-][A-Za-z0-9.-]+)?$') { throw 'Версия воркспейса не читается' }
$bundlePath = Join-Path $root 'packaging/bundle.json'
if (-not (Test-Path -LiteralPath $bundlePath -PathType Leaf)) { throw "Нет описания набора: $bundlePath" }
$description = Get-Content -LiteralPath $bundlePath -Raw -Encoding utf8 | ConvertFrom-Json
$entries = @($description.entries)
if (-not $StageDir) { $StageDir = Join-Path $root "releases\$version\AuraUI-$version-x64" }
if (-not $OutputDir) { $OutputDir = Join-Path $root "releases\$version" }
$StageDir = [IO.Path]::GetFullPath($StageDir)
$OutputDir = [IO.Path]::GetFullPath($OutputDir)

Write-Host "Набор $($description.product) $version ($($description.platform))"

# ------------------------------------------------------------------ сборка
if (-not $SkipBuild -and -not $ManifestOnly) {
    $buildFlags = @('build', '--locked', '--release', '--bins', '--manifest-path', (Join-Path $root 'Cargo.toml'))
    $packages = @($entries | Where-Object { $_.source.kind -eq 'cargo' } | ForEach-Object { $_.source.package } | Sort-Object -Unique)
    foreach ($package in $packages) {
        Write-Host "  cargo build -p $package"
        & cargo @buildFlags -p $package
        if ($LASTEXITCODE -ne 0) { throw "cargo build не собрал $package" }
    }
}

# ------------------------------------------------------------------ раскладка
if ((Test-Path -LiteralPath $StageDir) -and -not $KeepStage -and -not $ManifestOnly) {
    Remove-Item -LiteralPath $StageDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null

$layout = if ($ManifestOnly) { @() } else { $entries }
foreach ($entry in $layout) {
    $kind = $entry.source.kind
    switch ($kind) {
        'cargo' {
            $bin = $entry.source.bin
            $from = Join-Path $TargetDir ($bin + '.exe')
            if (-not (Test-Path -LiteralPath $from -PathType Leaf)) { throw "Сборка не дала $bin.exe (искал в $TargetDir)" }
            Copy-Item -LiteralPath $from -Destination (Join-Path $StageDir $entry.name) -Force
        }
        'cargo-glob' {
            $pattern = $entry.source.pattern
            $found = @(Get-ChildItem -LiteralPath $TargetDir -Filter $pattern -File)
            if ($found.Count -ne 1) { throw "Ожидался ровно один $pattern в $TargetDir, найдено $($found.Count)" }
            Copy-Item -LiteralPath $found[0].FullName -Destination $StageDir -Force
        }
        'file' {
            $from = Join-Path $root $entry.source.path
            if (-not (Test-Path -LiteralPath $from -PathType Leaf)) { throw "Нет файла $($entry.source.path)" }
            Copy-Item -LiteralPath $from -Destination (Join-Path $StageDir $entry.name) -Force
        }
        'dir' {
            $from = Join-Path $root $entry.source.path
            if (-not (Test-Path -LiteralPath $from -PathType Container)) { throw "Нет папки $($entry.source.path)" }
            $folder = $entry.name -replace '/\*$', ''
            Copy-Item -LiteralPath $from -Destination (Join-Path $StageDir $folder) -Recurse -Force
        }
        'generated' { }
        default { throw "Неизвестный вид источника: $kind" }
    }
}

# ------------------------------------------------------------------ манифест
# Последним: он считает хеши всех остальных файлов, включая те, что кладёт установщик.
$files = [ordered]@{}
foreach ($file in (Get-ChildItem -LiteralPath $StageDir -Recurse -File | Sort-Object FullName)) {
    if ($file.LastWriteTimeUtc.Year -lt 1980) { $file.LastWriteTimeUtc = [datetime]'1980-01-01T00:00:00Z' }
    $relative = $file.FullName.Substring($StageDir.Length + 1).Replace('\', '/')
    # Манифест не описывает сам себя: при -ManifestOnly он уже лежит в папке, и записать его
    # хеш невозможно — файл меняется ровно тем, что в него пишется. Запись «сам себя» с
    # устаревшим хешем заставила бы install.ps1 отказаться от собственного архива.
    if ($relative -eq 'manifest.json') { continue }
    $files[$relative] = [ordered]@{
        bytes  = $file.Length
        sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
$manifest = [ordered]@{
    product  = $description.product
    version  = $version
    platform = $description.platform
    built    = [DateTimeOffset]::Now.ToString('o')
    files    = $files
}
# Без BOM: manifest.json читают и PowerShell, и другие инструменты — обычный UTF-8.
$json = $manifest | ConvertTo-Json -Depth 10
[IO.File]::WriteAllText((Join-Path $StageDir 'manifest.json'), $json, (New-Object System.Text.UTF8Encoding($false)))

# ------------------------------------------------------------------ проверка
Write-Host '  проверяю набор'
& (Join-Path $PSScriptRoot 'verify-bundle.ps1') -Dir $StageDir -Bundle $bundlePath
if ($LASTEXITCODE -ne 0) { throw 'Набор не прошёл проверку — смотри сообщения выше' }

# ------------------------------------------------------------------ zip
if (-not $NoZip) {
    New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
    $zip = Join-Path $OutputDir ("AuraUI-$version-x64.zip")
    if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
    Compress-Archive -Path (Join-Path $StageDir '*') -DestinationPath $zip -CompressionLevel Optimal
    $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Host "  zip: $zip"
    Write-Host "  размер: $((Get-Item -LiteralPath $zip).Length) байт, sha256 $hash"
}

# ------------------------------------------------------------------ установщик
if (-not $InnoCompiler) {
    foreach ($candidate in @((Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'), (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'))) {
        if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) { $InnoCompiler = $candidate; break }
    }
}
if ($InnoCompiler) {
    New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
    $iss = Join-Path $root 'packaging/windows/aura.iss'
    Write-Host "  установщик: $InnoCompiler"
    & $InnoCompiler "/DSourceDir=$StageDir" "/DOutputDir=$OutputDir" "/DMyAppVersion=$version" $iss
    if ($LASTEXITCODE -ne 0) { throw 'Inno Setup не собрал установщик' }
    $setup = Join-Path $OutputDir ("AuraUI-$version-x64-setup.exe")
    if (Test-Path -LiteralPath $setup) {
        Write-Host "  установщик: $setup ($((Get-Item -LiteralPath $setup).Length) байт)"
    }
} else {
    Write-Host '  Inno Setup не найден: установщик не собран (zip и install.ps1 готовы).'
}

Write-Host "Готово: $StageDir" -ForegroundColor Green
