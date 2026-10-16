@echo off
cargo build --release
if errorlevel 1 exit /b 1
echo.
echo Built: %~dp0target\release\nock.exe
