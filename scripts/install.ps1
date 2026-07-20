# CheIME Windows install script
# Run from project root. Requires admin for HKLM registration.

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $projectRoot

$releaseDir = Join-Path $projectRoot "target\release"
$engineSource = Join-Path $releaseDir "cheime-engine.exe"
$dllSource = Join-Path $releaseDir "cheime_tip.dll"
$installerSource = Join-Path $releaseDir "cheime-installer.exe"
foreach ($artifact in @($engineSource, $dllSource, $installerSource)) {
    if (-not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
        throw "Build artifact not found: $artifact. Run 'cargo build --release' first."
    }
}

Write-Host "=== Installing CheIME ===" -ForegroundColor Cyan
$cheimeDir = Join-Path $env:LOCALAPPDATA "CheIME"
$binDir = Join-Path $cheimeDir "bin"
$dataDir = Join-Path $cheimeDir "data\dicts"
$configDir = Join-Path $cheimeDir "config"
New-Item -ItemType Directory -Force -Path $binDir, $dataDir, $configDir | Out-Null
Copy-Item -Force $engineSource $binDir
$dllPath = Join-Path $binDir "cheime-tip.dll"
Copy-Item -Force $dllSource $dllPath
Copy-Item -Force $installerSource $binDir
Copy-Item -Force (Join-Path $projectRoot "data\dicts\*") $dataDir

Write-Host "  Registering TIP DLL..." -ForegroundColor Yellow
& "$env:SystemRoot\System32\regsvr32.exe" /s $dllPath
if ($LASTEXITCODE -ne 0) { throw "regsvr32 failed with exit code $LASTEXITCODE for $dllPath" }

$clsid = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}"
$profile = "{D7E2A3B4-C5F6-7890-ABCD-EF1234567890}"
$inprocPath = "Registry::HKEY_CLASSES_ROOT\CLSID\$clsid\InprocServer32"
$registeredDll = (Get-ItemProperty -LiteralPath $inprocPath -Name "(default)")."(default)"
$threadingModel = (Get-ItemProperty -LiteralPath $inprocPath -Name "ThreadingModel").ThreadingModel
$profilePath = "Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\CTF\TIP\$clsid\LanguageProfile\0x00000804\$profile"
$description = (Get-ItemProperty -LiteralPath $profilePath -Name "Description").Description
if (-not [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($registeredDll), [IO.Path]::GetFullPath($dllPath))) { throw "Registered DLL path mismatch: expected '$dllPath', got '$registeredDll'" }
if ($threadingModel -ne "Apartment") { throw "ThreadingModel mismatch: expected 'Apartment', got '$threadingModel'" }
if ($description -ne "CheIME TIP") { throw "HKLM profile validation failed: expected 'CheIME TIP', got '$description'" }

Write-Host "  TIP registration verified" -ForegroundColor Green
Write-Host "=== Installation complete ===" -ForegroundColor Cyan
Write-Host "To start the engine: $binDir\cheime-engine.exe --dict-dir $dataDir"
Write-Host "To uninstall: .\scripts\uninstall.ps1"
