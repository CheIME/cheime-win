@echo off
cd /d "%~dp0"
powershell -NoExit -ExecutionPolicy Bypass -File "%~dp0guest-run.ps1"
