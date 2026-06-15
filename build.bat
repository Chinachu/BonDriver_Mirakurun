@echo off
rem ====================================================================
rem  BonDriver_Mirakurun build script
rem
rem  Usage:
rem    build.bat              build with the default target
rem    build.bat <target>     build with the given Rust target
rem
rem  Default target: x86_64-pc-windows-msvc (MSVC).
rem  On a machine with the LLVM-MinGW toolchain you can use:
rem    build.bat x86_64-pc-windows-gnu
rem
rem  Output: dist\BonDriver_Mirakurun.dll and dist\BonDriver_Mirakurun.ini
rem ====================================================================
setlocal enabledelayedexpansion
cd /d "%~dp0"

rem Decide the target (default: msvc / MSVC).
set "TARGET=%~1"
if "%TARGET%"=="" set "TARGET=x86_64-pc-windows-msvc"

rem Matching toolchain name
set "TOOLCHAIN=stable-%TARGET%"

echo [INFO] target    = %TARGET%
echo [INFO] toolchain = %TOOLCHAIN%

rem Pin the toolchain by putting its bin directory at the front of PATH.
rem This avoids picking up a different (e.g. standalone MSVC) cargo on PATH.
where rustup >nul 2>nul
if not errorlevel 1 (
    rustup toolchain list | findstr /C:"%TOOLCHAIN%" >nul 2>nul
    if errorlevel 1 (
        echo [INFO] Installing toolchain %TOOLCHAIN% ...
        rustup toolchain install %TOOLCHAIN% || (echo [ERROR] toolchain install failed & exit /b 1)
    )
    rustup run %TOOLCHAIN% rustup target add %TARGET% >nul 2>nul
)

set "TCBIN=%USERPROFILE%\.rustup\toolchains\%TOOLCHAIN%\bin"
if exist "%TCBIN%\cargo.exe" (
    echo [INFO] Using toolchain bin: %TCBIN%
    set "PATH=%TCBIN%;%PATH%"
) else (
    echo [WARN] %TCBIN% not found. Using cargo on PATH.
)

echo [INFO] cargo build --release --target %TARGET%
cargo build --release --target %TARGET%
if errorlevel 1 (
    echo [ERROR] build failed.
    exit /b 1
)

rem Copy artifacts to the dist folder
if not exist "dist" mkdir "dist"
copy /Y "target\%TARGET%\release\BonDriver_Mirakurun.dll" "dist\BonDriver_Mirakurun.dll" >nul
if errorlevel 1 (
    echo [ERROR] failed to copy DLL.
    exit /b 1
)
copy /Y "BonDriver_Mirakurun.ini" "dist\BonDriver_Mirakurun.ini" >nul

echo.
echo [OK] Build complete: dist\BonDriver_Mirakurun.dll
endlocal
