# CheIME Windows install script
# ==============================
# SAFETY: Only runs inside Windows Sandbox (CHEIME_DISPOSABLE_GUEST=1).
# Uses Start-Process -PassThru for regsvr32 exit codes.
# Validates registration against explicit HKLM view (never merged HKCR).

$ErrorActionPreference = "Stop"

# ── Guard ──────────────────────────────────────────────────────────────────
if ($env:CHEIME_DISPOSABLE_GUEST -ne '1') {
    Write-Error "Refusing: set CHEIME_DISPOSABLE_GUEST=1 first. Use scripts\start-sandbox.ps1 instead."
    exit 2
}

# Resolve paths
$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $projectRoot
$releaseDir = Join-Path $projectRoot "target\release"

$engineSource = Join-Path $releaseDir "cheime-engine.exe"
$uiSource     = Join-Path $releaseDir "cheime-ui.exe"
$dllSource    = Join-Path $releaseDir "cheime_tip.dll"
foreach ($artifact in @($engineSource, $uiSource, $dllSource)) {
    if (-not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
        throw "Build artifact not found: $artifact. Run 'scripts\build.ps1' first."
    }
}

Write-Host "=== Installing CheIME ===" -ForegroundColor Cyan
$cheimeDir = Join-Path $env:LOCALAPPDATA "CheIME"
$binDir    = Join-Path $cheimeDir "bin"
$dataDir   = Join-Path $cheimeDir "data\dicts"
$configDir = Join-Path $cheimeDir "config"
New-Item -ItemType Directory -Force -Path $binDir, $dataDir, $configDir | Out-Null
Copy-Item -Force $engineSource $binDir
Copy-Item -Force $uiSource $binDir
$dllPath = Join-Path $binDir "cheime-tip.dll"
Copy-Item -Force $dllSource $dllPath
Copy-Item -Force (Join-Path $projectRoot "assets\windows\*.ico") $binDir
Copy-Item -Force (Join-Path $projectRoot "data\dicts\*") $dataDir
$uiConfig = Join-Path $configDir "ui.yaml"
if (-not (Test-Path -LiteralPath $uiConfig -PathType Leaf)) {
    Copy-Item -Force (Join-Path $projectRoot "config\ui.yaml") $uiConfig
}

Write-Host "  Registering TIP DLL..." -ForegroundColor Yellow
$proc = Start-Process -FilePath "$env:SystemRoot\System32\regsvr32.exe" -ArgumentList @('/s', $dllPath) -Wait -PassThru -NoNewWindow
if ($proc.ExitCode -ne 0) { throw "regsvr32 failed with exit code $($proc.ExitCode) for $dllPath" }

# Validate against explicit HKLM view (not merged HKCR)
$clsid = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}"
$profile = "{D7E2A3B4-C5F6-7890-ABCD-EF1234567890}"
$inprocKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Classes\CLSID\$clsid\InprocServer32", $false)
if ($inprocKey -eq $null) { throw "InprocServer32 not found in HKLM view after registration" }
$registeredDll = $inprocKey.GetValue('')
$threadingModel = $inprocKey.GetValue('ThreadingModel')
$inprocKey.Close()
if (-not [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($registeredDll), [IO.Path]::GetFullPath($dllPath))) { throw "Registered DLL path mismatch: expected '$dllPath', got '$registeredDll'" }
if ($threadingModel -ne "Apartment") { throw "ThreadingModel mismatch: expected 'Apartment', got '$threadingModel'" }

$profileKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$clsid\LanguageProfile\0x00000804\$profile", $false)
if ($profileKey -ne $null) {
    $description = $profileKey.GetValue('Description')
    $profileKey.Close()
    if ($description -ne 'CheIME TIP') { throw "Profile description mismatch: expected 'CheIME TIP', got '$description'" }
}

Write-Host "  TIP registration verified" -ForegroundColor Green

# Start the per-user engine at sign-in. Quote both paths because LOCALAPPDATA
# and dictionary directories may contain spaces. New-ItemProperty -Force makes
# repeated installs update the same value instead of creating duplicates.
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$enginePath = Join-Path $binDir "cheime-engine.exe"
$engineCommand = "`"$enginePath`" --dict-dir `"$dataDir`""
New-ItemProperty -LiteralPath $runKey -Name "CheIME Engine" -Value $engineCommand -PropertyType String -Force | Out-Null
Write-Host "  Per-user engine startup registered" -ForegroundColor Green

# Make the freshly installed input method usable without requiring sign-out.
if (-not (Get-Process -Name "cheime-engine" -ErrorAction SilentlyContinue)) {
    Start-Process -FilePath $enginePath -ArgumentList @("--dict-dir", $dataDir) -WindowStyle Hidden
    Write-Host "  Engine started" -ForegroundColor Green
}

Write-Host "=== Installation complete ===" -ForegroundColor Cyan
Write-Host "To edit the UI config: $binDir\cheime-ui.exe"
Write-Host "To uninstall: .\scripts\uninstall.ps1"
