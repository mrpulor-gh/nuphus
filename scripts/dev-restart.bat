@echo off
REM dev-restart.bat — 后端代码变更后一键重建并重启 Nuphus（前端走 vite dev server 无需动）
REM 用法：双击运行或在终端执行。会强制关闭当前运行中的 Nuphus。
echo [1/3] 关闭运行中的 Nuphus...
taskkill /F /IM nuphus.exe >nul 2>&1
echo [2/3] 重建后端（cargo build）...
cd /d %~dp0..\src-tauri
cargo build
if errorlevel 1 (
  echo 构建失败，按任意键退出
  pause >nul
  exit /b 1
)
echo [3/3] 重启 Nuphus...
start "" "%~dp0..\target\debug\nuphus.exe"
echo 完成
