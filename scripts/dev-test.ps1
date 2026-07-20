# CheIME dev-test script
# =======================
# WARNING: This script previously performed host registration and engine launch.
# All registration-capable operations now require Windows Sandbox isolation.
#
# Use .\scripts\start-sandbox.ps1 instead for safe one-click Sandbox testing.

Write-Host ""
Write-Host "==================================================" -ForegroundColor Yellow
Write-Host " DEV-TEST REDIRECTED TO SANDBOX" -ForegroundColor Yellow
Write-Host "==================================================" -ForegroundColor Yellow
Write-Host ""
Write-Host "Host registration is permanently prohibited for safety." -ForegroundColor Cyan
Write-Host ""
Write-Host "Use the Sandbox workflow instead:" -ForegroundColor Green
Write-Host "  .\scripts\start-sandbox.ps1" -ForegroundColor Green
Write-Host ""
Write-Host "This will:" -ForegroundColor White
Write-Host "  1. Build and validate release artifacts"
Write-Host "  2. Stage a guest bundle (no registry access)"
Write-Host "  3. Launch Windows Sandbox with the bundle"
Write-Host "  4. Inside Sandbox: register TIP, start engine, run probes"
Write-Host "  5. Guide you through a manual Notepad test"
Write-Host "  6. Clean up (unregister, stop engine, remove files)"
Write-Host ""
Write-Host "For isolated COM vtable probing (no registration):" -ForegroundColor White
Write-Host "  cargo run -p cheime-probe --release" -ForegroundColor Green
Write-Host ""
