#define MyAppName "dongshan"
#ifndef MyAppVersion
#define MyAppVersion "0.0.0-dev"
#endif
#ifndef MyAppExe
#define MyAppExe "..\..\target\release\dongshan.exe"
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
LicenseFile=..\..\LICENSE

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

function RemovePathEntry(PathValue, Entry: string): string;
var
  Work, Search, EntryNeedle: string;
  P: Integer;
begin
  Work := ';' + PathValue + ';';
  Search := ';' + Uppercase(PathValue) + ';';
  EntryNeedle := ';' + Uppercase(Entry) + ';';

  while True do
  begin
    P := Pos(EntryNeedle, Search);
    if P = 0 then
      Break;
    Delete(Work, P, Length(Entry) + 1);
    Delete(Search, P, Length(Entry) + 1);
  end;

  while Pos(';;', Work) > 0 do
    StringChangeEx(Work, ';;', ';', True);

  if (Length(Work) > 0) and (Copy(Work, 1, 1) = ';') then
    Delete(Work, 1, 1);
  if (Length(Work) > 0) and (Copy(Work, Length(Work), 1) = ';') then
    Delete(Work, Length(Work), 1);

  Result := Work;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  PathValue, NewPath: string;
begin
  if CurUninstallStep <> usUninstall then
    Exit;

  if RegQueryStringValue(HKCU, 'Environment', 'Path', PathValue) then
  begin
    NewPath := RemovePathEntry(PathValue, ExpandConstant('{app}'));
    if NewPath <> PathValue then
      RegWriteExpandStringValue(HKCU, 'Environment', 'Path', NewPath);
  end;
end;
