# pre_tool_call hook
# 工具执行前调用
# 参数: tool_name json_params
# 返回: 0=允许执行, 非0=Veto（阻止执行）
param(
    [string]$ToolName,
    [string]$JsonParams
)

$ErrorActionPreference = "SilentlyContinue"
$script:HookRoot = Split-Path -Parent $PSCommandPath

# === 危险操作黑名单 ===
$dangerous_tools = @{
    "execute_shell" = $true   # shell执行需要额外检查参数
    "delete_file"   = $true
    "rm"            = $true
}

# 危险关键词（参数中包含则veto）
$dangerous_patterns = @(
    "Format-Table",
    "Format-List", 
    "ConvertTo-Json",
    "rm\s+-rf",
    "del\s+/[sqf]",
    "\$env:",
    "Set-ExecutionPolicy",
    "New-Service",
    "Stop-Computer",
    "Restart-Computer"
)

if ($dangerous_tools.ContainsKey($ToolName)) {
    # execute_shell 需要参数安全检查
    if ($ToolName -eq "execute_shell") {
        foreach ($pattern in $dangerous_patterns) {
            if ($JsonParams -match $pattern) {
                Write-Host "[PRE_HOOK] VETO: dangerous pattern '$pattern' in execute_shell params"
                exit 1
            }
        }
    } else {
        Write-Host "[PRE_HOOK] VETO: tool '$ToolName' is in blacklist"
        exit 1
    }
}

# === 日志记录 ===
$timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss.fff"
$logEntry = "[$timestamp] PRE  $ToolName | $JsonParams"
$logFile = "$HookRoot\..\logs\pre_tool_call.log"

# 确保日志目录存在
$logDir = Split-Path -Parent $logFile
if (-not (Test-Path $logDir)) {
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
}

Add-Content -Path $logFile -Value $logEntry

Write-Host "[PRE_HOOK] ALLOW: $ToolName"
exit 0
