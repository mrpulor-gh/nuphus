#!/bin/bash
# pre_tool_call hook
# 工具执行前调用
# 参数: tool_name json_params
# 返回: 0=允许执行, 非0=Veto（阻止执行）

TOOL_NAME="$1"
JSON_PARAMS="$2"
HOOK_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_DIR="$HOOK_DIR/../logs"
mkdir -p "$LOG_DIR"

# 危险工具黑名单
case "$TOOL_NAME" in
    execute_shell)
        # 危险关键词检查
        echo "$JSON_PARAMS" | grep -qE 'rm\s+-rf|del\s+/[sqf]|Format-Table|Format-List|ConvertTo-Json|\$env:|Set-ExecutionPolicy|New-Service|Stop-Computer|Restart-Computer'
        if [ $? -eq 0 ]; then
            echo "[PRE_HOOK] VETO: dangerous pattern in execute_shell params"
            exit 1
        fi
        ;;
    delete_file|rm)
        echo "[PRE_HOOK] VETO: tool '$TOOL_NAME' is in blacklist"
        exit 1
        ;;
esac

# 日志记录
echo "[$(date '+%Y-%m-%d %H:%M:%S.%3N')] PRE  $TOOL_NAME | $JSON_PARAMS" >> "$LOG_DIR/pre_tool_call.log"
echo "[PRE_HOOK] ALLOW: $TOOL_NAME"
exit 0
