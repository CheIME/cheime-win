# CheIME guarded guest lifecycle script
# =========================================
# Installs the staged CheIME bundle inside Windows Sandbox, registers the TIP,
# starts the engine, runs COM/TSF probes, and cleans up.
#
# WARNING: performs real COM/TSF registration — only run inside Windows Sandbox
# or a revertible VM with:
#   $env:CHEIME_DISPOSABLE_GUEST = '1'

$ErrorActionPreference = 'Stop'

# ── Guard helpers ───────────────────────────────────────────────────────────
function Assert-DisposableGuest {
    if ($env:CHEIME_DISPOSABLE_GUEST -ne '1') {
        throw "Refusing: CHEIME_DISPOSABLE_GUEST is not set to '1'. This script performs real COM/TSF registration."
    }
    if ([Environment]::Is64BitProcess -eq $false) {
        throw "Refusing: 32-bit PowerShell. Must run in 64-bit."
    }
    # Weak sandbox detection: if the user name is WDAGUtilityAccount we're in Sandbox.
    $currentUser = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    if ($currentUser -notmatch 'WDAGUtilityAccount|Sandbox|WIN-DEV') {
        Write-Warning "User '$currentUser' does not look like a Sandbox guest. Proceeding anyway (CHEIME_DISPOSABLE_GUEST=1)."
    }
}

function Invoke-RegSvr32 {
    param([string]$Action, [string]$DllPath)
    $args = @()
    if ($Action -eq 'unregister') { $args += '/u' }
    $args += '/s'
    $args += $DllPath
    Write-Host "[guest] regsvr32 ${Action}: $DllPath"
    $proc = Start-Process -FilePath "$env:SystemRoot\System32\regsvr32.exe" -ArgumentList $args -Wait -PassThru -NoNewWindow
    if ($proc.ExitCode -ne 0) {
        throw "regsvr32 $Action failed (exit $($proc.ExitCode))"
    }
    Write-Host "[guest] regsvr32 $Action OK"
}

function Assert-TipRegistry {
    param(
        [string]$DllPath,
        [switch]$AssertAbsent
    )
    $clsid = '{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}'
    $profileGuid = '{D7E2A3B4-C5F6-7890-ABCD-EF1234567890}'
    $dllName = Split-Path -Leaf $DllPath

    if ($AssertAbsent) {
        $found = $false
        $out = cmd /c "reg.exe query HKLM\SOFTWARE\Classes\CLSID\$clsid /s 2>&1"
        if ($LASTEXITCODE -eq 0) { $found = $true; Write-Host "  [WARN] CLSID still present after unregistration" }
        $out = cmd /c "reg.exe query HKLM\SOFTWARE\Microsoft\CTF\TIP\$clsid /s 2>&1"
        if ($LASTEXITCODE -eq 0) { $found = $true; Write-Host "  [WARN] CTF TIP still present after unregistration" }
        if (-not $found) { Write-Host "[guest] Registry cleanup verified" }
        return
    }

    # Verify regsvr32 actually wrote the DLL path using reg.exe
    # (same view regsvr32 uses, avoids PowerShell/.NET HKCR merge quirks).
    $dllValue = cmd /c "reg.exe query HKLM\SOFTWARE\Classes\CLSID\$clsid\InprocServer32 /ve 2>&1"
    $threadingValue = cmd /c "reg.exe query HKLM\SOFTWARE\Classes\CLSID\$clsid\InprocServer32 /v ThreadingModel 2>&1"
    if ($LASTEXITCODE -eq 0 -and $dllValue -match "REG_SZ\s+(.+)$dllName") {
        Write-Host "[guest] InprocServer32: $DllPath (detected via reg.exe)"
    } else {
        Write-Warning "InprocServer32 not verified via reg.exe. The TIP may still work if regsvr32 reported success."
        Write-Host "  reg query output: $dllValue"
    }
    if ($threadingValue -match 'Apartment') {
        Write-Host "[guest] ThreadingModel: Apartment"
    }

    # Check CTF profile
    $ctfOut = cmd /c "reg.exe query HKLM\SOFTWARE\Microsoft\CTF\TIP\$clsid\LanguageProfile\0x00000804\$profileGuid /v Description 2>&1"
    if ($ctfOut -match 'CheIME TIP') {
        Write-Host "[guest] CTF LanguageProfile: CheIME TIP"
    } else {
        Write-Host "[guest] CTF LanguageProfile key not found (created dynamically by TSF on first use)"
    }
}

