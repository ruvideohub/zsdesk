; Установщик AuraUI для Inno Setup 6 (ISCC.exe).
;
; Собирается только из проверенного набора: scripts/package.ps1 сначала раскладывает папку,
; пишет manifest.json и прогоняет scripts/verify-bundle.ps1, и лишь потом зовёт этот файл.
; Поэтому здесь нет списка «изданий» и нет условий по редакциям: в AuraUI один набор, и он
; описан в packaging/bundle.json. Если файла из набора нет, компилятор остановится ниже,
; а не соберёт установщик без половины программ.
;
;   ISCC.exe /DSourceDir=<папка набора> /DOutputDir=<куда> /DMyAppVersion=<версия> aura.iss

#ifndef SourceDir
#error SourceDir must point to a verified AuraUI payload
#endif
#ifndef OutputDir
#define OutputDir "."
#endif
#ifndef MyAppVersion
#error MyAppVersion is required
#endif

#define ProductName "AuraUI"
#define ProductDirectory "AuraUI"
#define ProductAppId "{{B7C0F4C2-6E1B-4E52-9E4E-6A0F1C2D3A41}"

#if !FileExists(SourceDir + "\AuraExplorer.exe") || !FileExists(SourceDir + "\AuraDock.exe") || !FileExists(SourceDir + "\Fences.exe") || !FileExists(SourceDir + "\widgets.exe") || !FileExists(SourceDir + "\auraservice.exe") || !FileExists(SourceDir + "\AuraArc.exe") || !FileExists(SourceDir + "\manifest.json")
#error The verified AuraUI payload is incomplete: AuraExplorer.exe, AuraDock.exe, Fences.exe, widgets.exe, auraservice.exe, AuraArc.exe and manifest.json are required
#endif

[Setup]
AppId={#ProductAppId}
AppName={#ProductName}
AppVersion={#MyAppVersion}
AppVerName={#ProductName} {#MyAppVersion}
AppPublisher=AuraUI contributors
AppPublisherURL=https://github.com/ruvideohub/zsdesk
AppSupportURL=https://github.com/ruvideohub/zsdesk/issues
AppUpdatesURL=https://github.com/ruvideohub/zsdesk/releases
DefaultDirName={localappdata}\Programs\{#ProductDirectory}
DefaultGroupName={#ProductName}
AllowNoIcons=yes
UsePreviousAppDir=yes
DisableProgramGroupPage=yes
DisableWelcomePage=yes
OutputDir={#OutputDir}
OutputBaseFilename=AuraUI-{#MyAppVersion}-x64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.18362
SetupIconFile=..\..\crates\aura-explorer\resources\aura.ico
UninstallDisplayIcon={app}\AuraExplorer.exe
VersionInfoDescription={#ProductName} {#MyAppVersion} Setup
; Программы закрываются перед установкой штатным механизмом Windows (Restart Manager),
; а не «поверх запущенных»: иначе exe занят и файл не заменится.
CloseApplications=yes
CloseApplicationsFilter=AuraExplorer.exe,AuraDock.exe,Fences.exe,widgets.exe,auraservice.exe,AuraArc.exe
RestartApplications=no

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
; Одна запись автозапуска — служба; что именно она поднимает, решается в её окне настроек.
Name: "autostart"; Description: "Запускать AuraUI при входе в Windows"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
; Программы: каждая — отдельный exe, все лежат рядом, потому что служба ищет их по имени.
Source: "{#SourceDir}\AuraExplorer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\AuraDock.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\Fences.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\widgets.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\auraservice.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\AuraArc.exe"; DestDir: "{app}"; Flags: ignoreversion
; Перехват окон проводника: имя с хешем, грузится по имени из папки программы.
Source: "{#SourceDir}\AuraExplorerWindowHook-*.dll"; DestDir: "{app}"; Flags: ignoreversion
; Опечатка сборки и установка без установщика: ими проверяются и, при желании, копируются
; файлы вручную — поэтому они тоже часть установленной папки.
Source: "{#SourceDir}\manifest.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\install.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\install.cmd"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\install-readme.txt"; DestDir: "{app}"; Flags: ignoreversion
; Лицензии: комплект, а не один файл — по ним видно, из чего собраны зависимости.
Source: "{#SourceDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\AuraExplorer"; Filename: "{app}\AuraExplorer.exe"
Name: "{group}\Fences — desktop grids"; Filename: "{app}\Fences.exe"
Name: "{group}\AuraUI — настройки и запуск"; Filename: "{app}\auraservice.exe"; Parameters: "--settings"; Comment: "Что запускать: компоненты, автозапуск, журнал"
Name: "{group}\AuraArc — архиватор"; Filename: "{app}\AuraArc.exe"
Name: "{group}\Удалить AuraUI"; Filename: "{uninstallexe}"
Name: "{autodesktop}\AuraExplorer"; Filename: "{app}\AuraExplorer.exe"; Tasks: desktopicon

[Registry]
; Единственный писатель автозапуска — установщик и служба. Одна запись: `auraservice --run`.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "AuraExplorer"; ValueData: """{app}\auraservice.exe"" --run"; Flags: uninsdeletevalue; Tasks: autostart
; Прежние записи отдельных компонентов: их работу делает служба, а лишняя запись подняла бы
; вторую панель или сетки без владельца.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "AuraDock"; Flags: deletevalue
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "AuraExplorer.Desktop"; Flags: deletevalue
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "AuraFences.Desktop"; Flags: deletevalue

[Run]
; В общей установке запуском Fences владеет служба, не отдельная запись Run.
; Удаляем оставшийся запуск этой копии, если ранее включали standalone-autostart.
Filename: "{app}\Fences.exe"; Parameters: "--uninstall-startup"; Flags: runhidden waituntilterminated
; Служба поднимает включённые компоненты сразу, не дожидаясь следующего входа в Windows.
Filename: "{app}\auraservice.exe"; Parameters: "--run"; Description: "Запустить AuraUI"; Flags: nowait postinstall skipifsilent
Filename: "{app}\auraservice.exe"; Parameters: "--settings"; Description: "Открыть настройки: что запускать"; Flags: nowait postinstall skipifsilent unchecked

[UninstallRun]
; Служба останавливается первой: иначе она поднимет то, что установщик только что убрал.
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM auraservice.exe /IM AuraDock.exe /IM Fences.exe /IM widgets.exe /IM AuraArc.exe"; Flags: runhidden skipifdoesntexist; RunOnceId: "AuraUIStop"
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM AuraExplorer.exe"; Flags: runhidden skipifdoesntexist; RunOnceId: "AuraUIStopFiles"

[Code]
{ Перед установкой AuraExplorer кооперативно закрывает свои окна; Restart Manager закрывает
  независимые программы, включая Fences и AuraService. Манифест проверен упаковщиком. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ExitCode: Integer;
  Helper: String;
begin
  Result := '';
  Helper := ExpandConstant('{app}\AuraExplorer.exe');
  if not FileExists(Helper) then
    exit;
  if not Exec(Helper, '--shutdown-for-update "' + Helper + '"', ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    Result := 'Не удалось закрыть запущенные программы AuraUI. Закрой их и повтори установку.'
  else if ExitCode <> 0 then
    Result := 'Программы AuraUI не закрылись. Закрой окна и повтори установку.';
end;
