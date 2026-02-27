#define MyAppName "dongshan"
#ifndef MyAppVersion
#define MyAppVersion "0.0.0-dev"
#endif
#ifndef MyAppExe
#define MyAppExe "target\release\dongshan.exe"
#endif

[Setup]
AppId={{C7AA56F4-51ED-4FA6-A95D-2210E242D901}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher=KonshinHaoshin
DefaultDirName={autopf}\dongshan
DefaultGroupName=dongshan
OutputBaseFilename=dongshan-setup-windows-x86_64
Compression=lzma
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
ChangesEnvironment=yes
DisableProgramGroupPage=yes
LicenseFile=LICENSE

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "addtopath"; Description: "Add dongshan to PATH (current user)"; GroupDescription: "Additional tasks:"; Flags: checkedonce

[Files]
Source: "{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\dongshan"; Filename: "{app}\dongshan.exe"
Name: "{group}\Uninstall dongshan"; Filename: "{uninstallexe}"

[Run]
Filename: "{app}\dongshan.exe"; Description: "Run dongshan --help"; Parameters: "--help"; Flags: nowait postinstall skipifsilent

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; \
    ValueData: "{olddata};{app}"; Tasks: addtopath; \
    Check: NeedsAddPath(ExpandConstant('{app}'))

[Code]
function NeedsAddPath(Param: string): boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', OrigPath) then
  begin
    Result := True;
    Exit;
  end;
  Result := Pos(';' + Uppercase(Param) + ';', ';' + Uppercase(OrigPath) + ';') = 0;
end;
