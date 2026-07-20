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

Write-Host "=== Uninstalling CheIME ===" -ForegroundColor Cyan
if (Test-Path -LiteralPath $dllPath -PathType Leaf) {
    $proc = Start-Process -FilePath "$env:SystemRoot\System32\regsvr32.exe" -ArgumentList @('/u', '/s', $dllPath) -Wait -PassThru -NoNewWindow
    if ($proc.ExitCode -ne 0) { throw "regsvr32 /u failed with exit code $($proc.ExitCode) for $dllPath" }
} else {
    Write-Host "  TIP DLL is absent; nothing can call DllUnregisterServer." -ForegroundColor Yellow
}

# Check against explicit HKLM views (not merged HKCR)
$clsid = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}"
$clsidKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Classes\CLSID\$clsid", $false)
$tipKey   = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$clsid", $false)
if ($clsidKey -ne $null) {
    $clsidKey.Close()
    throw "CLSID key still present after uninstall"
}
if ($tipKey -ne $null) {
    $tipKey.Close()
    throw "CTF TIP key still present after uninstall"
}

Write-Host "  Registration removed. Installed files remain at $cheimeDir for explicit removal." -ForegroundColor Green
Write-Host "=== Uninstallation complete ===" -ForegroundColor Cyan
