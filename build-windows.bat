@echo off
rem Azeroth Survivors - one-click Windows build.
rem
rem Installs what is missing (the MSVC C++ build tools, Rust), fetches the WoW 1.12.1 game data
rem once, builds the desktop game and puts everything it needs into one folder:
rem
rem   Azeroth Survivors\warcraft-survivors.exe
rem   Azeroth Survivors\Data\                    the client's MPQs, ~5 GB
rem   Azeroth Survivors\discord-app-id.txt       optional, see README "Discord status"
rem
rem Re-runnable: every step skips what is already done, so after an update it only rebuilds.
rem Build into another folder (the game data is fetched or moved there):
rem
rem   build-windows.bat "F:\Games\Azeroth Survivors"
rem
rem WOW_DATA_URL overrides the download, as for scripts/fetch-data.sh.

setlocal EnableExtensions
cd /d "%~dp0"

set "GAME_DIR=%~dp0Azeroth Survivors"
if not "%~1"=="" set "GAME_DIR=%~1"
set "DATA_URL=https://archive.org/download/WoW_1.12.1-5875/1.12.1-5875_enUS.zip"
if defined WOW_DATA_URL set "DATA_URL=%WOW_DATA_URL%"
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

echo.
echo == Azeroth Survivors: one-click build
echo    game folder: %GAME_DIR%
echo.

rem --- 1. The C++ build tools: Rust links with them, and the Lua VM is C ---------------------
echo == [1/5] C++ build tools
call :has_msvc && goto msvc_ok
echo    Not found. Installing the Visual Studio 2022 Build Tools with the C++ workload, ~3 GB.
echo    Windows will ask for administrator rights.
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-source-agreements --accept-package-agreements --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
call :has_msvc && goto msvc_ok
echo.
echo    The C++ tools are still missing. Open the "Visual Studio Installer", click "Modify" on
echo    your Visual Studio or Build Tools, tick "Desktop development with C++", install, and run
echo    this file again.
goto fail
:msvc_ok
echo    ok
echo.

rem --- 2. Rust: rust-toolchain.toml pins the version, cargo fetches it on first use -----------
echo == [2/5] Rust
where cargo >nul 2>nul && goto rust_ok
echo    Not found. Installing rustup.
winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements
where cargo >nul 2>nul && goto rust_ok
echo    Rust did not install. Get it from https://rustup.rs and run this file again.
goto fail
:rust_ok
echo    ok
echo.

rem --- 3. Update: a git checkout pulls; a downloaded zip builds as it is ---------------------
echo == [3/5] Update
if not exist ".git" goto no_git
where git >nul 2>nul || goto no_git
git pull --ff-only || echo    Could not update automatically; building the files as they are.
goto update_done
:no_git
echo    Not a git checkout; building the files as they are.
:update_done
echo.

rem --- 4. The game data, once: a WoW 1.12.1 client's Data folder ----------------------------
echo == [4/5] Game data
if exist "%GAME_DIR%\Data\terrain.MPQ" if exist "%GAME_DIR%\Data\patch.MPQ" goto data_ok
rem scripts/fetch-data.sh leaves it in wow-data\Data: reuse that.
if exist "wow-data\Data\terrain.MPQ" goto data_move
if not exist "wow-data" mkdir "wow-data"
echo    Downloading a WoW 1.12.1 client, ~5.5 GB. An interrupted download resumes on the next run.
echo    %DATA_URL%
curl.exe -L -C - --fail -o "wow-data\client.zip" "%DATA_URL%"
if not exist "wow-data\client.zip" goto fail
echo    Extracting the MPQs.
tar.exe -xf "wow-data\client.zip" -C "wow-data" "Data/*.MPQ"
if errorlevel 1 goto fail_extract
del "wow-data\client.zip"
:data_move
rem robocopy, not move: move cannot carry a folder to another drive.
robocopy "wow-data\Data" "%GAME_DIR%\Data" /E /MOVE /NFL /NDL /NJH /NJS /NP >nul
if errorlevel 8 goto fail
rmdir "wow-data" 2>nul
:data_ok
echo    ok
echo.

rem --- 5. The game: a player build, which finds Data\ beside itself --------------------------
echo == [5/5] Building the game. The first build takes 10-30 minutes, later ones a few.
rem A running game holds its exe: the copy below would fail after the whole build.
tasklist /fi "imagename eq warcraft-survivors.exe" 2>nul | find /i "warcraft-survivors.exe" >nul
if not errorlevel 1 goto fail_running
cargo build --release -p warcraft-survivors --no-default-features
if errorlevel 1 goto fail
if not exist "%GAME_DIR%" mkdir "%GAME_DIR%"
copy /y "target\release\warcraft-survivors.exe" "%GAME_DIR%\" >nul
if errorlevel 1 goto fail
echo.

rem --- Optional: the Discord status, asked once ---------------------------------------------
if exist "%GAME_DIR%\discord-app-id.txt" goto discord_done
echo Optional: show your runs on your Discord profile. Paste your own Discord Application ID
echo (README, "Discord status"), or just press Enter to skip. It goes into discord-app-id.txt
echo in the game folder, where you can change it later.
set "DISCORD_ID="
set /p "DISCORD_ID=   Application ID: "
if defined DISCORD_ID (>"%GAME_DIR%\discord-app-id.txt" echo %DISCORD_ID%) else (type nul >"%GAME_DIR%\discord-app-id.txt")
echo.
:discord_done

echo == Done. Play with:
echo    "%GAME_DIR%\warcraft-survivors.exe"
echo.
choice /c YN /m "Start the game now"
if errorlevel 2 goto end
start "" /d "%GAME_DIR%" "%GAME_DIR%\warcraft-survivors.exe"
goto end

:fail_running
echo    The game is running. Close it and run this file again.
goto fail

:fail_extract
echo    Extracting failed; the download may be incomplete. Run this file again to resume it.
:fail
echo.
echo == FAILED. See the messages above.
pause
exit /b 1

:end
endlocal
exit /b 0

rem True when a Visual Studio or Build Tools install has the x64 C++ compiler.
:has_msvc
if not exist "%VSWHERE%" exit /b 1
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do exit /b 0
exit /b 1
