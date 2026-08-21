# on_session_end hook
# 会话结束时调用（异步）
# 参数: session_id success output
param([string]$SessionId, [string]$Success, [string]$Output)

$ErrorActionPreference = "SilentlyContinue"
$HookRoot = Split-Path -Parent $PSCommandPath
$timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss.fff"

$logDir = "$HookRoot\..\logs\sessions"
if (-not (Test-Path $logDir)) { New-Item -ItemType Directory -Path $logDir -Force | Out-Null }

$sessionLog = "$logDir\$SessionId.log"
$footer = "`n========================================`nSESSION END: $timestamp | Success=$Success`nOutput: $Output`n========================================`n"
Add-Content -Path $sessionLog -Value $footer

# 更新活跃会话索引（标记结束）
$activeFile = "$logDir\active_sessions.txt"
if (Test-Path $activeFile) {
    $content = Get-Content $activeFile -Raw
    $content = $content -replace "^(.*\| $SessionId \|.*)$", "$1 | ENDED=$timestamp"
    Set-Content -Path $activeFile -Value $content
}
exit 0
