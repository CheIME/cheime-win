@echo off
reg add "HKCU\Software\Microsoft\PowerShell\1\ShellIds\Microsoft.PowerShell" /v ExecutionPolicy /t REG_SZ /d RemoteSigned /f >nul 2>&1
set CHEIME_DISPOSABLE_GUEST=1
cd /d "%~dp0"
powershell -NoExit -ExecutionPolicy Bypass -File "%~dp0guest-run.ps1"
