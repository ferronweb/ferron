; Ferron Inno Setup Script
#define MyAppName "Ferron"
#ifndef MyAppVersion
#define MyAppVersion "3.0.0"
#endif
#define MyAppPublisher "Ferron"
#define MyAppURL "https://ferron.sh"
#define MyAppSupportURL "https://ferron.sh/support"
#define MyAppExeName "ferron.exe"

#if "x86_64-pc-windows-msvc" == MyAppTargetTriple
#define MyAppSetupArchitecture "x64"
#define MyAppArchitecturesAllowed "x64compatible"
#define MyAppInstallIn64BitMode "x64compatible"
#elif "aarch64-pc-windows-msvc" == MyAppTargetTriple
#define MyAppSetupArchitecture "x64"
#define MyAppArchitecturesAllowed "arm64"
#define MyAppInstallIn64BitMode "arm64"
#else
#define MyAppSetupArchitecture "x86"
#define MyAppArchitecturesAllowed "x86compatible"
#define MyAppInstallIn64BitMode ""
#endif

[Setup]
AppId={{D1A3B0F5-9B7C-4C9D-A6F2-8A7E7C5A4B2E}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppSupportURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableWelcomePage=no
LicenseFile=..\..\LICENSE
OutputDir=..\..\dist
OutputBaseFilename=ferron-{#MyAppVersion}-{#MyAppTargetTriple}-setup
Compression=lzma
SolidCompression=yes
WizardStyle=modern dynamic
SetupIconFile=icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName} {#MyAppVersion}
WizardSmallImageFile=smallimage.png
WizardImageFile=image.png
;SetupArchitecture={#MyAppSetupArchitecture}
ArchitecturesAllowed={#MyAppArchitecturesAllowed}
ArchitecturesInstallIn64BitMode={#MyAppInstallIn64BitMode}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "path"; Description: "Add Ferron to the system PATH"; GroupDescription: "Additional tasks:"
Name: "service"; Description: "Install Ferron as a Windows service"; GroupDescription: "Additional tasks:"

[Files]
Source: "staging\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "staging\ferron-kdl2ferron.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "staging\ferron-passwd.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "staging\ferron-precompress.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "staging\ferron-serve.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "staging\wwwroot\*"; DestDir: "{app}\wwwroot"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "staging\ferron.conf"; DestDir: "{commonappdata}\Ferron"; Flags: ignoreversion onlyifdoesntexist

[Dirs]
Name: "{commonappdata}\Ferron"; Permissions: users-modify

[Run]
; Install the service if requested
Filename: "{app}\{#MyAppExeName}"; Parameters: "winservice install -c ""{commonappdata}\Ferron\ferron.conf"""; StatusMsg: "Installing Windows Service..."; Tasks: service; Flags: runhidden

[UninstallRun]
; Stop and uninstall the service
Filename: "{app}\{#MyAppExeName}"; Parameters: "winservice uninstall"; RunOnceId: "UninstallService"; Flags: runhidden

[Registry]
; Add to PATH
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; \
    ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; \
    Check: NeedsAddPath; Tasks: path

[Code]
function NeedsAddPath(): Boolean;
var
  OldPath: String;
begin
  if not RegQueryStringValue(HKEY_LOCAL_MACHINE, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', OldPath) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + UpperCase(ExpandConstant('{app}')) + ';', ';' + UpperCase(OldPath) + ';') = 0;
end;
