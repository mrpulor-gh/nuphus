# on_session_start hook
# 会话开始时调用（异步）
# 参数: session_id user_input
param([string]$SessionId, [string]$UserInput)

$ErrorActionPreference = "SilentlyContinue"
$HookRoot = Split-Path -Parent $PSCommandPath
$timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss.fff"

$logDir = "$HookRoot\..\logs\sessions"
if (-not (Test-Path $logDir)) { New-Item -ItemType Directory -Path $logDir -Force | Out-Null }

$sessionLog = "$logDir\$SessionId.log"
$header = "========================================`nSESSION START: $timestamp`nSession ID: $SessionId`nUser Input: $UserInput`n========================================`n"
Add-Content -Path $sessionLog -Value $header

$activeFile = "$logDir\active_sessions.txt"
Add-Content -Path $activeFile -Value "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') | $SessionId | $UserInput"
exit 0
