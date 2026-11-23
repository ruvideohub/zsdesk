# AuraExplorer / AuraUI

AuraExplorer 是 Windows 多标签文件管理器；桌面格子和文件夹映射由独立应用 **Fences** 提供。
完整 AuraUI 套件通过 `auraservice.exe` 集中管理 Fences、AuraDock 与文件管理器的启动设置。

[下载安装](https://github.com/ruvideohub/zsdesk/releases/latest) · [使用说明](docs/user-guide.md) · [更新日志](CHANGELOG.md) · [问题反馈](https://github.com/ruvideohub/zsdesk/issues)

适用于 Windows 10 / 11，64 位。当前工作区版本为 **0.3.23**，支持简体中文和英文界面。

English UI is available. Non-Chinese Windows systems use English by default. Choose **≡ → Language → English** to change the application language immediately.

## 选择套件

| 包 | 适合谁 |
|---|---|
| **AuraUI 完整套件** | 文件管理、搜索、Fences 桌面格子、AuraDock 和 AuraService；一般用户使用此包 |
| **Standalone Fences** | 只需要桌面格子和文件夹映射；不依赖 AuraExplorer、AuraDock 或 AuraService |

AuraUI 完整套件提供安装程序和便携 ZIP；Standalone Fences 有独立 ZIP 和可选安装器。
两种方式均面向当前 Windows 用户，不需要管理员权限。完整套件的共享自动启动由
`auraservice.exe` 管理；独立 Fences 可以选择自己的登录启动。

## 开始使用

1. 安装或解压完整 AuraUI 套件后打开 AuraExplorer，先用普通文件夹熟悉操作。
2. 按 **Ctrl+T** 新建标签，**Ctrl+L** 输入目录，**Ctrl+F** 搜索。
3. 在 **≡ → 布局** 开启双栏、调整标签位置和命令栏；两栏分别保留自己的目录与标签。
4. 需要替代系统文件管理器时，进入 **≡ → 系统集成 → 默认打开文件夹、磁盘和此电脑**；选择“不接管系统资源管理器”可恢复。
5. 从 AuraExplorer 菜单或 Fences 快捷方式启动桌面格子；完整套件的开关位于 **AuraUI — 启动与组件**。删除格子不会删除磁盘文件。

## 常用操作

| 操作 | 快捷键或入口 |
|---|---|
| 新建、关闭、恢复标签 | Ctrl+T / Ctrl+W / Ctrl+Shift+T |
| 顶部标签栏 | 右键关闭所点标签，右键空白处关闭当前标签，Shift+右键打开标签菜单 |
| 在新标签打开文件夹 | 鼠标中键点击文件夹 |
| 后退、前进、上一级 | Alt+← / Alt+→ / Alt+↑，支持鼠标侧键 |
| 调整图标大小 | Ctrl+滚轮 |
| 快速预览 | 空格；右侧预览窗格用 Alt+P |
| 新窗口 | Ctrl+N |
| 双击空白处返回上一级 | 在“预览与操作”中开关 |
| 显示文件夹总大小并排序 | 排序 → 计算并按总大小排序 |
| 检查版本更新 | 关于 AuraExplorer → 检查更新 |

## 搜索、预览和右键

- **搜索**：只有一个引擎——自带搜索（共享内存索引与本地快照）；网络共享及已有映射网络目录使用限定目录遍历搜索。首次建立本地索引需要时间，索引就绪状态显示在搜索结果里。
- **预览**：内置预览与 QuickLook 可选择。Office 文档预览需要相应的已安装组件；图片、音视频格式取决于系统解码器。
- **右键**：Windows 10 使用经典系统菜单；Windows 11 按系统能力选择已验证的原生呈现或公共 Shell 命令兼容呈现。Shift 右键可直接使用经典菜单。
- **Listary**：安装和连接方式见[详细使用说明](docs/user-guide.md)及 `integrations/listary`。

## 0.3.23：Fences 独立组件

- `Fences.exe` 现在从 `aura-fences` 单独构建，不再是 AuraExplorer 的桌面模式。
- 完整 AuraUI 由 `auraservice.exe` 集中管理组件启动；Fences 菜单打开同一个启动设置窗口。
- Standalone Fences 可单独下载，不需要 AuraExplorer、AuraDock 或 AuraService；文件打开由 Windows Explorer 处理。
- 打包边界、Cargo 目标、CI、安装脚本和 Inno Setup 已指向独立 Fences 可执行文件。

## 0.3.21 更新

- 右键顶部标签关闭所点页，右键标签栏空白处关闭当前页，Shift+右键打开标签菜单。

- “右键菜单 → 优先显示常用命令”可在加载扩展前显示常用操作，完整菜单仍可从末项打开。
- 快捷键列表支持滚轮和键盘翻页，搜索快捷键设置在当前面板打开。
- 内置搜索降低大结果集分页和索引快照的峰值内存。安装包和便携包含 Listary 6、7.0 的当前目录插件。

## 0.3.17 更新

- 增加英文界面和语言设置，修复网络文件夹搜索、剪切图标变浅及搜索后导航。
- 分离鼠标观察与桌面绘制，减少图标完成后的重复重绘，修复索引空闲释放，限制空闲菜单进程和文件窗口线程。
- 增加文件管理器托盘开关与右侧收藏夹宽度调整，改善任务栏分组、远程文件剪贴板及安装升级退出。

- 个别环境中当前标签难以辨认时，可开启“≡ → 布局 → 当前标签增强对比”；默认关闭，不改变既有主题，增强模式使用中性色轮廓而非蓝色短线。
- 地址栏支持在当前本地目录启动 `cmd`、`cmd.exe`；网络目录使用 PowerShell 或 pwsh。
- “接管新出现的系统浏览窗口”可以独立开启：不设为默认文件管理器时，仅在文件管理窗口打开期间生效，最后一个窗口关闭后停止。
- 改善 Win+E 按键与窗口激活时序，以及连续右键点击时的菜单准备行为。
- 启动时每天最多检查一次更新，可在“关于”中关闭；发现新版本后由用户决定下载。下载自动匹配版本类型并核对 SHA-256，安装版可启动安装程序，便携版打开已校验的 ZIP 后手动替换程序，保留 `data`。
- 包含文件选择窗口、上传文件夹识别与外部目录定位的修复。
- 原选择窗口具有额外选择动作或自定义确认下拉时，保留完整原生选择器及其回调。
- 桌面图标拖动时按可见窗口避让，历史布局中的不可见项目不占用落点；从格子拖出保留实际抓取点。

## Панель и Launchpad вместо меню «Пуск»

Панель внизу экрана — **Laminar Dock** (`davesousa/Laminar_Dock`, MIT), взятый в
исходниках: `crates/aura-dock` и `crates/laminar-sys`. В ней закреплённые и
запущенные приложения, стеки, превью окон, трей, поиск и Launchpad.

- **Один набор**: `packaging/bundle.json` описывает AuraExplorer, Fences, панель,
  AuraService и архиватор. `scripts/package.ps1` собирает набор, а
  `scripts/verify-bundle.ps1` сверяет файлы и manifest. Подробности — [docs/packaging.md](docs/packaging.md).
- **Один владелец запуска**: `auraservice.exe` хранит общие переключатели и запускает Fences,
  AuraDock и AuraExplorer. `widgets.exe` пока остаётся дочерним процессом AuraDock.
  Standalone Fences работает и без этой службы.
- **Launchpad проще апстрима**: одна тихая панель по центру (~70 % экрана),
  поиск сверху, сетка приложений в середине, счётчик страниц — и ничего больше.
  Колонки виджетов и центр управления из Laminar выключены.
- Настройки панели: `%APPDATA%\AuraExplorer\Dock\config.json`. Панель не обновляет
  себя; в полном AuraUI-наборе обновление панели приходит вместе с suite-установщиком.
- Интерфейс панели и Launchpad — на русском (`language` в `config.json`: `"ru"`, `"en"`,
  `"auto"`), из апстрима убраны виджеты, коннекторы и «О программе».
- Иконка AuraExplorer — первая на панели; на рабочем столе — стеклянный виджет часов
  в стиле macOS (время и дата по-русски, не перекрывает окна).
- **Часы настраиваются сами**: правый клик по карточке — угол, размер, 12/24 часа, дата,
  секунды, «убрать часы»; перетаскивание со снапом к ближайшему углу.
- **Виджеты на панели**: правый клик по панели → «Виджеты на панели» (часы, погода,
  календарь, монитор ресурсов, батарея, «сейчас играет», сеть, быстрые действия) с
  галочками; «Другие виджеты…» — весь каталог в галерее Launchpad.
- **Свой значок у панели**: `AuraDock.exe` несёт иконку (золотая полоса с плитками,
  16–256 px) и номер версии, поэтому в Проводнике, «Пуске» и списке программ видно
  панель, а не пустую системную иконку. Рисуется из кода: `python3 scripts/make-icons.py`.

### Установка и автозапуск

```powershell
# из папки с AuraExplorer.exe, Fences.exe, AuraDock.exe и auraservice.exe
powershell -ExecutionPolicy Bypass -File install.ps1

# или двойным щелчком: install.cmd
install.cmd -Remove        # удалить: процессы, автозапуск, ярлыки, папку
```

- Ставит полный набор в профиль пользователя (по умолчанию `%LOCALAPPDATA%\Programs\AuraExplorer`), администратор не нужен.
- Перед копированием сверяет файлы с `manifest.json` (версия и sha256 каждого): подменённый
  или недокачанный архив не установится молча.
- Автозапуск — одна запись `HKCU\…\Run`: `"<папка>\auraservice.exe" --run`. Что именно она
  поднимает, решается переключателями в окне «AuraUI — настройки и запуск»
  (`auraservice.exe --settings`): компоненты, автозапуск и журнал — там же.
- Отдельные записи для панели или сеток не нужны и вредны: два хозяина запуска дают две
  панели, которые дерутся за место на экране.
- Прочее: `-NoAutostart`, `-DesktopShortcut`, `-NoStart`, `-Dir <путь>`.

```powershell
# весь набор сразу: сборка, manifest.json, проверка, zip (и установщик, если есть Inno Setup)
pwsh -NoProfile -File scripts/package.ps1
```

Что именно изменено относительно апстрима, почему панель осталась отдельным
процессом и как устроен Launchpad — в [docs/dock.md](docs/dock.md).

## 反馈问题

请在 [Issues](https://github.com/ruvideohub/zsdesk/issues) 中提供程序版本和版本类型、Windows 版本、触发步骤、预期结果和错误信息。右键问题同时注明菜单样式及相关第三方扩展。

修复提交、正式发布与反馈电脑上的复测分别记录；各项进展见[兼容性与反馈状态](docs/feedback-status.md)。

## 构建与许可证

主体使用 Rust 与 Win32/COM，文件浏览和文件操作复用 Windows Shell。构建步骤、依赖和打包参数见[使用与开发说明](docs/user-guide.md)。

采用 MIT 许可证；第三方组件许可见 [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt)。
