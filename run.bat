@echo off
rem ===========================================================================
rem  Ebb - launcher для Windows: проверка окружения + запуск.
rem  Двойной клик или: run.bat
rem ===========================================================================
chcp 65001 >nul
setlocal enabledelayedexpansion
cd /d "%~dp0"
title Ebb launcher

echo ==========================================================
echo   Ebb  -  проверка окружения и запуск
echo ==========================================================
echo.
echo [1/2] Компоненты...
echo.

set "MISSING="
call :check "Rust/cargo" "cargo --version" "https://rustup.rs  (toolchain MSVC)"

rem Inno Setup нужен только для установщика: без него dev и тесты работают.
set "INNO="
where ISCC.exe >nul 2>nul && set "INNO=1"
if exist "%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe" set "INNO=1"
if exist "%ProgramFiles%\Inno Setup 6\ISCC.exe" set "INNO=1"
if exist "%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" set "INNO=1"
if defined INNO (
  echo   [OK]   Inno Setup 6
) else (
  echo   [ - ]  Inno Setup 6 не найден - нужен только для установщика:
  echo          winget install JRSoftware.InnoSetup
)

echo.
if defined MISSING (
  echo ----------------------------------------------------------
  echo  Не хватает компонентов ^(см. [НЕТ] выше^). Установите их
  echo  по подсказкам и запустите run.bat снова.
  echo ----------------------------------------------------------
  echo.
  pause
  exit /b 1
)

rem Dev-экземпляр живёт отдельно от установленного Ebb: свой профиль
rem (настоящие заметки не трогаются) и своё имя экземпляра (свой мьютекс,
rem трей и окна - можно запускать рядом с рабочим Ebb).
set "DEVPROFILE=%CD%\target\dev-profile"

echo [2/2] Что запустить?
echo ==========================================================
echo     [1] Dev-сборка на тестовом профиле
echo     [2] Dev-сборка + egui inspection ^(egui MCP, 127.0.0.1:5731^)
echo     [3] Собрать установщик ^(dist\Ebb-Setup-*.exe + zip^)
echo     [4] Тесты ^(cargo test --release^)
echo ==========================================================
set "CHOICE="
set /p "CHOICE=Выбор [1]: "
if "%CHOICE%"=="" set "CHOICE=1"

if "%CHOICE%"=="1" goto :dev
if "%CHOICE%"=="2" goto :dev_inspect
if "%CHOICE%"=="3" goto :build
if "%CHOICE%"=="4" ( echo. & call cargo test --release --locked & goto :end )
echo Неизвестный выбор: %CHOICE%
goto :end

rem ---------------------------------------------------------------------------
:dev
echo.
call cargo build --locked
if errorlevel 1 ( echo   ОШИБКА сборки. & goto :end )
call :run_dev
goto :end

:dev_inspect
echo.
call cargo build --locked --features inspection
if errorlevel 1 ( echo   ОШИБКА сборки. & goto :end )
set "EGUI_INSPECTION=127.0.0.1:5731"
call :run_dev
goto :end

:run_dev
rem LOCALAPPDATA подменяется только здесь, после cargo: сборке он нужен настоящий.
if not exist "%DEVPROFILE%" mkdir "%DEVPROFILE%"
echo.
echo   профиль: %DEVPROFILE%\Ebb
echo   выход: закрыть окно или "Выход" в трее dev-экземпляра
echo.
set "LOCALAPPDATA=%DEVPROFILE%"
set "EBB_INSTANCE=dev"
target\debug\ebb.exe
exit /b 0

:build
if not defined INNO (
  echo.
  echo  Inno Setup 6 не найден. Установите: winget install JRSoftware.InnoSetup
  goto :end
)
echo. & echo Сборка установщика...
powershell -NoProfile -ExecutionPolicy Bypass -File "installer\build.ps1"
goto :end

rem ---------------------------------------------------------------------------
:check
rem %~1=имя  %~2=команда-версии  %~3=подсказка по установке
for /f "delims=" %%v in ('%~2 2^>nul') do (
  echo   [OK]   %~1: %%v
  exit /b 0
)
echo   [НЕТ]  %~1 - установка: %~3
set "MISSING=1"
exit /b 0

rem ---------------------------------------------------------------------------
:end
echo.
pause
exit /b 0
