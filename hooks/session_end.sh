#!/bin/bash
# session_end hook
# 会话结束时调用
# 参数: session_id success output

SESSION_ID="$1"
SUCCESS="$2"
OUTPUT="$3"
HOOK_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_DIR="$HOOK_DIR/../logs"
mkdir -p "$LOG_DIR"

echo "[$(date '+%Y-%m-%d %H:%M:%S.%3N')] SESSION_END $SESSION_ID success=$SUCCESS | ${OUTPUT:0:200}" >> "$LOG_DIR/session_end.log"
exit 0
