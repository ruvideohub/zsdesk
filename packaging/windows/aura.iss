; Standalone AuraDock and its desktop widgets; Fences has a separate installer.
#ifndef SourceDir
#error SourceDir must point to a verified AuraUI payload
#endif
#ifndef OutputDir
#define OutputDir "."
#endif
#ifndef MyAppVersion
#error MyAppVersion is required
#endif
#if !FileExists(SourceDir + "\AuraDock.exe") || !FileExists(SourceDir + "\widgets.exe") || !FileExists(SourceDir + "\manifest.json")
#error AuraDock.exe, widgets.exe and manifest.json are required
#endif

[Setup]
AppId={{A84A62E5-9820-4618-B7B9-2358BE8F2AF8}
AppName=AuraDock
AppVersion={#MyAppVersion}
AppVerName=AuraDock {#MyAppVersion}
AppPublisher=AuraUI contributors
AppPublisherURL=https://github.com/ruvideohub/zsdesk
AppSupportURL=https://github.com/ruvideohub/zsdesk/issues
DefaultDirName={localappdata}\Programs\AuraDock
DefaultGroupName=AuraDock
AllowNoIcons=yes
UsePreviousAppDir=yes
DisableProgramGroupPage=yes
DisableWelcomePage=yes
OutputDir={#OutputDir}
OutputBaseFilename=AuraDock-{#MyAppVersion}-x64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.18362
SetupIconFile=..\..\crates\aura-dock\resources\aura-dock.ico
UninstallDisplayIcon={app}\AuraDock.exe
CloseApplications=yes
CloseApplicationsFilter=AuraDock.exe,widgets.exe
RestartApplications=no

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "autostart"; Description: "Start Dock when I sign in to Windows"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#SourceDir}\AuraDock.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\widgets.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\manifest.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\install.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\install.cmd"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\install-readme.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\AuraDock"; Filename: "{app}\AuraDock.exe"
Name: "{group}\AuraDock Settings"; Filename: "{app}\AuraDock.exe"; Parameters: "--settings"
Name: "{group}\Uninstall AuraDock"; Filename: "{uninstallexe}"
Name: "{autodesktop}\AuraDock"; Filename: "{app}\AuraDock.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\AuraDock.exe"; Description: "Launch Dock"; Flags: nowait postinstall skipifsilent

[Code]
const RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
const RunName = 'AuraDock';

function DockCommand: String;
begin
  Result := '"' + ExpandConstant('{app}\AuraDock.exe') + '"';
end;

function OwnDockEntry: Boolean;
var Value: String;
begin
  Result := RegQueryStringValue(HKCU, RunKey, RunName, Value) and
    (CompareText(Trim(Value), DockCommand) = 0);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var Value: String;
begin
  Result := '';
  { Never reuse a directory belonging to the old combined product or another Fences install. }
  if FileExists(ExpandConstant('{app}\Fences.exe')) or
     (CompareText(ExpandConstant('{app}'), ExpandConstant('{localappdata}\Programs\AuraUI')) = 0) then
  begin
    Result := 'Choose a separate AuraDock folder. The old combined installation and Fences are not modified.';
    exit;
  end;
  { Previous Explorer versions could own folder-opening registry values. Never delete or
    overwrite their binary before their own uninstaller has restored those associations. }
  if FileExists(ExpandConstant('{app}\AuraExplorer.exe')) or
     FileExists(ExpandConstant('{app}\auraservice.exe')) or
     (RegQueryStringValue(HKCU, RunKey, 'AuraExplorer', Value) and
       (Pos('auraservice.exe', Lowercase(Value)) > 0)) or
     RegKeyExists(HKCU, 'Software\Classes\Directory\shell\AuraExplorer') or
     RegKeyExists(HKCU, 'Software\Classes\Drive\shell\AuraExplorer') or
     RegKeyExists(HKCU, 'Software\Classes\Directory\Background\shell\AuraExplorer') or
     RegKeyExists(HKCU, 'Software\Classes\CLSID\{20D04FE0-3AEA-1069-A2D8-08002B30309D}\shell\AuraExplorer') then
  begin
    Result := 'Older AuraExplorer integration was found. Back up your profiles and restore folder integration using the old version. Verify Windows Explorer works before removing old files. Then uninstall the old version and retry.';
    exit;
  end;
  { Never overwrite a different installation's entry, even when autostart is unchecked. }
  if RegValueExists(HKCU, RunKey, RunName) and not OwnDockEntry then
    Result := 'The AuraDock startup entry belongs to another installation. It was not changed.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    if WizardIsTaskSelected('autostart') then
    begin
      if (RegValueExists(HKCU, RunKey, RunName) and not OwnDockEntry) or
         not RegWriteStringValue(HKCU, RunKey, RunName, DockCommand) then
        MsgBox('Could not configure Dock startup. The existing startup entry was not replaced.', mbError, MB_OK);
    end
    else if OwnDockEntry then
      if not RegDeleteValue(HKCU, RunKey, RunName) then
        MsgBox('Could not remove this Dock startup entry.', mbError, MB_OK);
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usUninstall) and OwnDockEntry then
    if not RegDeleteValue(HKCU, RunKey, RunName) then
      MsgBox('Could not remove this Dock startup entry.', mbError, MB_OK);
end;
