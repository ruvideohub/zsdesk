# Как `fences.exe` перестанет быть копией `AuraExplorer.exe`

Замечание по делу: до сих пор `fences.exe` — это тот же самый код (`aura-explorer`),
собранный вторым бинарником. Совпадает всё: 9 095 строк хоста сеток лежат в
`crates/aura-explorer/src/desktop/`, общий ключ одного экземпляра, общий файл сессии
`explorer-session.json`, тот же значок и те же строки версии. Ниже — что уже исправлено и в
каком порядке код разъезжается по-настоящему.

## Уже исправлено (0.3.24)

* **Режим задаёт программа, а не командная строка.** `fences.exe` подставляет себе `--desktop`
  (см. `aura_explorer::fences_arguments`). Строка службы `fences.exe --tray` раньше читалась
  как *файловое окно с файлом «--tray»* — хост сеток не запускался вообще.
* **Свой ключ одного экземпляра.** `instance::property` хеширует имя программы вместе с
  профилем: `fences.exe` и `AuraExplorer.exe` — две программы, и запуск сеток больше не
  пересылается уже открытому файловому окну.
* **Чужой путь — ошибка, а не окно.** `fences.exe <путь>` отвечает понятным отказом: этот
  бинарник не открывает файлы.
* Оба правила закрыты тестами (`fences_arguments`, `property_for`), чтобы вернуться не могли.

## Порядок вырезания

1. **`aura-shell-menu` — общая подсистема контекстного меню.** `context_menu_process` (1 276
   строк), `context_menu_common` (266), `context_menu_actions` (143), `context_menu_child`
   (246), `context_menu_background` (398), `context_preferences` (30) нужны и файловому окну, и
   сеткам. По правилу «компоненты не зависят друг от друга» они переезжают в отдельный общий
   крейт (как `aura-windows`), и оба компонента зависят от него, а не друг от друга.
2. **Мелкие общие помощники — в `aura-core`/`aura-windows`:** `archive` (138 строк, запуск
   `AuraArc`), `explorer_forwarder::same_path`, `runtime::trace`, `flyout_menu::MenuAnchor`,
   `edition` (какая сборка), `window::diagnostics::IterationTimer`.
3. **`crates/aura-fences`:** сюда переезжает `desktop/` (19 файлов, 9 095 строк) и
   `bin/fences.rs`. Крейт зависит от `aura-core`, `aura-windows`, `aura-shell-menu` — и ни от
   чего из `aura-explorer`.
4. **`aura-explorer` теряет `desktop/`.** Режим `AuraExplorer.exe --desktop` остаётся, но
   становится запуском `fences.exe`, а не вторым хостом сеток внутри себя.
5. **Свои ресурсы:** `fences.ico` и блок версии с `OriginalFilename=fences.exe` (отдельные
   resource-цели в `build.rs` — сейчас `embed_resource` даёт один `.rc` на пакет).
6. **Проверка в CI:** `scripts/check-programs.py` — каждый поставляемый бинарник обязан иметь
   свою точку входа и свои ресурсы; два бинарника одного крейта с одинаковым размером и
   одинаковым `.rc` считаются ошибкой, а не «так и надо».

Шаги 1–2 механические и проверяются сборкой; шаг 3 — перенос файлов и правка 63 ссылок
`crate::…` в `desktop/` (по счёту: `context_menu_process` — 21, `storage` — 8,
`explorer_forwarder` — 5, `edition` — 5, `flyout_menu` — 4, остальное по 1–3).
