# Contributing to Nuphus

感谢你对 Nuphus 的关注！Nuphus 是一个协同共生桌面助手，采用 Rust 构建核心 + Tauri 桌面壳。

## 开发环境

### 必需工具
- **Rust** 1.95.0（[rustup](https://rustup.rs/)）。仓库通过 `rust-toolchain.toml` 锁定该版本，
  无需手动 `rustup toolchain install`——rustup 读该文件自动切换。
- **Node.js** 20+（用于 Tauri 前端与前端测试；核心功能不依赖）。下限由 vitest 4 决定
  （`engines.node = ^20 || ^22 || >=24`），CI 统一使用 Node 20。
- **Tauri CLI**: `cargo install tauri-cli --version "^2"`
- **Git**

### 快速开始

```bash
git clone https://github.com/mrpulor-gh/nuphus.git
cd nuphus

# 安装依赖（根目录 Tauri CLI + 前端依赖）
npm install

# 开发模式启动（前端 dev server + Rust 引擎）
npx tauri dev

# 核心库 + 桌面壳（主 workspace 全量）
cargo build
cargo test

# 只跑核心引擎的 lib 测试（最快的反馈环，936 个用例）
cargo test -p nuphus --lib

# 中继服务器是独立 workspace，需单独构建与测试
cargo test --manifest-path relay-server/Cargo.toml
```

> `cargo build` / `cargo test` 在仓库根目录作用于主 workspace
> （`src`、`src-tauri`、`src-tauri/crates/desktop-api`、`crates/nuphus-index`、
> `crates/nuphus-browser`）。`relay-server/` 不在其中。
>
> 首次构建会由 `src-tauri/build.rs` 自动下载 sherpa-onnx / onnxruntime 平台库到
> `src-tauri/desktop/sherpa/`，需要网络。**若上一次 `tauri dev` 仍在运行**，该同步步骤
> 会因 DLL 被占用而失败（`os error 32`）——先退出正在运行的实例再构建。

## 项目结构

```
nuphus/
├── src/                        # 核心库（nuphus crate；同时经 main.rs 导出 nuphus CLI）
│   ├── agent/                  # Agent 引擎（Leader 决策、ExecAgent 执行、WorkflowAgent 设计）
│   ├── runtime/                # 统一主循环，三模式路由（Leader / Workflow / Custom）
│   ├── transports/             # 多 Provider 抽象（chat_completions / anthropic / responses）
│   ├── llm/                    # 客户端工厂（ClientFactory）
│   ├── api/                    # ProviderKind 与 API 数据类型
│   ├── config/                 # 配置加载（load_registry、providers.toml、config.toml）
│   ├── tools/                  # 工具注册与执行（ToolRegistry、builtin/ 各领域工具）
│   ├── cache/                  # 工具结果缓存（ToolCache）
│   ├── desktop/                # 桌面控制客户端（DesktopClient、OCR 引擎）
│   ├── desktop_automation/     # 平台 RPA（windows_uia、macos_accessibility）
│   ├── workflow/               # 工作流引擎（Executor、Compiler、调试与回放）
│   ├── memory/                 # 记忆条目与 tenets（跨会话经验）
│   ├── store/                  # 持久化（SQLite + FTS5、会话、资源内容寻址去重）
│   ├── security/               # 注入检测与人在回路（InjectionDetector、approval、user_input）
│   ├── skill/                  # 技能（plugin/skills/ 的 .md 方法论）
│   ├── mcp/                    # MCP 接入
│   ├── cookies/                # Cookie 与密钥保险库（Windows DPAPI 加密）
│   ├── session/                # 会话状态
│   └── permissions.rs          # 权限策略
├── src-tauri/                  # Tauri 桌面应用（mobile_server、relay_client、config commands）
│   └── crates/desktop-api/     # 桌面控制基础设施（Win32 + xcap + ONNX 视觉）
├── crates/
│   ├── nuphus-index/           # 知识库索引引擎（为 plugin/knowledge/ 的 .md 建索引）
│   └── nuphus-browser/         # 浏览器自动化内核（CDP / chromiumoxide，与 nuphus-mcp 共享）
├── relay-server/               # 中继服务器（独立 workspace；只路由不落盘）
├── plugin/                     # 零编译扩展（workflows、skills、knowledge、ui-maps、apps、mcp…）
├── frontend/                   # React 前端（Tauri 渲染层，含 mobile PWA 与 locales/）
├── npm-desktop/                # npm 分发包（@nuphus/nuphus-desktop）
├── hooks/                      # Agent 生命周期钩子（pre/post_tool_call、session_start/end）
└── tools/                      # 辅助工具（stt-proto 等）
```

## 构建与检查命令

### Rust

| 命令 | 说明 |
|------|------|
| `cargo build` | 构建核心库 |
| `cargo check --workspace` | 快速检查编译（CI 用） |
| `cargo test --workspace` | 运行所有测试（CI 用） |
| `cargo test -p nuphus --lib` | 仅运行核心库 lib 测试 |
| `cargo clippy --all-targets -- -D warnings` | Clippy 门禁，警告即失败（CI 用） |
| `cargo fmt --all` | 格式化代码 |
| `cargo fmt --all -- --check` | 只检查格式，不改写（CI 用） |

### 前端

| 命令 | 说明 |
|------|------|
| `npm run dev` | 启动前端 dev server |
| `npm run build` | `tsc && vite build` |
| `npx tsc --noEmit` | 类型检查（CI 用） |
| `npx eslint . --ext .ts,.tsx` | Lint（CI 用） |
| `npx vitest run` | 前端测试（CI 用；低资源机器可加 `--no-file-parallelism`） |
| `npx prettier --check "src/**/*.{ts,tsx,css}"` | 只检查格式，不改写（CI 用） |

> 根目录与 `frontend/` 各有一套 npm scripts：根目录的 `dev` / `build` 会自动 `cd frontend`。
> 上表前端命令均在 `frontend/` 目录下执行。

## PR 流程

1. Fork 仓库并创建功能分支（`feat/xxx` 或 `fix/xxx`）
2. 本地跑通与 CI 一致的全部门禁（见下表），避免把红灯交给 reviewer
3. 提交 PR 并简要描述变更内容
4. 提交 PR 即表示你同意将你的贡献按 Apache-2.0 许可证授权（Developer Certificate of Origin）

### CI 实际门禁

| 门禁 | 命令 | 触发 |
|------|------|------|
| Format check | `cargo fmt --all -- --check` | PR、tag、main |
| Clippy | `cargo clippy --all-targets -- -D warnings` | PR、tag、手动 |
| Check compilation | `cargo check --workspace` | PR、tag、手动 |
| Test | `cargo test --workspace` | PR、tag、手动（ubuntu + windows） |
| macOS arm64 | `cargo check --workspace` + 定向契约测试 + 启动冒烟 | PR、tag、手动 |
| TypeScript | `npx tsc --noEmit` | PR、tag、main |
| ESLint | `npx eslint . --ext .ts,.tsx` | PR、tag、手动 |
| Vitest | `npx vitest run` | PR、tag、手动 |
| Prettier | `npx prettier --check "src/**/*.{ts,tsx,css}"` | PR、tag、main |

> 日常 push 到 main **不**跑全量 CI（界面/文案类改动用本地 tsc/vitest/cargo check 把关，
> 发版时集中跑）。`main-guard.yml` 只补三类与运行环境无关的全仓扫描：
> `cargo fmt --check`、`tsc --noEmit`、`prettier --check`。详见 `.github/workflows/`。

## 国际化贡献

Nuphus 支持中文（zh）和英文（en）两种语言，语言包在 `frontend/src/locales/`：

```
locales/
├── index.ts               # LangProvider + t()，packs 注册处
├── zh.ts / en.ts          # 主语言包（各 2014 条自有 key）
├── workflowEditor.ts      # 工作流编辑器文案（editorZh / editorEn 各 337 条，被 zh.ts / en.ts 展开）
└── workflowCanvasCopy.ts  # 画布文案（41 条，被 workflowEditor.ts 合并进 editorZh/editorEn）
```

当前合计 **2351 个 key**（zh 与 en 各 2351，键集合零漂移）。

新增或修改文案时：

1. 在 `zh.ts` 与 `en.ts` **同时**增删 key——键集合漂移会让 UI 直接显示原始 key
2. 跑 `npx vitest run src/test/locales.test.ts` 确认中英键集合完全相等

新增**语言**（而非仅修改文案）时，还需在 `frontend/src/locales/index.ts` 的 `packs`
中注册语言包，并参照 `zh.ts` 的结构拆出 `{lang_code}.ts`。

> 键一致性由测试强制保证，**不需要手工维护 key 清单**：
> - `src/test/locales.test.ts` 断言 zh/en 键集合零漂移，且总量不低于基线 500（防整表误删）
> - `src/locales/en.mojibake.test.ts` 拦截英文包里的乱码（历史踩过的坑）
>
> 不要为了"顺手"给测试快照精确 key 数——该测试注释里已说明理由：键数每次增删都要手改，
> 纯负担且易漏。

## 代码风格

- Rust 遵循 `cargo fmt` 格式；前端遵循 `prettier`（`npm run format`）
- 新功能需包含测试
- 修改公共 API 需更新相关文档注释
- 注释优先使用英文（技术术语）或中文（业务说明）
- 避免引入不必要的依赖
- **注释写「为什么」而非「做了什么」**——本仓库多处注释保留了实测数据与踩坑记录，
  是刻意维护的资产，请沿用