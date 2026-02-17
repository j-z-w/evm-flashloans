@echo off
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0run_shadow_1h.ps1" %*
exit /b %errorlevel%