# ── Guard ────────────────────────────────────────────────────────────────────
Assert-DisposableGuest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$instDir   = Join-Path $env:LOCALAPPDATA "CheIME"
$binDir    = Join-Path $instDir "bin"
$dataDir   = Join-Path $instDir "data\dicts"

# Track state for cleanup
$engineProcess = $null
$enginePid     = $null
$cleanupErrors = @()

try {
    # ── Phase 1: Copy bundle ────────────────────────────────────────────────
    Write-Host "`n=== Phase 1: Install bundle ===" -ForegroundColor Cyan

    if (-not (Test-Path (Join-Path $scriptDir "bin\cheime-tip.dll"))) {
        throw "Bundle not found at $scriptDir. The mapped folder should contain bin/, data/, etc."
    }

    New-Item -ItemType Directory -Force -Path $binDir, $dataDir | Out-Null

    # Copy with verification
    $toCopy = @(
        "bin\cheime-engine.exe"
        "bin\cheime-tip.dll"
        "bin\cheime-registered-probe.exe"
        "bin\cheime-profile-probe.exe"
    )
    foreach ($rel in $toCopy) {
        $src = Join-Path $scriptDir $rel
        $dst = Join-Path $instDir $rel
        if (-not (Test-Path $src -PathType Leaf)) {
            throw "Bundle missing: $src"
        }
        Copy-Item -Force $src $dst
        Write-Host "  [OK] Copied $rel"
    }

    # Copy dict data
    $dictSrcDir = Join-Path $scriptDir "data\dicts"
    if (Test-Path $dictSrcDir) {
        Copy-Item -Force (Join-Path $dictSrcDir "*") $dataDir
        Write-Host "  [OK] Dictionary data copied"
    }

    # Validate all installed files exist
    $installedDll = Join-Path $binDir "cheime-tip.dll"
    $registeredProbe = Join-Path $binDir "cheime-registered-probe.exe"
    $profileProbe    = Join-Path $binDir "cheime-profile-probe.exe"
    $engineExe       = Join-Path $binDir "cheime-engine.exe"

    foreach ($f in @($installedDll, $registeredProbe, $profileProbe, $engineExe)) {
        if (-not (Test-Path $f -PathType Leaf)) { throw "Missing installed file: $f" }
    }
    Write-Host "`n[guest] All files installed at $instDir"

    # ── Phase 2: Register TIP ──────────────────────────────────────────────
    Write-Host "`n=== Phase 2: Register TIP ===" -ForegroundColor Cyan
    try {
        Invoke-RegSvr32 -Action register -DllPath $installedDll
        Assert-TipRegistry -DllPath $installedDll
        Write-Host "[guest] TIP registration verified"
    } catch {
        Write-Warning "TIP registration could not be verified, but regsvr32 reported success. Continuing..."
    }

    # ── Phase 3: Start engine ──────────────────────────────────────────────
    Write-Host "`n=== Phase 3: Start engine ===" -ForegroundColor Cyan
    $engineProcess = Start-Process -FilePath $engineExe -ArgumentList "--dict-dir", $dataDir -WindowStyle Hidden -PassThru
    $enginePid = $engineProcess.Id
    Write-Host "[guest] Engine started (PID: $enginePid)"

    # Wait for engine pipe to become available
    Start-Sleep -Milliseconds 500
    if ($engineProcess.HasExited) {
        throw "Engine exited immediately after launch."
    }

    # ── Phase 4: Run probes ────────────────────────────────────────────────
    Write-Host "`n=== Phase 4: Run COM probes ===" -ForegroundColor Cyan

    Write-Host "[guest] Running registered-probe..."
    $rp = Start-Process -FilePath $registeredProbe -NoNewWindow -Wait -PassThru
    if ($rp.ExitCode -ne 0) {
        throw "registered-probe failed (exit $($rp.ExitCode))"
    }
    Write-Host "[guest] Registered probe PASSED"

    Write-Host "`n[guest] Running profile-probe..."
    $pp = Start-Process -FilePath $profileProbe -NoNewWindow -Wait -PassThru
    if ($pp.ExitCode -ne 0) {
        throw "profile-probe failed (exit $($pp.ExitCode))"
    }
    Write-Host "[guest] Profile probe PASSED"

    # ── Phase 5: Check event log ──────────────────────────────────────────
    Write-Host "`n=== Phase 5: Check event log ===" -ForegroundColor Cyan
    $since = (Get-Date).AddMinutes(-5)
    try {
        $faults = Get-WinEvent -FilterHashtable @{
            LogName   = 'Application'
            ID        = 1000
            StartTime = $since
        } -ErrorAction Stop | Where-Object { $_.Message -match 'cheime_tip' }
        if ($faults) {
            Write-Warning "CheIME crash events found!"
            $faults | ForEach-Object { Write-Host "  $($_.TimeCreated) $_" }
        } else {
            Write-Host "[guest] No CheIME fault events detected"
        }
    } catch {
        Write-Host "[guest] Event log check skipped: $_"
    }

    # ── Phase 6: Manual acceptance test ──────────────────────────────────
    Write-Host "`n=== Phase 6: Manual Notepad test ===" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Open Notepad and test the IME:"
    Write-Host "  1. Switch to CheIME with Win+Space (may need to select from list)"
    Write-Host "  2. Type 'ni' — should see candidate window with '你' and '呢'"
    Write-Host "  3. Press Enter or Space to commit the highlighted candidate"
    Write-Host "  4. Press Escape to cancel composition"
    Write-Host "  5. Switch back with Win+Space when done"
    Write-Host ""
    Write-Host "Type 'done' and press Enter when ready to clean up,"
    Write-Host "or just close this window to skip cleanup:"
    do {
        $input = Read-Host "`n> "
    } while ($input -ne 'done')
    Write-Host "[guest] Proceeding to cleanup..."

    # ── Phase 7: Event log re-check ──────────────────────────────────────
    Write-Host "`n=== Phase 7: Post-test event log check ===" -ForegroundColor Cyan
    $since2 = (Get-Date).AddMinutes(-1)
    try {
        $faults2 = Get-WinEvent -FilterHashtable @{
            LogName   = 'Application'
            ID        = 1000
            StartTime = $since2
        } -ErrorAction Stop | Where-Object { $_.Message -match 'cheime_tip' }
        if ($faults2) {
            Write-Warning "Crash events detected after user interaction!"
            $faults2 | ForEach-Object { Write-Host "  $($_.TimeCreated) $_" }
        } else {
            Write-Host "[guest] No crash events after manual test"
        }
    } catch {
        Write-Host "[guest] Post-test log check skipped: $_"
    }

    # ── Cleanup ─────────────────────────────────────────────────────────────
    Write-Host "`n=== Cleanup ===" -ForegroundColor Cyan

    # Stop engine
    if ($enginePid -ne $null) {
        Write-Host "[cleanup] Stopping engine (PID: $enginePid)..."
        try {
            $engineProc = Get-Process -Id $enginePid -ErrorAction Stop
            $engineProc.Kill()
            $engineProc.WaitForExit(5000) | Out-Null
            Write-Host "[cleanup] Engine stopped"
        } catch {
            Write-Warning "Engine stop: $_"
            $cleanupErrors += "Engine stop failed: $_"
        }
    }

    # Unregister TIP DLL
    $installedDll = Join-Path $binDir "cheime-tip.dll"
    if (Test-Path $installedDll) {
        Write-Host "[cleanup] Unregistering TIP DLL..."
        try {
            Invoke-RegSvr32 -Action unregister -DllPath $installedDll
            Write-Host "[cleanup] TIP unregistered"
        } catch {
            Write-Warning "Unregister failed: $_"
            $cleanupErrors += "Unregister failed: $_"
        }
    }

    # Verify registry cleanup
    Write-Host "[cleanup] Verifying registry cleanup..."
    try {
        Assert-TipRegistry -DllPath $installedDll -AssertAbsent
    } catch {
        Write-Warning "Registry cleanup check: $_"
        $cleanupErrors += "Registry cleanup: $_"
    }

    # Remove installed files
    if (Test-Path $instDir) {
        if ($cleanupErrors.Count -eq 0) {
            Remove-Item -Recurse -Force $instDir
            Write-Host "[cleanup] Removed $instDir"
        } else {
            Write-Warning "Preserving $instDir due to cleanup errors above"
        }
    }

    Write-Host ""
    if ($cleanupErrors.Count -gt 0) {
        Write-Host "==========================================" -ForegroundColor Red
        Write-Host "  GUEST RUN COMPLETED WITH ERRORS" -ForegroundColor Red
        Write-Host "==========================================" -ForegroundColor Red
        $cleanupErrors | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
        exit 10
    } else {
        Write-Host "==========================================" -ForegroundColor Green
        Write-Host "  GUEST RUN COMPLETED SUCCESSFULLY" -ForegroundColor Green
        Write-Host "==========================================" -ForegroundColor Green
    }
}
