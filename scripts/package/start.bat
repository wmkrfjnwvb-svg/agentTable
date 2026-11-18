@echo off
rem Starts agenttable from this folder and opens the browser.
cd /d "%~dp0"
agenttable.exe --open %*
pause
