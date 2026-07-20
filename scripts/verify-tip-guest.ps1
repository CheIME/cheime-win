# CheIME Guest Verification Script
# ==================================
# Delegates to sandbox\guest-run.ps1 for the full lifecycle.
# Kept as a compatibility entry point.

$ErrorActionPreference = 'Stop'

$guestRunner = Split-Path -Parent $MyInvocation.MyCommand.Path | Join-Path -ChildPath "..\sandbox\guest-run.ps1"
$guestRunner = (Resolve-Path $guestRunner).Path

if (-not (Test-Path $guestRunner)) {
    Write-Error "Guest runner not found at $guestRunner"
    exit 1
}

Write-Host "=== Delegate to sandbox\guest-run.ps1 ===" -ForegroundColor Cyan
& $guestRunner
exit $LASTEXITCODE
