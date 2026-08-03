# CheIME Windows uninstall script
# ================================
# SAFETY: Only runs inside Windows Sandbox (CHEIME_DISPOSABLE_GUEST=1).
# Uses Start-Process -PassThru for regsvr32 exit codes.
# Validates cleanup against explicit HKLM view (never merged HKCR).

$ErrorActionPreference = "Stop"

# ── Guard ──────────────────────────────────────────────────────────────────
if ($env:CHEIME_DISPOSABLE_GUEST -ne '1') {
    Write-Error "Refusing: set CHEIME_DISPOSABLE_GUEST=1 first. Use scripts\start-sandbox.ps1 instead."
    exit 2
}

$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $projectRoot
$cheimeDir = Join-Path $env:LOCALAPPDATA "CheIME"
$dllPath = Join-Path $cheimeDir "bin\cheime-tip.dll"
$x86DllPath = Join-Path $cheimeDir "bin\x86\cheime-tip.dll"

Write-Host "=== Uninstalling CheIME ===" -ForegroundColor Cyan

# Remove only CheIME's own per-user startup value. This is idempotent and
# leaves all unrelated Run entries untouched.
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
Remove-ItemProperty -LiteralPath $runKey -Name "CheIME Engine" -ErrorAction SilentlyContinue
Get-Process -Name "cheime-engine" -ErrorAction SilentlyContinue | Stop-Process

$registrations = @(
    [pscustomobject]@{ Exe = "$env:SystemRoot\SysWOW64\regsvr32.exe"; Dll = $x86DllPath },
    [pscustomobject]@{ Exe = "$env:SystemRoot\System32\regsvr32.exe"; Dll = $dllPath }
)
foreach ($registration in $registrations) {
    if (Test-Path -LiteralPath $registration.Dll -PathType Leaf) {
        $proc = Start-Process -FilePath $registration.Exe -ArgumentList @('/u', '/s', $registration.Dll) -Wait -PassThru -NoNewWindow
        if ($proc.ExitCode -ne 0) {
            throw "regsvr32 /u failed with exit code $($proc.ExitCode) for $($registration.Dll)"
        }
    } else {
        Write-Host "  TIP DLL is absent: $($registration.Dll)" -ForegroundColor Yellow
    }
}

# Check both explicit HKLM views (not merged HKCR).
$clsid = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}"
foreach ($view in @([Microsoft.Win32.RegistryView]::Registry64, [Microsoft.Win32.RegistryView]::Registry32)) {
    $baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::LocalMachine,
        $view
    )
    $clsidKey = $baseKey.OpenSubKey("SOFTWARE\Classes\CLSID\$clsid", $false)
    if ($clsidKey -ne $null) {
        $clsidKey.Close()
        $baseKey.Close()
        throw "CLSID key still present in $view after uninstall"
    }
    $baseKey.Close()
}
$tipKey   = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$clsid", $false)
if ($tipKey -ne $null) {
    $tipKey.Close()
    throw "CTF TIP key still present after uninstall"
}

Write-Host "  Registration removed. Installed files remain at $cheimeDir for explicit removal." -ForegroundColor Green
Write-Host "=== Uninstallation complete ===" -ForegroundColor Cyan
