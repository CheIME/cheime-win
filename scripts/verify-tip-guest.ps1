# CheIME Level 3 Guest Verification Script
# WARNING: runs real COM/TSF registration. Only execute in Windows Sandbox
# or a revertible VM with:
#   $env:CHEIME_DISPOSABLE_GUEST = '1'

$ErrorActionPreference = 'Continue'

if ($env:CHEIME_DISPOSABLE_GUEST -ne '1') {
    Write-Error 'Refusing registration: set CHEIME_DISPOSABLE_GUEST=1.'
    exit 2
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$dll = Join-Path $scriptDir 'cheime_tip.dll'
$registeredProbe = Join-Path $scriptDir 'cheime-registered-probe.exe'
$profileProbe = Join-Path $scriptDir 'cheime-profile-probe.exe'

foreach ($f in @($dll, $registeredProbe, $profileProbe)) {
    if (-not (Test-Path $f)) {
        Write-Error "Missing artifact: $f"
        exit 3
    }
}

$dllHash = (Get-FileHash -Algorithm SHA256 $dll).Hash
$expectedHash = 'F6A03524E1F63F5BF2E8313A4FE850C6621B0F65B301B051998347B054E5BAF1'
Write-Host "[guest] DLL SHA-256: $dllHash"
Write-Host "[guest] Expected  : $expectedHash"
if ($dllHash -ne $expectedHash) {
    Write-Host "WARNING: DLL hash differs from Level 2 baseline."
}

# ---- Phase 1: Register the TIP ----
Write-Host "[guest] Registering TIP DLL..."
$regProcess = Start-Process -FilePath "$env:SystemRoot\System32\regsvr32.exe" -ArgumentList @('/s', $dll) -Wait -PassThru
if ($regProcess.ExitCode -ne 0) {
    Write-Error "regsvr32 failed with exit code $($regProcess.ExitCode)"
    exit 4
}
Write-Host "[guest] regsvr32 OK"

$clsid = '{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}'
$inprocKey = "Registry::HKEY_CLASSES_ROOT\CLSID\$clsid\InprocServer32"
if (-not (Test-Path $inprocKey)) {
    Write-Error "InprocServer32 not found after registration"
    exit 5
}
$registeredPath = (Get-ItemProperty $inprocKey).'(default)'
Write-Host "[guest] InprocServer32: $registeredPath"

# ---- Phase 2: Registered COM probe ----
Write-Host "[guest] Running registered COM probe..."
$proc = Start-Process -FilePath $registeredProbe -NoNewWindow -Wait -PassThru
if ($proc.ExitCode -ne 0) {
    Write-Error "registered-probe failed: $($proc.ExitCode)"
    exit 6
}
Write-Host "[guest] registered-probe PASSED"

# ---- Phase 3: Process-scoped profile probe ----
Write-Host "[guest] Running profile probe..."
$proc = Start-Process -FilePath $profileProbe -NoNewWindow -Wait -PassThru
if ($proc.ExitCode -ne 0) {
    Write-Error "profile-probe failed: $($proc.ExitCode)"
    exit 7
}
Write-Host "[guest] profile-probe PASSED"

# ---- Phase 4: Check event log ----
Write-Host "[guest] Checking Application log for CheIME faults..."
$since = (Get-Date).AddMinutes(-10)
$faults = Get-WinEvent -FilterHashtable @{
    LogName = 'Application'
    ID = 1000
    StartTime = $since
} -ErrorAction SilentlyContinue | Where-Object {
    $_.Message -match 'cheime_tip'
}
if ($faults) {
    Write-Host "WARNING: CheIME crash events found:"
    $faults | ForEach-Object { Write-Host "  $($_.TimeCreated) $($_.Message)" }
} else {
    Write-Host "[guest] No CheIME crash events detected"
}

# ---- Phase 5: Unregister ----
Write-Host "[guest] Unregistering TIP DLL..."
$unregProcess = Start-Process -FilePath "$env:SystemRoot\System32\regsvr32.exe" -ArgumentList @('/u', '/s', $dll) -Wait -PassThru
if ($unregProcess.ExitCode -ne 0) {
    Write-Host "WARNING: regsvr32 /u returned $($unregProcess.ExitCode)"
} else {
    Write-Host "[guest] Unregistration OK"
}

if (Test-Path $inprocKey) {
    Write-Host "WARNING: InprocServer32 still present after unregistration"
} else {
    Write-Host "[guest] CLSID key removed"
}

Write-Host ''
Write-Host '========================================='
Write-Host '    GUEST VERIFICATION COMPLETE'
Write-Host '========================================='
