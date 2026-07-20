# CheIME host-only build and staging script
# ===========================================
# SAFETY: This script NEVER touches the registry, never calls regsvr32,
# never starts the engine, and never inspects TSF state. It only builds
# release artifacts and copies them to a staging directory.
#
# Run from any directory — resolves the repo root via $PSScriptRoot.

param(
    [string]$StagingRoot = $(if (Test-Path "D:\tmp\ime_test") { "D:\tmp\ime_test_v2" } else { Join-Path $env:TEMP "cheime-stage" }),
    [switch]$SkipGates
)

$ErrorActionPreference = "Stop"

function Write-Step($msg) {
    Write-Host "`n=== $msg ===" -ForegroundColor Cyan
}

# Resolve repo root (parent of scripts/)
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

Write-Step "Building CheIME (release)"
Write-Host "Repo root: $repoRoot"
Write-Host "Staging root: $StagingRoot"

if (-not $SkipGates) {
    Write-Step "Gate: cargo fmt"
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw "cargo fmt failed" }

    Write-Step "Gate: cargo clippy"
    cargo clippy --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "clippy failed" }

    Write-Step "Gate: cargo test --workspace"
    cargo test --workspace
    if ($LASTEXITCODE -ne 0) { throw "tests failed" }
}

Write-Step "cargo build --release"
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "build failed" }

# Validate required artifacts
$releaseDir = Join-Path $repoRoot "target\release"
$artifacts = @{
    "cheime-engine.exe"       = Join-Path $releaseDir "cheime-engine.exe"
    "cheime_tip.dll"          = Join-Path $releaseDir "cheime_tip.dll"
    "cheime-registered-probe.exe" = Join-Path $releaseDir "cheime-registered-probe.exe"
    "cheime-profile-probe.exe"    = Join-Path $releaseDir "cheime-profile-probe.exe"
}

Write-Step "Validating release artifacts"
foreach ($name in $artifacts.Keys) {
    $path = $artifacts[$name]
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Missing artifact: $name at $path"
    }
    Write-Host "  [OK] $name ($( (Get-Item $path).Length / 1KB ) KB)"
}

# Stage bundle
Write-Step "Staging guest bundle"
$bundleDir = Join-Path $StagingRoot "cheime-bundle"
$binDir    = Join-Path $bundleDir "bin"
$dataDir   = Join-Path $bundleDir "data\dicts"

# Clean previous stage
if (Test-Path $bundleDir) { Remove-Item -Recurse -Force $bundleDir }
New-Item -ItemType Directory -Force -Path $binDir, $dataDir | Out-Null

# Copy binaries
Copy-Item -Force (Join-Path $releaseDir "cheime-engine.exe")          (Join-Path $binDir "cheime-engine.exe")
Copy-Item -Force (Join-Path $releaseDir "cheime_tip.dll")              (Join-Path $binDir "cheime-tip.dll")
Copy-Item -Force (Join-Path $releaseDir "cheime-registered-probe.exe") (Join-Path $binDir "cheime-registered-probe.exe")
Copy-Item -Force (Join-Path $releaseDir "cheime-profile-probe.exe")    (Join-Path $binDir "cheime-profile-probe.exe")

# Copy dictionary data
Copy-Item -Force (Join-Path $repoRoot "data\dicts\*") $dataDir

# Copy guest scripts
$sandboxDir = Join-Path $repoRoot "sandbox"
foreach ($guestScript in @("guest-run.ps1", "run.bat")) {
    $src = Join-Path $sandboxDir $guestScript
    if (Test-Path $src) {
        Copy-Item -Force $src (Join-Path $bundleDir $guestScript)
    }
}
if (Test-Path (Join-Path $sandboxDir "CheIME.wsb.template")) {
    Copy-Item -Force (Join-Path $sandboxDir "CheIME.wsb.template") (Join-Path $bundleDir "CheIME.wsb.template")
}

# Verify staged bundle
Write-Step "Verifying staged bundle"
$stagedFiles = @(
    "bin\cheime-engine.exe",
    "bin\cheime-tip.dll",
    "bin\cheime-registered-probe.exe",
    "bin\cheime-profile-probe.exe",
    "data\dicts\pinyin_small.dict.yaml"
)
foreach ($relPath in $stagedFiles) {
    $fullPath = Join-Path $bundleDir $relPath
    if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
        throw "Staged file missing: $relPath"
    }
    Write-Host "  [OK] $relPath"
}

Write-Host ""
Write-Host "==========================================" -ForegroundColor Green
Write-Host "  BUILD & STAGE COMPLETE" -ForegroundColor Green
Write-Host "  Bundle: $bundleDir" -ForegroundColor Green
Write-Host "==========================================" -ForegroundColor Green
Write-Host ""
Write-Host "To launch in Sandbox:" -ForegroundColor Yellow
Write-Host "  .\scripts\start-sandbox.ps1" -ForegroundColor Yellow
