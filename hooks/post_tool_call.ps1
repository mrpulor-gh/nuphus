# post_tool_call hook
# 工具执行后调用（异步，不阻塞主流程）
# 参数: tool_name json_params json_result
param(
    [string]$ToolName,
    [string]$JsonParams,
    [string]$JsonResult
)

$ErrorActionPreference = "SilentlyContinue"
$script:HookRoot = Split-Path -Parent $PSCommandPath

# === 日志记录 ===
$timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss.fff"

# 解析结果判断成功/失败
$success = $JsonResult -match '"success":\s*true'
$status = if ($success) { " OK " } else { "FAIL" }

$logEntry = "[$timestamp] POST $status $ToolName | $JsonParams"
$logFile = "$HookRoot\..\logs\post_tool_call.log"

# 确保日志目录存在
$logDir = Split-Path -Parent $logFile
if (-not (Test-Path $logDir)) {
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
}

Add-Content -Path $logFile -Value $logEntry

# === 失败通知（写入通知队列）===
if (-not $success) {
    $notifyDir = "$HookRoot\..\logs\notifications"
    if (-not (Test-Path $notifyDir)) {
        New-Item -ItemType Directory -Path $notifyDir -Force | Out-Null
    }
    $notifyFile = "$notifyDir\$(Get-Date -Format 'yyyyMMdd_HHmmss')_${ToolName}_failure.json"
    
    $notify = @{
        timestamp = $timestamp
        tool = $ToolName
        params = $JsonParams
        result = $JsonResult
    } | ConvertTo-Json -Depth 3
    
    Add-Content -Path $notifyFile -Value $notify
}

Write-Host "[POST_HOOK] $status: $ToolName"
exit 0
