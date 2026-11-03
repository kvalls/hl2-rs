@echo off
setlocal
cd /d "%~dp0"
set "dist="
for %%a in (%*) do if /i "%%~a"=="--dist" set "dist=1"
if defined dist goto dist
if not exist "bin\hl2-bevy.exe" (
  echo Build with scripts\build-bevy.ps1 first.
  exit /b 1
)
"bin\hl2-bevy.exe" %*
goto finish

:dist
rem Builds with the plugins of the last build, then runs the game without the launcher's own argument.
set "plugins=advanced-options,debug-tools"
if exist "bin\build-bevy-info.json" for /f "usebackq delims=" %%p in (`powershell -NoProfile -Command "(Get-Content -Raw 'bin\build-bevy-info.json' | ConvertFrom-Json).plugins"`) do set "plugins=%%p"
powershell -NoProfile -ExecutionPolicy Bypass -File "scripts\build-bevy.ps1" -Dist -Plugins "%plugins%"
if errorlevel 1 (
  set "bevyExitCode=1"
  goto failed
)
set "gameArgs="
:collect
if "%~1"=="" goto run
if /i not "%~1"=="--dist" set gameArgs=%gameArgs% "%~1"
shift
goto collect
:run
"bin\hl2-bevy.exe" %gameArgs%

:finish
set "bevyExitCode=%errorlevel%"
:failed
if not "%bevyExitCode%"=="0" pause
exit /b %bevyExitCode%
