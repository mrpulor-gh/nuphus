# 构建预测与验证约束

**定位适用范围，再执行**

| 被验证对象 | 执行部分 |
|---|---|
| 用户项目（任意技术栈） | Part I + Part III |
| Agent 所在项目自身 | Part II + Part III |
| Nuphus 仓库自身 | Part IV |

误用即错：**Part II / Part IV 的取舍不得套用到用户项目**。

---

# Part I — 通用（任意用户项目）

## 1. 动手前四问 — 全过才构建

| # | 判据 | 不通过时 |
|---|---|---|
| 1 | 动作会改变结论吗？ | 不会 → 不执行 |
| 2 | 本机环境允许吗？(平台 / 依赖 / 权限 / 凭据) | 不允许 → 声明盲区，换路径，**不重试** |
| 3 | 改动落在 L1~L5 哪一层？ | 未定位 → 先定位 |
| 4 | 谁消费结论？ | 无人 → 不执行 |

## 2. 分层执行 — 改哪层测哪层

| 层 | 范围 | 触发 |
|---|---|---|
| L1 | 语法 / 编译 | 改任何源码 |
| L2 | 类型 / 静态分析 | 改签名、结构、接口 |
| L3 | 单元 / 契约 | 改逻辑、分支、数据形状 |
| L4 | 集成 / 端到端 | 改跨模块调用、配置、协议 |
| L5 | 运行时 / 真机 | 改 UI、并发、系统集成、性能 |

改 L1 只跑 L1；改 L3 跑 L1+L2+L3。禁止习惯性全量。

## 3. 工具链表（按栈执行）

| 栈 | L1 编译/构建 | L2 类型·静态 | L3 单测 | 结构·风格 |
|---|---|---|---|---|
| Rust | `cargo build` | `clippy -- -D warnings` | `cargo test` | `cargo fmt --check` |
| Node / TS | `tsc` · `vite build` | `tsc --noEmit` · `eslint` | `vitest` / `jest` | `prettier --check` |
| Python | `compileall` | `mypy` / `pyright` · `ruff check` | `pytest` | `ruff format --check` |
| Go | `go build ./...` | `go vet` · `staticcheck` | `go test ./...` | `gofmt -l` |
| Java / Kotlin | `mvn compile` / `gradle compile` | `javac -Xlint` · `spotbugs` | `mvn test` / `gradle test` | `spotless` / `ktlint` |
| C# / .NET | `dotnet build` | `/warnaserror` | `dotnet test` | `dotnet format --verify-no-changes` |
| C / C++ | `cmake --build` | `-Wall -Werror` · `clang-tidy` | `ctest` / `gtest` | `clang-format --dry-run` |
| PHP | `php -l` | `phpstan` | `phpunit` | `php-cs-fixer --dry-run` |
| Ruby | `ruby -c` | `rubocop` | `rspec` | `rubocop` |
| Swift | `swift build`（需 macOS） | `swift build` | `swift test` | `swift-format` |
| SQL / 迁移 | `psql -f` / 干跑 | `EXPLAIN` 审计划 | 迁移往返测试 | — |
| Docker / K8s | `docker build` | `hadolint` · `kubeconform` | 镜像内 smoke | — |
| Shell | `bash -n` | `shellcheck` | `bats` | `shfmt -d` |

执行规则：
- 增量优先；只跑受影响目标（`-p <crate>` / 单文件）
- 工具缺失 → 能装则装；不能装 → 降级 + 声明盲区，**禁止假装测过**

## 4. 测不了 — 按缺什么资源判定

| 缺什么 | 处置 |
|---|---|
| 平台机器（mac / iOS / Android / 多平台发行） | 交 CI 或真机 |
| 真实硬件（GPU / 串口 / USB / 蓝牙 / 传感器） | 真机 |
| 外部依赖（第三方 API / 凭据 / 额度 / 生产数据） | 联调环境 |
| 时间与规模（长稳 / 并发 / 数据量 / 性能 SLA） | 专用探针 |
| 真人判断（设计 / 文案 / UX / 产品取舍） | 交用户 |
| 真实交互（真机 UI / 手势 / 输入法 / 权限弹窗） | 真机 |

可降级至 L1~L4 则降级验证；不可降级 → **声明盲区 + 指明验证方**。

## 5. 平台约束 — 遇到即按此处置

| 约束 | 处置 |
|---|---|
| 产物被运行中进程锁定 | 先停程序；停不了 → 声明盲区，交 CI，**不绕过** |
| 平台运行时缺失（dll / 系统库 / SDK） | 补齐；补不了 → 降级 + 声明盲区 |
| 重型原生依赖初始化卡死（ONNX / GPU） | 单独跑 + 设超时；必要时精确 skip 并声明 |
| 构建器串行（一个卡死用例阻塞其后全部） | 精确 skip，禁止整轮重跑 |

## 6. CI 分工

| 触发 | 跑什么 |
|---|---|
| 日常提交 | 仅**秒级**静态检查（格式 / 类型 / lint），不编译 |
| PR / 发布 | **全量**：编译 + lint + 单测 + 平台矩阵 |
| 发版前 | **集中跑一次**；禁止单项修改频繁触发 CI |

## 7. 禁止

1. 单一技术栈 SOP 当通用规则
2. 本机环境限制当任务结论
3. 用构建/测试数量冒充判断质量
4. 为零消费方的产物堆验证仪式
5. 在测不了的层反复试而不声明盲区
6. 把自身项目取舍（Part II）套到用户项目

---

# Part II — 自身构建（验证对象 = Agent 所在项目）

触发条件：被改 / 被验的代码库**就是运行 Agent 的这个项目**。

## 8. 原 L0「自身构建原则」条文（现由本文档承载）

