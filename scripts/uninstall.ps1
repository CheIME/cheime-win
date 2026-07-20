# CheIME Windows uninstall script
# Requires admin because DllUnregisterServer removes HKLM TSF profile data.

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $projectRoot
$cheimeDir = Join-Path $env:LOCALAPPDATA "CheIME"
$dllPath = Join-Path $cheimeDir "bin\cheime-tip.dll"

Write-Host "=== Uninstalling CheIME ===" -ForegroundColor Cyan
if (Test-Path -LiteralPath $dllPath -PathType Leaf) {
    & "$env:SystemRoot\System32\regsvr32.exe" /s /u $dllPath
    if ($LASTEXITCODE -ne 0) { throw "regsvr32 /u failed with exit code $LASTEXITCODE for $dllPath" }
} else {
    Write-Host "  TIP DLL is absent; nothing can call DllUnregisterServer." -ForegroundColor Yellow
}

$clsid = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}"
$clsidPath = "Registry::HKEY_CLASSES_ROOT\CLSID\$clsid"
$profilePath = "Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\CTF\TIP\$clsid"
if ((Test-Path -LiteralPath $clsidPath) -or (Test-Path -LiteralPath $profilePath)) {
    throw "Registration remains after uninstall. Installed files were preserved for recovery."
}

Write-Host "  Registration removed. Installed files remain at $cheimeDir for explicit removal." -ForegroundColor Green
Write-Host "=== Uninstallation complete ===" -ForegroundColor Cyan
