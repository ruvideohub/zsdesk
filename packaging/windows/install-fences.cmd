@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-fences.ps1" %*
exit /b %errorlevel%
