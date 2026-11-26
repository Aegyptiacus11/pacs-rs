# The Windows installer, run as a user would on a clean machine (an administrator's session,
# as on the release workflow's runner):
#   1. the port taken: Setup installs, the service does not start, Setup exits 1 and the log
#      says why;
#   2. Setup again, the port free: the service runs and answers; a study is stored;
#   3. Setup again, an upgrade: the service is stopped and started, the study is still there;
#   4. the uninstaller: the service and the program are gone, the settings and data stay.
#
#   pwsh scripts/windows-installer-test.ps1 -Setup dist\pacs-rs-<version>-windows-amd64-setup.exe
param([Parameter(Mandatory)][string]$Setup)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Data = 'C:\ProgramData\pacs-rs'
$App = 'C:\Program Files\pacs-rs'
$Url = 'http://127.0.0.1:8080'
$Sample = Join-Path $PSScriptRoot '..\docs\sample\00001.dcm'

if (-not (Test-Path $Setup)) { throw "no installer at $Setup" }
if (-not (Test-Path $Sample)) { throw "no sample at $Sample" }
if (Get-Service pacs-rs -ErrorAction SilentlyContinue) { throw 'a pacs-rs service exists already' }
if (Test-Path $Data) { throw "$Data exists already" }

$Run = 0
function Invoke-Setup {
    $script:Run += 1
    $log = Join-Path $env:RUNNER_TEMP "setup-$script:Run.log"
    $process = Start-Process -FilePath $Setup -Wait -PassThru `
        -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$log`""
    Write-Host "Setup $script:Run exited $($process.ExitCode); its log:"
    Get-Content $log | Select-Object -Last 15 | ForEach-Object { Write-Host "  $_" }
    $process.ExitCode
}

function Test-Healthy {
    try {
        (Invoke-WebRequest "$Url/healthz" -UseBasicParsing -TimeoutSec 5).StatusCode -eq 200
    } catch [System.Net.Http.HttpRequestException] {
        # nothing listening yet
        $false
    }
}

function Get-StudyCount {
    $studies = Invoke-RestMethod "$Url/studies" -TimeoutSec 10
    @($studies).Count
}

function Get-Log {
    if (-not (Test-Path "$Data\logs")) { return '' }
    (Get-ChildItem "$Data\logs" -File | Get-Content -Raw) -join "`n"
}

Write-Host '== 1. the port taken: the service does not start'
$taken = [System.Net.Sockets.TcpListener]::new([ipaddress]::Loopback, 8080)
$taken.Start()
try {
    $code = Invoke-Setup
    if ($code -ne 1) { throw "Setup exited $code with the port taken, not 1" }
    if (-not (Get-Service pacs-rs -ErrorAction SilentlyContinue)) { throw 'the service was not installed' }
    if (-not (Test-Path "$Data\pacs.toml")) { throw 'the settings were not written' }
    if ((Get-Log) -notmatch 'ERROR') { throw "no error in the log:`n$(Get-Log)" }
} finally {
    $taken.Stop()
}

Write-Host '== 2. again, the port free: it runs and stores'
$code = Invoke-Setup
if ($code -ne 0) { throw "Setup exited $code" }
if ((Get-Service pacs-rs).Status -ne 'Running') { throw 'the service is not running' }
if (-not (Test-Healthy)) { throw "no answer on $Url/healthz" }
$stored = Invoke-WebRequest "$Url/instances" -Method Post -InFile $Sample `
    -ContentType 'application/dicom' -UseBasicParsing
if ($stored.StatusCode -ne 200) { throw "storing the sample: $($stored.StatusCode)" }
if ((Get-StudyCount) -ne 1) { throw 'the stored study is not listed' }
# an edit of the settings must survive the upgrade
Add-Content "$Data\pacs.toml" '# kept'

Write-Host '== 3. an upgrade: stopped, replaced, started, nothing lost'
$code = Invoke-Setup
if ($code -ne 0) { throw "Setup exited $code on the upgrade" }
if ((Get-Service pacs-rs).Status -ne 'Running') { throw 'the service is not running after the upgrade' }
if (-not (Test-Healthy)) { throw "no answer on $Url/healthz after the upgrade" }
if ((Get-StudyCount) -ne 1) { throw 'the study is gone after the upgrade' }
if ((Get-Content "$Data\pacs.toml" -Raw) -notmatch '# kept') { throw 'the settings were replaced' }

Write-Host '== 4. the uninstaller: the service and program gone, the data kept'
$uninstaller = Join-Path $App 'unins000.exe'
if (-not (Test-Path $uninstaller)) { throw "no uninstaller at $uninstaller" }
# -Wait waits for the copy the uninstaller starts of itself, a child of it
$process = Start-Process -FilePath $uninstaller -Wait -PassThru `
    -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'
if ($process.ExitCode -ne 0) { throw "the uninstaller exited $($process.ExitCode)" }
$since = Get-Date
while (Test-Path "$App\pacs-server.exe") {
    if (((Get-Date) - $since).TotalSeconds -gt 60) { throw 'pacs-server.exe is still installed' }
    Start-Sleep -Milliseconds 500
}
if (Get-Service pacs-rs -ErrorAction SilentlyContinue) { throw 'the service is still installed' }
if (Test-Healthy) { throw 'something still answers on 8080' }
if (-not (Test-Path "$Data\pacs.toml")) { throw 'the settings were removed' }
if (-not (Test-Path "$Data\data\index.sqlite")) { throw 'the index was removed' }

Write-Host 'the installer: all four steps as expected'
