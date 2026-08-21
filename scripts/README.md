# Nuphus 工具脚本

## 图标生成

| 脚本 | 说明 |
|------|------|
| `generate_icons.py` | 图标生成器 — 生成 Nuphus 全尺寸图标（16×16 ~ 256×256），输出到 `src-tauri/icons/` 和 `frontend/public/` |
| `render_svg_cairo.py` | SVG 图标渲染 — 使用 pycairo 原生渲染 Nuphus 图标 SVG，生成高质量 PNG |

## 开发辅助

| 脚本 | 说明 |
|------|------|
| `dev-restart.bat` | 开发重启 — 重启前端 dev server |
| `dogfood.ps1` | 自用（dogfooding）脚本 — 项目内自动化辅助 |

## 使用注意

- 所有 `.ps1` 脚本为 Windows PowerShell，部分有对应 `.py` 跨平台版本
- ~~涉及 Nuphus 进程管理的脚本（`restart_nuphus`、`build_restart`、`candle_rebuild`）必须由外部守护进程调用，不可由 Leader 直接执行~~ **2026-07-11 移除**：`restart_nuphus.ps1` 与 `build_restart.ps1` 因存在 `Stop-Process` 自杀风险且无自动调用方（已 grep 全项目无引用）已删除。Nuphus 重启请直接通过开发环境（`cd frontend && npx tauri dev`）或人工触发 cargo build 完成。
- 可执行权限：PowerShell 需 `-ExecutionPolicy Bypass`