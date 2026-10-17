; Nock installer (Inno Setup 6). Per-user, no admin rights needed.
; Built by the release workflow:  ISCC.exe /DMyAppVersion=1.2.3 installer\nock.iss
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif

[Setup]
AppId={{A3F5D0C2-6B1E-4F7A-9C2D-5E8B7A1D3F40}
AppName=Nock
AppVersion={#MyAppVersion}
AppVerName=Nock {#MyAppVersion}
AppPublisher=Akhilesh Varute
AppPublisherURL=https://github.com/Akhilesh-Varute/Nock
AppSupportURL=https://github.com/Akhilesh-Varute/Nock/issues
DefaultDirName={localappdata}\Programs\Nock
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=Output
OutputBaseFilename=NockSetup-{#MyAppVersion}
SetupIconFile=..\assets\notch.ico
UninstallDisplayIcon={app}\nock.exe
UninstallDisplayName=Nock
LicenseFile=..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
VersionInfoVersion={#MyAppVersion}
VersionInfoProductName=Nock

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startup"; Description: "Start Nock when &Windows starts"; GroupDescription: "Options:"
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "..\target\release\nock.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; same file the app keeps up to date itself, so Windows search finds "Nock"
Name: "{autoprograms}\Nock"; Filename: "{app}\nock.exe"
Name: "{autodesktop}\Nock"; Filename: "{app}\nock.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Nock"; ValueData: """{app}\nock.exe"""; Flags: uninsdeletevalue; Tasks: startup

[Run]
Filename: "{app}\nock.exe"; Description: "Start Nock now"; Flags: nowait postinstall skipifsilent

[Code]
// Nock has no window to close, so stop it before replacing or removing the file.
procedure StopNock();
var
  rc: Integer;
begin
  Exec('taskkill.exe', '/F /IM nock.exe', '', SW_HIDE, ewWaitUntilTerminated, rc);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopNock();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    StopNock();
    // also remove the start-with-Windows entry if Nock's own tray menu created it
    RegDeleteValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Nock');
  end;
end;
