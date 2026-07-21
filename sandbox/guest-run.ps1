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

function Write-Phase {
    param([string]$Title, [int]$Num)
    Write-Host "`n=== Phase $Num : $Title ===" -ForegroundColor Cyan
}

# ── Phase config (no code, just display strings) ───────────────────────────
$phases = @(
    "Install bundle"
    "Register TIP"
    "Start engine"
    "Run COM probes"
    "Check event log"
    "Manual Notepad test"
    "Cleanup and exit"
)

# ── Guard ──────────────────────────────────────────────────────────────────
Assert-DisposableGuest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$instDir   = Join-Path $env:LOCALAPPDATA "CheIME"
$binDir    = Join-Path $instDir "bin"
$dataDir   = Join-Path $instDir "data\dicts"

$installedDll     = Join-Path $binDir "cheime-tip.dll"
$registeredProbe  = Join-Path $binDir "cheime-registered-probe.exe"
$profileProbe     = Join-Path $binDir "cheime-profile-probe.exe"
$engineExe        = Join-Path $binDir "cheime-engine.exe"

$engineProcess = $null
$enginePid     = $null
$cleanupErrors = @()

# ── Cleanup function (callable from multiple places) ────────────────────────
function Invoke-Cleanup {
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
            Write-Host "[cleanup] Engine already stopped (or kill failed)"
        }
    }

    # Unregister TIP DLL
    if (Test-Path $installedDll) {
        Write-Host "[cleanup] Unregistering TIP DLL..."
        try {
            Invoke-RegSvr32 -Action unregister -DllPath $installedDll
            Write-Host "[cleanup] TIP unregistered"
        } catch {
            Write-Host "[cleanup] Unregister failed (may already be unregistered): $_"
        }
    }

    # Verify registry cleanup
    Write-Host "[cleanup] Verifying registry cleanup..."
    $clsid = '{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}'
    $r1 = cmd /c "reg.exe delete HKLM\SOFTWARE\Classes\CLSID\$clsid /f 2>&1"
    $r2 = cmd /c "reg.exe delete HKLM\SOFTWARE\Microsoft\CTF\TIP\$clsid /f 2>&1"
    Write-Host "[cleanup] Registry cleanup attempted"

    # Remove installed files
    if (Test-Path $instDir) {
        try {
            Remove-Item -Recurse -Force $instDir -ErrorAction Stop
            Write-Host "[cleanup] Removed $instDir"
        } catch {
            Write-Host "[cleanup] Could not remove $instDir (files may be in use): $_"
        }
    }
}

# ═══════════════════════════════════════════════════════════════════════════
# Main: execute each phase.  On error, print it, run cleanup, and exit.
# ═══════════════════════════════════════════════════════════════════════════

try {
    # ── Phase 1: Copy bundle ──────────────────────────────────────────────
    Write-Phase -Title $phases[0] -Num 1

    if (-not (Test-Path (Join-Path $scriptDir "bin\cheime-tip.dll"))) {
        throw "Bundle not found at $scriptDir. The mapped folder should contain bin/, data/, etc."
    }

    New-Item -ItemType Directory -Force -Path $binDir, $dataDir | Out-Null

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

    $dictSrcDir = Join-Path $scriptDir "data\dicts"
    if (Test-Path $dictSrcDir) {
        Copy-Item -Force (Join-Path $dictSrcDir "*") $dataDir
        Write-Host "  [OK] Dictionary data copied"
    }

    foreach ($f in @($installedDll, $registeredProbe, $profileProbe, $engineExe)) {
        if (-not (Test-Path $f -PathType Leaf)) { throw "Missing installed file: $f" }
    }
    Write-Host "`n[guest] All files installed at $instDir"

    # ── Phase 2: Register TIP ────────────────────────────────────────────
    Write-Phase -Title $phases[1] -Num 2

    Invoke-RegSvr32 -Action register -DllPath $installedDll

    # Quick verification — warn but don't fail
    $clsid = '{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}'
    $verify = cmd /c "reg.exe query HKLM\SOFTWARE\Classes\CLSID\$clsid\InprocServer32 /ve 2>&1"
    if ($LASTEXITCODE -eq 0 -and $verify -match 'cheime-tip') {
        Write-Host "[guest] InprocServer32 verified via reg.exe"
    } else {
        Write-Host "[guest] regsvr32 reported OK — continuing (reg.exe verify optional)"
    }
    Write-Host "[guest] TIP registration complete"

    # ── Phase 3: Start engine ────────────────────────────────────────────
    Write-Phase -Title $phases[2] -Num 3

    $engineProcess = Start-Process -FilePath $engineExe -ArgumentList "--dict-dir", $dataDir -WindowStyle Hidden -PassThru
    $enginePid = $engineProcess.Id
    Write-Host "[guest] Engine started (PID: $enginePid)"

    Start-Sleep -Milliseconds 500
    if ($engineProcess.HasExited) {
        throw "Engine exited immediately after launch."
    }

    # ── Phase 4: Run probes ──────────────────────────────────────────────
    Write-Phase -Title $phases[3] -Num 4

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

    # ── Phase 5: Check event log ─────────────────────────────────────────
    Write-Phase -Title $phases[4] -Num 5

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
    Write-Phase -Title $phases[5] -Num 6

    Write-Host ""
    Write-Host "Open Notepad and test the IME:"
    Write-Host "  1. Switch to CheIME with Win+Space"
    Write-Host "  2. Type 'ni' — should see candidate window"
    Write-Host "  3. Press Space to commit the highlighted candidate"
    Write-Host "  4. Press Escape to cancel composition"
    Write-Host ""
    Write-Host "Type 'done' and press Enter when ready to clean up,"
    Write-Host "or just close this window to skip cleanup:"
    do {
        $input = Read-Host "`n> "
    } while ($input -ne 'done')
    Write-Host "[guest] Proceeding to cleanup..."

    # ── Phase 7: Post-test event log re-check ────────────────────────────
    Write-Host "`n--- Post-test event log check ---"
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

} catch {
    Write-Host ""
    Write-Host "==========================================" -ForegroundColor Red
    Write-Host "  ERROR: $_" -ForegroundColor Red
    Write-Host "==========================================" -ForegroundColor Red
} finally {
    Invoke-Cleanup
}

Write-Host ""
Write-Host "==========================================" -ForegroundColor Green
Write-Host "  GUEST RUN COMPLETED" -ForegroundColor Green
Write-Host "==========================================" -ForegroundColor Green
