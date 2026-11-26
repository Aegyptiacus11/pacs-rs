; The Windows installer of pacs-rs (Inno Setup 6): the program in Program Files, its settings,
; index, files and log in ProgramData\pacs-rs, and the Windows service of docs/windows.md.
;
;   ISCC.exe /DVersion=<version> /DStage=<directory with the files> /O<output dir> pacs-rs.iss
;
; Installing: the service is installed (pacs-server --install-service), its account given
; ProgramData\pacs-rs, and started. Installing again (an upgrade): the service is stopped before
; the files are replaced and started after; the settings and data stay. Uninstalling: the
; service is removed, the settings and data stay. A step that fails is said, and Setup exits 1.
; scripts/windows-installer-test.ps1 runs all of it.

#ifndef Version
  #error Version is not defined: ISCC /DVersion=<version>
#endif
#ifndef Stage
  #error Stage is not defined: ISCC /DStage=<directory with the files>
#endif
#define Service "pacs-rs"

[Setup]
; never changes: it is how Windows knows an upgrade from another program
AppId={{D2843E3C-D286-4688-A09C-19E0A7D3E60F}
AppName=pacs-rs
AppVersion={#Version}
AppPublisher=pacs-rs
AppPublisherURL=https://github.com/Aegyptiacus11/pacs-rs
AppSupportURL=https://github.com/Aegyptiacus11/pacs-rs/issues
DefaultDirName={autopf}\pacs-rs
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; the service is stopped by the code below, before the files are replaced
CloseApplications=no
OutputBaseFilename=pacs-rs-{#Version}-windows-amd64-setup
Compression=lzma2
SolidCompression=yes
UninstallDisplayName=pacs-rs {#Version}
UninstallDisplayIcon={app}\pacs-server.exe
WizardStyle=modern

[Files]
Source: "{#Stage}\pacs-server.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\echoscu.exe"; DestDir: "{app}\tools"; Flags: ignoreversion
Source: "{#Stage}\storescu.exe"; DestDir: "{app}\tools"; Flags: ignoreversion
Source: "{#Stage}\storescp.exe"; DestDir: "{app}\tools"; Flags: ignoreversion
Source: "{#Stage}\findscu.exe"; DestDir: "{app}\tools"; Flags: ignoreversion
Source: "{#Stage}\movescu.exe"; DestDir: "{app}\tools"; Flags: ignoreversion
Source: "{#Stage}\getscu.exe"; DestDir: "{app}\tools"; Flags: ignoreversion
Source: "{#Stage}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\README.md"; DestDir: "{app}"; Flags: ignoreversion
; the settings are the user's once written: never replaced, never removed
Source: "pacs.toml"; DestDir: "{commonappdata}\{#Service}"; Flags: onlyifdoesntexist uninsneveruninstall

[Code]
var
  Failed: Boolean;

{ Run a program hidden and wait; true when it exits 0. }
function Run(const Exe, Params: String): Boolean;
var
  Code: Integer;
begin
  Log('Running: ' + Exe + ' ' + Params);
  Result := Exec(Exe, Params, '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
  if not Result then
    Log(Format('Failed (exit code %d)', [Code]));
end;

{ Stop-Service and Start-Service wait until the service has stopped or runs, unlike sc.exe. }
function PowerShell(const Command: String): Boolean;
begin
  Result := Run(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    '-NoProfile -NonInteractive -Command "' + Command + '"');
end;

function ServiceExists: Boolean;
begin
  Result := Run(ExpandConstant('{sys}\sc.exe'), 'query {#Service}');
end;

function DataDir: String;
begin
  Result := ExpandConstant('{commonappdata}\{#Service}');
end;

procedure Fail(const Message: String);
begin
  Failed := True;
  Log('Error: ' + Message);
  SuppressibleMsgBox(Message, mbCriticalError, MB_OK, IDOK);
end;

{ An upgrade: the running service holds pacs-server.exe open. Stopped before anything changes. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if ServiceExists and not PowerShell('Stop-Service -Name {#Service} -ErrorAction Stop') then
    Result := 'The service {#Service} could not be stopped; nothing was changed.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Exe: String;
begin
  if CurStep <> ssPostInstall then
    Exit;
  Exe := ExpandConstant('{app}\pacs-server.exe');
  if not ServiceExists then
  begin
    if not Run(Exe, '--install-service --config "' + DataDir + '\pacs.toml"') then
    begin
      Fail('The service {#Service} could not be installed: check ' + DataDir + '\pacs.toml.');
      Exit;
    end;
    { the account exists once the service does }
    if not Run(ExpandConstant('{sys}\icacls.exe'),
      '"' + DataDir + '" /grant "NT SERVICE\{#Service}:(OI)(CI)M"') then
    begin
      Fail('The service account could not be given ' + DataDir + '.');
      Exit;
    end;
  end;
  if not PowerShell('Start-Service -Name {#Service} -ErrorAction Stop') then
    Fail('pacs-rs is installed, but its service did not start: the reason is in '
      + DataDir + '\logs. Correct ' + DataDir + '\pacs.toml, then: Start-Service {#Service}');
end;

function GetCustomSetupExitCode: Integer;
begin
  if Failed then
    Result := 1
  else
    Result := 0;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usUninstall) and ServiceExists then
    if not Run(ExpandConstant('{app}\pacs-server.exe'), '--uninstall-service') then
      { before any file is removed: the uninstall stops, and can be run again }
      RaiseException('The service {#Service} could not be removed; pacs-rs is left installed.');
end;
