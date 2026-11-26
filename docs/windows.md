# On Windows

`pacs-server.exe` runs as a Windows service: it starts with the machine, with nobody logged in,
runs under an account of its own, and starts again after a failure. The release's
`pacs-rs-<version>-windows-amd64.zip` holds the program; it needs no runtime.

Every command below runs in an administrator's PowerShell.

## Installing

The program in Program Files, its settings, index, files and log in one directory under
ProgramData:

```powershell
mkdir "C:\Program Files\pacs-rs", C:\ProgramData\pacs-rs
Copy-Item pacs-server.exe "C:\Program Files\pacs-rs\"
Set-Content C:\ProgramData\pacs-rs\pacs.toml 'listen = "0.0.0.0:8080"'
& "C:\Program Files\pacs-rs\pacs-server.exe" --install-service --config C:\ProgramData\pacs-rs\pacs.toml
```

`pacs.toml` takes every [setting](settings.md), by its name without `PACS_` and in lower case.
The file is checked before anything is installed: a mistake in it installs nothing. Relative
paths are taken from the file's directory, so the defaults put the index and files in
`C:\ProgramData\pacs-rs\data` and the log in `C:\ProgramData\pacs-rs\logs`, one file a day,
30 of them kept (`log_dir`, `log_days`).

The service is named `pacs-rs` (another name with `--service-name`, to run two). It runs as
the virtual account `NT SERVICE\pacs-rs`, which has no password and only the rights it is
given, not as LocalSystem. Give it the directories it writes to, then start it:

```powershell
icacls C:\ProgramData\pacs-rs /grant "NT SERVICE\pacs-rs:(OI)(CI)M"
sc.exe start pacs-rs
```

An index or files elsewhere (`index`, `storage` in `pacs.toml`, such as `file:D:/pacs`) need
the same `icacls` on their directory.

It then answers on `http://127.0.0.1:8080/healthz`, and appears in `services.msc` as pacs-rs.
A start that fails, on its settings or a port already taken, is said in the log, and the
service stops with error 1066 (`sc.exe query pacs-rs`); it is started again 10 seconds after a
failure, up to three times a day.

## Secrets

Secrets are never read from `pacs.toml` ([Settings](settings.md)). Put each in a file only
administrators and the service can read, and name the files in the service's environment:

```powershell
mkdir C:\ProgramData\pacs-rs\secrets
Set-Content -NoNewline C:\ProgramData\pacs-rs\secrets\auth '<at least 32 bytes>'
Set-Content -NoNewline C:\ProgramData\pacs-rs\secrets\admin '<the first administrator password>'
icacls C:\ProgramData\pacs-rs\secrets /inheritance:r /grant "Administrators:F" "NT SERVICE\pacs-rs:R"
reg add HKLM\SYSTEM\CurrentControlSet\Services\pacs-rs /v Environment /t REG_MULTI_SZ /d "PACS_AUTH_SECRET_FILE=C:\ProgramData\pacs-rs\secrets\auth\0PACS_ADMIN_PASSWORD_FILE=C:\ProgramData\pacs-rs\secrets\admin"
Restart-Service pacs-rs
```

`\0` separates the variables of `Environment`. The values themselves are not put there: the
service's registry key can be read by every user of the machine.

## The firewall

Windows asks nothing for a service; a port other machines reach needs a rule, as DIMSE does
when `dimse_listen = "0.0.0.0:11112"`:

```powershell
netsh advfirewall firewall add rule name="pacs-rs DIMSE" dir=in action=allow protocol=TCP localport=11112
```

## Upgrading and removing

Stop the service, replace `pacs-server.exe` at the same path, start it again: the service names
the program by its path. [Backing up and upgrading](backup.md) says what to save first.

```powershell
& "C:\Program Files\pacs-rs\pacs-server.exe" --uninstall-service
```

stops the service and removes it, and leaves the settings, index, files and log where they are.
