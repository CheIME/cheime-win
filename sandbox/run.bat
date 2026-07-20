@echo off
set CHEIME_DISPOSABLE_GUEST=1
cd /d "%~dp0"
powershell -NoExit -ExecutionPolicy Bypass -File "%~dp0guest-run.ps1"
