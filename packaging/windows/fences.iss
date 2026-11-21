#ifndef SourceDir
#error SourceDir must point to a verified standalone Fences payload
#endif
#ifndef OutputDir
#error OutputDir must point to the installer output directory
#endif
#ifndef MyAppVersion
#error MyAppVersion is required
#endif
#if !FileExists(SourceDir + "\Fences.exe") || !FileExists(SourceDir + "\manifest.json")
#error The Fences executable or verified package manifest is missing
#endif

[Setup]
AppId={{F0A6F8B3-1A8A-4B81-9F59-1A84B0C7D45E}
AppName=Fences
AppVersion={#MyAppVersion}
AppVerName=Fences {#MyAppVersion}
AppPublisher=AuraUI contributors
AppPublisherURL=https://github.com/ruvideohub/zsdesk
AppSupportURL=https://github.com/ruvideohub/zsdesk/issues
AppUpdatesURL=https://github.com/ruvideohub/zsdesk/releases
DefaultDirName={localappdata}\Programs\Fences
DefaultGroupName=Fences
AllowNoIcons=yes
UsePreviousAppDir=yes
DisableWelcomePage=yes
DisableReadyPage=yes
DisableProgramGroupPage=yes
DisableDirPage=yes
OutputDir={#OutputDir}
OutputBaseFilename=Fences-{#MyAppVersion}-x64-setup
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.18362
SetupIconFile=..\..\crates\aura-fences\resources\aura-fences.ico
UninstallDisplayIcon={app}\Fences.exe
CloseApplications=yes
CloseApplicationsFilter=Fences.exe
RestartApplications=no
ShowLanguageDialog=auto
LanguageDetectionMethod=uilanguage
VersionInfoDescription=Fences Setup

[Languages]
Name: "chinesesimp"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "autostart"; Description: "{cm:FencesAutostart}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#SourceDir}\Fences.exe"; DestName: "Fences-update-helper.exe"; Flags: dontcopy
Source: "{#SourceDir}\msvcp140.dll"; Flags: dontcopy
Source: "{#SourceDir}\vcruntime140.dll"; Flags: dontcopy
Source: "{#SourceDir}\vcruntime140_1.dll"; Flags: dontcopy
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Fences"; Filename: "{app}\Fences.exe"; Check: not WizardNoIcons
Name: "{autodesktop}\Fences"; Filename: "{app}\Fences.exe"; Tasks: desktopicon; Check: not WizardNoIcons

[Run]
Filename: "{app}\Fences.exe"; Parameters: "--enable-startup"; Flags: runhidden waituntilterminated; Tasks: autostart
Filename: "{app}\Fences.exe"; Description: "{cm:LaunchFences}"; Flags: nowait postinstall skipifsilent

[CustomMessages]
chinesesimp.FencesAutostart=登录 Windows 时启动 Fences
english.FencesAutostart=Start Fences when I sign in to Windows
chinesesimp.LaunchFences=启动 Fences
english.LaunchFences=Launch Fences
chinesesimp.CloseFailed=Fences 仍在运行，请退出后重试安装。
english.CloseFailed=Fences is still running. Exit it and retry setup.
chinesesimp.StartupFailed=无法清理 Fences 登录启动设置；卸载已停止。
english.StartupFailed=Fences startup settings could not be removed; uninstall was stopped.

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ExitCode, I: Integer;
begin
  Result := '';
  if FileExists(ExpandConstant('{app}\Fences.exe')) then
  begin
    ExtractTemporaryFile('Fences-update-helper.exe');
    ExtractTemporaryFile('msvcp140.dll');
    ExtractTemporaryFile('vcruntime140.dll');
    ExtractTemporaryFile('vcruntime140_1.dll');
    if not Exec(ExpandConstant('{tmp}\Fences-update-helper.exe'), '--quit', ExpandConstant('{tmp}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) then
      Result := CustomMessage('CloseFailed')
    else if ExitCode <> 0 then Result := CustomMessage('CloseFailed')
    else
    begin
      for I := 1 to 40 do
      begin
        if not CheckForMutexes('Local\AuraFences.Application') then Break;
        Sleep(250);
      end;
      if CheckForMutexes('Local\AuraFences.Application') then Result := CustomMessage('CloseFailed');
    end;
  end;
end;

procedure RemoveLegacyDesktopStartup;
var
  Names: array[0..3] of String;
  I: Integer;
  Value, LowerValue: String;
begin
  Names[0] := 'AuraExplorer';
  Names[1] := 'AuraExplorer.Desktop';
  Names[2] := 'AuraExplorer.Theme.Desktop';
  Names[3] := 'AuraExplorer.DesktopBox.Desktop';
  for I := 0 to 3 do
  begin
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', Names[I], Value) then
    begin
      LowerValue := Lowercase(Value);
      if (Pos('auraexplorer.exe', LowerValue) > 0) and (Pos('--desktop', LowerValue) > 0) then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', Names[I]);
    end;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then RemoveLegacyDesktopStartup;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ExitCode: Integer;
begin
  if (CurUninstallStep = usUninstall) and FileExists(ExpandConstant('{app}\Fences.exe')) then
  begin
    if not Exec(ExpandConstant('{app}\Fences.exe'), '--uninstall-startup', ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) then
      RaiseException(CustomMessage('StartupFailed'));
    if ExitCode <> 0 then RaiseException(CustomMessage('StartupFailed'));
  end;
end;
