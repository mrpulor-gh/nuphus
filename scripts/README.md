# Nuphus 工具脚本

## 图标生成

图标真相源：`src-tauri/icons/icon-source.svg` + `render-icons.mjs`（headless Chrome 渲染，用法见该文件头注释）。

## 开发辅助

| 脚本 | 说明 |
|------|------|
| `dev-restart.bat` | 开发重启 — 重启前端 dev server |
| `workspace-patrol.ps1` | 工作区巡检 — 报告（可选清理）会话垃圾 / ignore 误伤 / 凭证形状串；Agent 提交前的参照实现 |

### 工作区巡检

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\workspace-patrol.ps1          # 只读报告
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\workspace-patrol.ps1 -Clean    # 顺带清掉会话垃圾
```

四项检查，按 Agent 提交前应当执行的顺序：

| 段 | 查什么 | 性质 |
|----|--------|------|
| A git 卫生 | 分支 / HEAD / 未提交文件 | 仅上下文，脏工作区是正常状态，不算 finding |
| B 会话垃圾 | 根目录截图、`debug_*`、`*.bak` / `*.orig`、陈旧日志、`frontend/.tmp`、`temp/` | 规则与 `.gitignore` 同源；**已跟踪文件永不删**（`third_party/*.orig` 这种 vendored 文件长得就像补丁残留） |
| C ignore 审计 | 被 ignore 规则吞掉的已跟踪文件（规则写错）/ 被忽略但长得像源码的文件（规则可能太宽） | 只报告不动手 |
| D 凭证扫描 | 未提交 + 未跟踪文本里的 `sk-` / `Bearer` / `api_key` 形状串 | 命中即脱敏输出；example / template / fixture 路径跳过 |

退出码即闸门：`0` = 无 WARN，`1` = 有 ignore 误伤或凭证命中。`-IncludeTempDir` 才把 `temp/` 纳入清理（那目录放着注册表回滚件之类的正当产物，默认不扫）。

## 使用注意

- 所有 `.ps1` 脚本为 Windows PowerShell
- ~~涉及 Nuphus 进程管理的脚本（`restart_nuphus`、`build_restart`、`candle_rebuild`）必须由外部守护进程调用，不可由 Leader 直接执行~~ **2026-07-11 移除**：`restart_nuphus.ps1` 与 `build_restart.ps1` 因存在 `Stop-Process` 自杀风险且无自动调用方（已 grep 全项目无引用）已删除。Nuphus 重启请直接通过开发环境（`cd frontend && npx tauri dev`）或人工触发 cargo build 完成。
- 可执行权限：PowerShell 需 `-ExecutionPolicy Bypass`