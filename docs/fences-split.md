# Fences — отдельный компонент AuraUI

Fences больше не является вторым бинарником `aura-explorer` и не запускает desktop-host внутри
AuraExplorer. Рабочий стол и сетки принадлежат отдельному процессу `Fences.exe` из Cargo-пакета
`aura-fences`; доменная модель находится в `aura-fences-model`.

## Что изменено

- `crates/aura-explorer/src/desktop/` и прежний `aura-explorer` bin `fences` удалены.
- `crates/aura-fences/src/desktop/` содержит самостоятельный хост, а отдельные ресурсы описывают
  продукт как `Fences.exe`.
- AuraExplorer отвергает старые аргументы запуска desktop-host (`--desktop`, `--desktop-guard`,
  `--tray`, `--organize` и связанные команды) вместо того, чтобы незаметно открывать окно Explorer.
  В интерфейсе AuraExplorer те же действия передаются соседнему `Fences.exe` с тем же профилем.
- Fences использует собственный профиль `%APPDATA%\\AuraFences`, поддерживает миграцию старой
  раскладки и передаёт открытие файлов/папок системному `explorer.exe`.
- Fences не запускает AuraExplorer или AuraDock и может работать как отдельный продукт.

## Запуск и общие настройки

В полном AuraUI-наборе `auraservice.exe` является владельцем запуска компонентов и общей записи
автозапуска. Компонент `fences` запускается как `Fences.exe --tray`; при необходимости служба
передаёт ему `--profile-dir`. Обычный запуск AuraExplorer только обеспечивает запуск
`auraservice.exe --run`, если он ещё не работает.

При установленной службе меню Fences «Запуск и компоненты…» и соответствующий пункт меню
AuraExplorer открывают `auraservice.exe --settings`. Они не создают отдельный переключатель
автозапуска Fences. В standalone Fences-пакете, где соседнего `auraservice.exe` нет, Fences
сохраняет собственную необязательную запись запуска при входе; установщик этого пакета может
её включить, а uninstall удаляет запись именно этой копии.

## Сборка и установка

- `packaging/bundle.json` называет `Fences.exe` Cargo-целью `aura-fences` / `Fences`; общий
  `scripts/package.ps1` собирает и проверяет весь AuraUI-набор.
- `.github/workflows/dock-build.yml` явно собирает `aura-fences --bin Fences` отдельно от
  `aura-explorer` и `aura-dock`.
- `packaging/windows/aura.iss` и `packaging/windows/install.ps1` кладут Fences рядом с
  AuraService и AuraExplorer; они не включают для Fences отдельный автозапуск.
- `scripts/package-fences.ps1` создаёт отдельный standalone ZIP и, если передан Inno Setup,
  installer из `packaging/windows/fences.iss`. В этот пакет не входят AuraExplorer, AuraDock
  или AuraService.
- `python scripts/check_fences_boundary.py` проверяет границы крейтов, источник бинарника,
  общий запуск службы и статические предположения об установщиках.

Граница проверена статически. `cargo test`, Rust-сборки, запуск Windows-программ и компиляция
Inno Setup должны быть выполнены на Windows с соответствующими инструментами.
