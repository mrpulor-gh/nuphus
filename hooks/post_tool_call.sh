#!/bin/bash
# post_tool_call hook
# 工具执行后调用
# 参数: tool_name json_params json_result
# 用于日志、监控、通知等

TOOL_NAME="$1"
JSON_PARAMS="$2"
JSON_RESULT="$3"
HOOK_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_DIR="$HOOK_DIR/../logs"
mkdir -p "$LOG_DIR"

echo "[$(date '+%Y-%m-%d %H:%M:%S.%3N')] POST $TOOL_NAME | $JSON_PARAMS | $JSON_RESULT" >> "$LOG_DIR/post_tool_call.log"
exit 0
