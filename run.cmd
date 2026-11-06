@echo off
rem Build the current code and start the workbench from app\ (see tools\run.ps1).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\run.ps1" %*
if errorlevel 1 pause