L0 已精简为一句指路（`src/agent/prompt.rs:163`、`:1103`），条文细节以本表为准：

| 情形 | 执行 |
|------|---------|
| 验证自身项目 | 只做增量 check：复用项目 `target/`，**禁止 build** |
| 多处改动 | 攒团，不单独发起 |
| 纯文案 / 注释 | 零验证 |
| 触及类型 / schema / 字段 / 载荷 | 契约级验证 |
| 新建 target 目录 | 禁止——丢依赖缓存即全量重编 |
| `target` 被占用 | 报告用户定时机，不迂回 |

## 9. 同源约束（L0 逐字）

- Verify：「验证标准是**交付物满足下游契约**而非编译通过；低风险改动静态验证，触及边界（类型/schema/字段/载荷）需契约级验证」
- 失败处置：「同一策略连续失败 **3 次** → 终止无效循环，切换路径」
- 归因纪律：「先问『功能缺失什么逻辑』；**禁止环境绕过（数据副本 / 临时环境变量 / 降断言）制造假绿**」
- Done：`目标达成 ∧ 产物已验证 ∧ 证据链可追溯 ∧ 下游可集成 ∧ 无错误残留`

---

# Part III — 执行纪律（任何项目通用）

| 情形 | 执行 |
|---|---|
| 任务 >2 分钟 | `Start-Process` + `RedirectStandardOutput/Error` 落盘 + 轮询；**禁止阻塞等待** |
| `system_shell` 超时 | 上限 600s，超时"已取消"**不杀子进程** → 孤儿继续跑但输出已断 = 废跑 |
| 命令莫名挂起 | 禁用 `-PassThru -NoNewWindow`、`taskkill \| Out-String`（句柄继承）→ 用 `-WindowStyle Hidden`；`taskkill` 不加管道 |

---

# Part IV — Nuphus 仓库专属

> ⚠️ 仅适用于 Nuphus 仓库自身（含关联仓库 `nuphus-mcp`）。用户项目不适用。
> 依据：`.github/workflows/`、根 `Cargo.toml`、`frontend/package.json`

## 10. 结构

```
Rust workspace: src / src-tauri / src-tauri/crates/desktop-api / crates/nuphus-index / crates/nuphus-browser
exclude: third_party/chromiumoxide（由 [patch.crates-io] 本地路径覆盖）
前端: frontend/（独立 npm 工程，内嵌 dist）
```
- clippy allow 白名单：`too_many_arguments` / `type_complexity` / `len_without_is_empty` / `new_without_default` / `module_inception` / `manual_range_contains` / `manual_is_multiple_of` / `items_after_test_module`；其余由 CI `-D warnings` 强制

## 11. 构建

| 约束 | 执行 |
|---|---|
| 产物被运行中 `nuphus.exe` 锁定（`sherpa-onnx-c-api.dll` / `onnxruntime.dll` 等） | 报告用户定时机；S5 禁 kill 自身，**禁迂回** |
| 全量 build | 禁止；只做 `cargo check -p <crate>` 增量 |
| 新建 / 迁移 `target` | 禁止 |
| 多点改动 | 攒团一次验 |

## 12. 测试

| 层 | 命令 |
|---|---|
| Rust 单测 | `cargo test -p <crate> <模块路径>` |
| Rust 契约 | `cargo check -p <crate>`（或 `--bins`） |
| 前端单测 | `npx vitest run <单文件>` |
| 前端全量 | `npx vitest run`（仅收尾 / 发版） |
| 格式 | `cargo fmt --all -- --check` · `npx prettier --check "src/**/*.{ts,tsx,css}"` |
| Lint | `cargo clippy --all-targets -- -D warnings` · `npx eslint . --ext .ts,.tsx` |

| 已知毒瘤 | 处置 |
|---|---|
| `paddle_ocr` ONNX 用例本机卡死（非失败） | `-- --skip <case_name>` 精确排除，禁止整轮重跑 |
| macOS RPA 契约测试（本机非 macOS） | 交 CI `check-macos` |
| 桌面自动化 / UIA 操作（需真实桌面会话） | 真机或 `#[ignore]` 测试 |

## 13. CI 门禁

| 触发 | 内容 |
|---|---|
| push main | Main Guard：`cargo fmt --all -- --check` + `npx tsc --noEmit` + `npx prettier --check`（秒级，不编译） |
| PR / tag / 手动 | `clippy --all-targets -D warnings` · `cargo check --workspace` · `cargo test --workspace`(ubuntu+windows) · `check-macos` · `smoke-macos` · frontend(tsc+eslint+vitest `--no-file-parallelism`+prettier) |
| 发布 | `release.yml`；发版前集中跑一次 |

`check-macos` 定点契约（仅 macOS runner）：
```
cargo test -p nuphus --lib desktop_automation::
cargo test -p nuphus --lib tools::semantic_desktop::
cargo test -p nuphus --lib tools::registry::tests::
cargo test -p nuphus --lib desktop::
cargo test -p desktop-api --lib
cargo test -p nuphus-desktop --bin nuphus macos_permissions::
cargo test -p nuphus-desktop --bin nuphus commands::config::jev::tests::
```
AX 契约测试**不授予** macOS 辅助功能权限；真实应用动作需另行授权的 Mac 验收。

`nuphus-mcp` 门禁：`check --workspace`(3 OS) · `cargo test`(win/mac) · real-Chrome 集成(win, `--ignored --test-threads=1`) · `cargo audit` · `cargo fmt --check`。

## 14. 分叉

`desktop-api` / `nuphus-browser` 在 Nuphus 主仓库与 nuphus-mcp 仓库**各一份且已分叉**：改一处不同步另一处，两侧须分别验证。
