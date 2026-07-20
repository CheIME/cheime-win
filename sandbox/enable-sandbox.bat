@echo off
reg add "HKCU\Software\Microsoft\PowerShell\1\ShellIds\Microsoft.PowerShell" /v ExecutionPolicy /t REG_SZ /d RemoteSigned /f >nul 2>&1
set CHEIME_DISPOSABLE_GUEST=1
echo.
echo Execution policy set to RemoteSigned.
echo CHEIME_DISPOSABLE_GUEST=1 set.
echo.
echo You can now run scripts directly, e.g.:
echo   diagnose.ps1
echo   guest-run.ps1
