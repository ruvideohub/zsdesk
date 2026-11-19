@echo off
rem Double-click helper: runs install.ps1 with the execution policy bypassed.
rem   install.cmd              install for the current user and start AuraExplorer
rem   install.cmd -Remove      uninstall (autostart, shortcuts, folder)
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
if errorlevel 1 echo.
if errorlevel 1 echo AuraExplorer install finished with an error (see the messages above).
echo.
pause
