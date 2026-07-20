# CheIME TSF registration diagnostics
param(
    [string]$DllPath = "$env:LOCALAPPDATA\CheIME\bin\cheime-tip.dll"
)

$ErrorActionPreference = 'Continue'

Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  CheIME Registration Diagnostics" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan

# 1. DLL info
Write-Host "`n[1] DLL" -ForegroundColor Yellow
if (Test-Path $DllPath) {
    $dll = Get-Item $DllPath
    Write-Host "  Path: $DllPath"
    Write-Host "  Size: $($dll.Length / 1KB) KB"
} else {
    Write-Host "  [ERROR] DLL not found: $DllPath"
}

# 2. regsvr32 test
Write-Host "`n[2] regsvr32" -ForegroundColor Yellow
$proc = Start-Process regsvr32.exe -ArgumentList "/s", $DllPath -Wait -PassThru -NoNewWindow
Write-Host "  Exit code: $($proc.ExitCode)"
if ($proc.ExitCode -eq 0) {
    Write-Host "  Status: S_OK" -ForegroundColor Green
} else {
    Write-Host "  Status: FAILED" -ForegroundColor Red
}

# 3. COM CLSID
Write-Host "`n[3] COM CLSID (HKLM)" -ForegroundColor Yellow
$clsid = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}"
$inprocKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Classes\CLSID\$clsid\InprocServer32", $false)
if ($inprocKey -ne $null) {
    $dllValue = $inprocKey.GetValue('')
    $threading = $inprocKey.GetValue('ThreadingModel')
    $inprocKey.Close()
    Write-Host "  InprocServer32: $dllValue"
    Write-Host "  ThreadingModel: $threading"
} else {
    Write-Host "  [WARN] HKLM CLSID not found" -ForegroundColor Red
    $hkcrKey = [Microsoft.Win32.Registry]::ClassesRoot.OpenSubKey("CLSID\$clsid\InprocServer32", $false)
    if ($hkcrKey -ne $null) {
        $dllValue2 = $hkcrKey.GetValue('')
        $hkcrKey.Close()
        Write-Host "  HKCR InprocServer32 (merged): $dllValue2"
    }
}

# 4. CTF TIP
Write-Host "`n[4] CTF TIP" -ForegroundColor Yellow
$ctfKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$clsid", $false)
if ($ctfKey -ne $null) {
    Write-Host "  CTF TIP key: present" -ForegroundColor Green
    $profileKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$clsid\LanguageProfile\0x00000804\{D7E2A3B4-C5F6-7890-ABCD-EF1234567890}", $false)
    if ($profileKey -ne $null) {
        $desc = $profileKey.GetValue('Description')
        Write-Host "  LanguageProfile Description: $desc"
        $profileKey.Close()
    } else {
        Write-Host "  [WARN] LanguageProfile subkey missing" -ForegroundColor Red
    }
    $ctfKey.Close()
} else {
    Write-Host "  [WARN] CTF TIP key not found in HKLM" -ForegroundColor Yellow
}

# 5. HKCU Enable
Write-Host "`n[5] EnableLanguageProfile (HKCU)" -ForegroundColor Yellow
$enablePath = "SOFTWARE\Microsoft\CTF\TIP\$clsid\LanguageProfile\0x00000804\{D7E2A3B4-C5F6-7890-ABCD-EF1234567890}"
$enableKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($enablePath, $false)
if ($enableKey -ne $null) {
    $enableVal = $enableKey.GetValue('Enable')
    Write-Host "  Enable = $enableVal"
    if ($enableVal -eq 1) {
        Write-Host "  Status: ENABLED" -ForegroundColor Green
    } elseif ($enableVal -eq 0) {
        Write-Host "  Status: DISABLED" -ForegroundColor Red
    } else {
        Write-Host "  [WARN] Enable value missing" -ForegroundColor Yellow
    }
    $enableKey.Close()
} else {
    Write-Host "  [WARN] HKCU profile key not found (EnableLanguageProfile may not have been called)" -ForegroundColor Yellow
}

# 6. TSF activation log
Write-Host "`n[6] TSF activation log (%TEMP%\cheime-tsf-log.txt)" -ForegroundColor Yellow
$logPath = "$env:TEMP\cheime-tsf-log.txt"
if (Test-Path $logPath) {
    Get-Content $logPath
} else {
    Write-Host "  (no log file - TIP has not been activated yet)"
}

Write-Host "`n============================================" -ForegroundColor Cyan
Write-Host "  Diagnostics complete" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
