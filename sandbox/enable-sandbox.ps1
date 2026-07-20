@echo off
:: Set up the Sandbox for easy PowerShell script execution
:: Run this once at the start of each Sandbox session.
reg add "HKCU\Software\Microsoft\PowerShell\1\ShellIds\Microsoft.PowerShell" /v ExecutionPolicy /t REG_SZ /d RemoteSigned /f >nul 2>&1
set CHEIME_DISPOSABLE_GUEST=1
echo Powershell execution policy set to RemoteSigned.
echo CHEIME_DISPOSABLE_GUEST=1 set.
echo.
echo You can now run scripts directly, e.g.:
echo   .\guest-run.ps1
echo   .\diagnose.ps1
