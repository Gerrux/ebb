; Ebb installer (Inno Setup 6). Built by installer\build.ps1, which passes the
; version from Cargo.toml:
;
;   ISCC /DAppVersion=0.1.0 installer\ebb.iss
;
; Per-user: no admin rights, installs into %LOCALAPPDATA%\Programs\Ebb, just like
; the data lives in %LOCALAPPDATA%\Ebb and autostart is a per-user logon task.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define AppExe "ebb.exe"

[Setup]
; Never change AppId: upgrades and uninstall find the installed copy by it.
AppId={{6E0B7C2A-4F1D-4B8E-9C3A-EBB000000001}
AppName=Ebb
AppVersion={#AppVersion}
AppVerName=Ebb {#AppVersion}
AppPublisher=Gerrux
AppPublisherURL=https://github.com/Gerrux/ebb
AppSupportURL=https://github.com/Gerrux/ebb/issues
AppUpdatesURL=https://github.com/Gerrux/ebb/releases
DefaultDirName={localappdata}\Programs\Ebb
DefaultGroupName=Ebb
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=..\dist
OutputBaseFilename=Ebb-Setup-{#AppVersion}
SetupIconFile=..\assets\ebb.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName=Ebb
VersionInfoVersion={#AppVersion}
VersionInfoProductName=Ebb
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
CloseApplications=no

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
ru.Autostart=Запускать Ebb при входе в Windows
en.Autostart=Start Ebb when I sign in to Windows
ru.Launch=Запустить Ebb
en.Launch=Launch Ebb

[Tasks]
Name: "autostart"; Description: "{cm:Autostart}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Ebb"; Filename: "{app}\{#AppExe}"
Name: "{userdesktop}\Ebb"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
; The logon task points at the exe that registers it, so it is registered from
; the installed copy. Unchecking the task on an upgrade turns autostart off.
Filename: "{app}\{#AppExe}"; Parameters: "--autostart-on"; Flags: runhidden waituntilterminated; Tasks: autostart
Filename: "{app}\{#AppExe}"; Parameters: "--autostart-off"; Flags: runhidden waituntilterminated; Tasks: not autostart
Filename: "{app}\{#AppExe}"; Description: "{cm:Launch}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\{#AppExe}"; Parameters: "--quit"; Flags: runhidden waituntilterminated; RunOnceId: "Quit"
Filename: "{app}\{#AppExe}"; Parameters: "--autostart-off"; Flags: runhidden waituntilterminated; RunOnceId: "Autostart"

; Notes in %LOCALAPPDATA%\Ebb are kept on uninstall: removing the program must
; not remove what the user wrote.

[Code]
// A running instance holds ebb.exe open; ask it to exit (the same request a
// second launch with --quit sends) before the file is replaced.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Exe: String;
  Code: Integer;
begin
  Result := '';
  Exe := ExpandConstant('{app}\{#AppExe}');
  if FileExists(Exe) then
  begin
    Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
    Sleep(1000);
  end;
end;
