@echo off
rem ====================================================================
rem  BonDriver_Mirakurun environment setup script
rem
rem  Builds an MSVC-capable Rust environment using "winget configure".
rem
rem  Usage:
rem    setup-env.bat
rem
rem  This installs (via configuration.dsc.yaml):
rem    - Visual Studio 2022 Build Tools (VCTools workload + Windows SDK)
rem    - Rustup (Rust toolchain manager)
rem  and then sets up the x86_64-pc-windows-msvc Rust toolchain.
rem
rem  Note: winget configure may prompt for elevation (UAC) and asks you
rem  to review/agree to the configuration before it is applied.
rem ====================================================================
setlocal enabledelayedexpansion
cd /d "%~dp0"

set "DSC_FILE=configuration.dsc.yaml"

rem --- Check winget availability ---
where winget >nul 2>nul
if errorlevel 1 (
    echo [ERROR] winget not found.
    echo         Install "App Installer" from the Microsoft Store, then retry.
    exit /b 1
)

if not exist "%DSC_FILE%" (
    echo [ERROR] %DSC_FILE% not found next to this script.
    exit /b 1
)

rem --- Apply the DSC configuration ---
echo [INFO] Applying configuration: %DSC_FILE%
winget configure --file "%DSC_FILE%" --accept-configuration-agreements
if errorlevel 1 (
    echo [ERROR] winget configure failed.
    exit /b 1
)

rem --- Refresh PATH so rustup/cargo are visible in this session ---
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

where rustup >nul 2>nul
if errorlevel 1 (
    echo [WARN] rustup not on PATH yet. Open a new terminal and run:
    echo            rustup default stable-x86_64-pc-windows-msvc
    echo            rustup target add x86_64-pc-windows-msvc
    echo        Then build with: build.bat x86_64-pc-windows-msvc
    endlocal
    exit /b 0
)

rem --- Set up the MSVC Rust toolchain ---
echo [INFO] Installing MSVC toolchain and target ...
rustup toolchain install stable-x86_64-pc-windows-msvc || (echo [ERROR] toolchain install failed & exit /b 1)
rustup default stable-x86_64-pc-windows-msvc
rustup target add x86_64-pc-windows-msvc

echo.
echo [OK] MSVC Rust environment is ready.
echo      Build with: build.bat x86_64-pc-windows-msvc
endlocal
