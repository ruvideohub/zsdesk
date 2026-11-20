# Сборка и выпуск AuraUI

Раньше одно и то же знание «что входит в AuraUI» было записано трижды: в CI (список
`Copy-Item`), в `install.ps1` (список `$needed`/`$components`) и в `aura.iss`. Ещё один список
жил в `scripts/package.ps1` — от прежнего проекта, с шестью «изданиями» файлового менеджера,
которых в AuraUI уже нет. Расхождения никто не ловил: CI мог выпустить набор без файла, а
установщик — с файлом, которого в архиве не было.

Теперь знание одно: **`packaging/bundle.json`**.

## Что описывает bundle.json

Строка на файл: имя в наборе, роль (кто это), откуда его взять и обязателен ли он.

```json
{ "name": "auraservice.exe", "role": "service", "title": "Служба запуска и её настройки",
  "source": { "kind": "cargo", "package": "aura-service", "bin": "auraservice" },
  "required": true }
```

`source.kind` бывает четырёх видов:

| kind | Откуда | Пример |
|---|---|---|
| `cargo` | `target/release/<bin>.exe` после `cargo build -p <package> --bins` | `AuraDock.exe` |
| `cargo-glob` | единственный файл по маске (имя с хешем пишет build-скрипт) | `AuraExplorerWindowHook-*.dll` |
| `file` | файл из репозитория | `install.ps1`, `LICENSE` → `LICENSE.txt` |
| `dir` | папка из репозитория | `licenses/*` |
| `generated` | файл, который пишет сам упаковщик | `manifest.json` |

Набор AuraUI: шесть программ (`AuraExplorer`, `AuraDock`, `fences`, `widgets`, `auraservice`,
`AuraArc`), DLL перехвата окон, установка (`install.ps1`, `install.cmd`, `install-readme.txt`),
лицензии и `manifest.json`.

## Как выпускается сборка

```powershell
pwsh -NoProfile -File scripts/package.ps1
```

1. Читает версию из `Cargo.toml` (единственное место, где она написана) и набор из
   `packaging/bundle.json`.
2. Собирает пакеты, которые упомянуты в наборе: `cargo build --locked --release --bins -p …`.
3. Раскладывает папку `releases/<версия>/AuraUI-<версия>-x64`.
4. Пишет `manifest.json`: продукт, версия, платформа, время сборки, размер и sha256 каждого
   файла. Он пишется **последним**, потому что считает все остальные, и **сам себя не
   описывает** — иначе его хеш был бы заведомо устаревшим, и `install.ps1` отказался бы от
   собственного архива.
5. Зовёт `scripts/verify-bundle.ps1` и падает, если набор не сошёлся.
6. Упаковывает zip `AuraUI-<версия>-x64.zip` и печатает его размер и sha256.
7. Если найден Inno Setup 6 (или задан `-InnoCompiler`) — собирает
   `AuraUI-<версия>-x64-setup.exe`.

Полезные ключи: `-SkipBuild` (взять уже собранное), `-TargetDir`, `-StageDir`, `-OutputDir`,
`-NoZip`, `-KeepStage`, `-InnoCompiler`.

## Что проверяет verify-bundle.ps1

```powershell
pwsh -NoProfile -File scripts/verify-bundle.ps1 -Dir dist
```

1. **Всё обязательное на месте.** Нет `widgets.exe` — красный шаг и имя файла.
2. **Лишнего нет.** Любой файл, которого нет в описании, — ошибка: это либо забытая копия,
   либо забытое описание, и в обоих случаях набор надо привести в порядок, а не возить как есть.
3. **Хеши сходятся с `manifest.json`.** Сборка, собранная из разных версий файлов или
   подменённая после упаковки, не пройдёт.
4. **Служба не зовёт того, чего нет.** Все `exe` из `crates/aura-service/src/launch.rs`
   должны быть в наборе: иначе служба будет искать файл, который не установлен, и молча
   не запускать компонент.
5. **Установщик знает про каждый exe.** `install.ps1` должен копировать все exe набора.

Код возврата 1 — набор негоден. Скрипт ничего не меняет: только читает и говорит.

## Что делает CI

`.github/workflows/dock-build.yml` больше не держит свой список файлов. Он собирает пакеты и
зовёт тот же упаковщик:

```powershell
pwsh -NoProfile -File scripts/package.ps1 -SkipBuild -TargetDir target/release -StageDir dist -NoZip
```

То есть CI и релиз раскладывают набор **одним и тем же кодом**, и проверка та же. Дальше
CI подписывает файлы, грузит артефакт `aura-dock-windows-x64` (набор целиком, вместе с
`manifest.json`) и, если на образе есть Inno Setup, собирает установщик вторым артефактом.
Отсутствие Inno Setup выпуск не ломает: zip и `install.ps1` — самостоятельные способы установки.

Изменения в `packaging/**` и `scripts/**` теперь тоже запускают CI: правка упаковки должна
проверяться, а не проезжать мимо.

## Три способа установки, одна проверка

| Способ | Что делает | Проверка файлов |
|---|---|---|
| `install.ps1` / `install.cmd` | копирует программы в профиль, ярлыки, автозапуск службы | сверяет sha256 по `manifest.json` до копирования |
| `AuraUI-<версия>-x64-setup.exe` | обычный установщик Inno Setup, ставит в `%LOCALAPPDATA%\Programs\AuraUI` | собирается только из проверенного набора |
| руками из zip | распаковать и запустить `AuraExplorer.exe` | `manifest.json` рядом |

Автозапуск во всех трёх случаях один и тот же: одна запись `HKCU\…\Run` →
`"<папка>\auraservice.exe" --run`. Что именно поднимает служба, решается её переключателями
(«AuraUI — настройки и запуск» в меню «Пуск»), а не тем, какой файл установщик положил первым.

## Как добавить компонент

1. Добавить его exe в `packaging/bundle.json` (роль, `source`, `required`).
2. Если это компонент службы — дописать его в `crates/aura-service/src/launch.rs`
   (`COMPONENTS` или `PROGRAMS`) и в `install.ps1`.
3. `pwsh -NoProfile -File scripts/verify-bundle.ps1 -Dir <папка набора>` — он скажет, если
   где-то забыли строку. Затем `scripts/package.ps1`.

Забыть нельзя молча: проверка либо проходит, либо называет файл.
