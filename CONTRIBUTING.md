# Contributing to Nuphus

感谢你对 Nuphus 的关注！Nuphus 是一个协同共生桌面助手，采用 Rust 构建核心 + Tauri 桌面壳。

## 开发环境

### 必需工具
- **Rust** 1.78+（[rustup](https://rustup.rs/)）
- **Node.js** 18+（用于 Tauri 前端；核心功能不依赖）
- **Tauri CLI**: `cargo install tauri-cli --version "^2"`
- **Git**

### 快速开始

```bash
git clone https://github.com/mrpulor-gh/nuphus.git
cd nuphus

# 构建核心库
cargo build

# 运行测试
cargo test

# 构建 Tauri 桌面应用（需 Node.js）
cd src-tauri && cargo tauri build
```

## 项目结构

```
nuphus/
├── src/                        # 核心库（nuphus crate）
│   ├── agent/                  # Agent 引擎（ReActAgent、SubTaskLoop）
│   ├── api/                    # AI API 客户端（ProviderKind）
│   ├── bin/                    # CLI 工具（nuphus-cli，可执行入口）
│   ├── cache/                  # 工具结果缓存（ToolCache）
│   ├── desktop/                # 桌面自动化（DesktopClient、OCR 字典引擎）
│   ├── knowledge/              # 知识库引擎
│   ├── permissions.rs          # 权限策略
│   ├── runtime/                # 运行时（sub_task_loop）
│   ├── security/               # 注入检测（InjectionDetector）
│   ├── tools/                  # 工具注册与执行（ToolRegistry）
│   └── workflow/               # 工作流引擎（Executor、Compiler）
├── src-tauri/                  # Tauri 桌面应用
│   └── crates/desktop-api/     # 桌面控制基础设施（Win32 + xcap）
├── crates/nuphus-index/        # 知识库索引引擎
├── plugin/                     # 插件（workflows、skills、knowledge）
└── frontend/                   # React 前端（Tauri 渲染层，可与 src-tauri 协同启动）
```

## 构建命令

| 命令 | 说明 |
|------|------|
| `cargo build` | 构建核心库 |
| `cargo test` | 运行所有测试 |
| `cargo test --lib` | 仅运行 lib 测试 |
| `cargo clippy --all-targets` | 运行 Clippy 检查 |
| `cargo fmt --all` | 格式化代码 |
| `cargo check` | 快速检查编译 |

## PR 流程

1. Fork 仓库并创建功能分支（`feat/xxx` 或 `fix/xxx`）
2. 确保 `cargo test` 全部通过
3. 确保 `cargo clippy --all-targets` 无新增警告
4. 确保 `cargo fmt --all` 格式化一致
5. 提交 PR 并简要描述变更内容
6. 提交 PR 即表示你同意将你的贡献按 MIT 许可证授权（Developer Certificate of Origin）

## 国际化贡献

Nuphus 目前支持中文（zh）和英文（en）两种语言。
如需添加新语言：
1. 在 `frontend/src/locales/` 目录下创建 `{lang_code}.ts` 文件，格式参考 `zh.ts`
2. 在 `frontend/src/locales/index.ts` 中注册新语言包
3. 确保所有 i18n keys 与中文版一致（共 50x 个 key）

## 代码风格

- 遵循 `cargo fmt` 格式
- 新功能需包含测试
- 修改公共 API 需更新相关文档注释
- 注释优先使用英文（技术术语）或中文（业务说明）
- 避免引入不必要的依赖