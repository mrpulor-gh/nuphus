# Nuphus

> CLAUDE.md 是给外部 AI 编码助手的项目指引。
> 项目自身的 Constitution 见 `.nuphus/charter.md`。

## 技术栈

- Rust 1.95+ (核心引擎)
- Tauri v2 (桌面壳)
- React 18 + TypeScript (前端)
- Candle (本地 Embedding)
- jieba-rs (中文分词)

## 项目结构

```
src/         — 核心库 (agent / api / tools / workflow / security)
src-tauri/   — Tauri 桌面应用
crates/      — 独立 crate (nuphus-index / nuphus-browser)
src-tauri/crates/desktop-api — 桌面自动化基础设施
plugin/      — 插件 (skills / workflows / knowledge / apps 运行时实例)
frontend/    — React 前端
prompts/     — Agent 提示词模板
```

## 开发命令

```bash
cargo build          # 构建核心
cargo test           # 运行测试
cargo clippy --all-targets -- -D warnings   # Lint
cargo fmt --all -- --check                  # 格式检查
cd frontend && npx tauri dev               # 启动桌面应用
```