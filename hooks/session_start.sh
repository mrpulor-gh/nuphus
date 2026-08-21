#!/bin/bash
# session_start hook
# 会话开始时调用
# 参数: session_id user_input

SESSION_ID="$1"
USER_INPUT="$2"
HOOK_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_DIR="$HOOK_DIR/../logs"
mkdir -p "$LOG_DIR"

echo "[$(date '+%Y-%m-%d %H:%M:%S.%3N')] SESSION_START $SESSION_ID | $USER_INPUT" >> "$LOG_DIR/session_start.log"
exit 0
