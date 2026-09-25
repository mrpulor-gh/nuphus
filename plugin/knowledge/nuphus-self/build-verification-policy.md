# 验证约束

## 触发

已做出影响结论的改动才验证。只读、分析、建议、纯文案/注释/文档改动：零验证。

## 适用范围

| 被改的代码库 | 适用 |
|---|---|
| 用户项目（任意栈） | A + C |
| agent 自身所在项目 | B + C |
| Nuphus 仓库自身 | D + C |

互不通用。

## 验证前四问（全过才验证）

1. 改动影响结论？否 → 不验证
2. 本机环境允许（平台/依赖/权限/凭据）？否 → 声明盲区换路径，不重试
3. 改动落在 L1~L5 哪层？未定 → 先定位
4. 谁消费结论？无人 → 不验证

## A. 用户项目 — 改哪层测哪层

| 层 | 内容 | 触发 |
|---|---|---|
| L1 | 语法/编译 | 改任何源码 |
| L2 | 类型/静态 | 改签名、结构、接口 |
| L3 | 单元/契约 | 改逻辑、分支、数据形状 |
| L4 | 集成/端到端 | 改跨模块调用、配置、协议 |
| L5 | 运行时/真机 | 改 UI、并发、系统集成、性能 |

改 L1 只跑 L1；改 L3 跑 L1+L2+L3。禁止习惯性全量。

工具：
- Rust：`cargo check` / `clippy -D warnings` / `cargo test` / `fmt --check`
- Node/TS：`tsc --noEmit` / `eslint` / `vitest` / `prettier --check`
- Python：`compileall` / `ruff` / `mypy` / `pytest`
- Go：`go build|vet|test ./...` / `gofmt -l`
- C#：`dotnet build|test` / `format --verify-no-changes`
- C/C++：`cmake --build` / `-Wall -Werror` / `ctest`
- Docker：`docker build` / `hadolint` / `kubeconform`
- Shell：`bash -n` / `shellcheck` / `bats`

测不了 → 降级或移交：

| 缺 | 交给 |
|---|---|
| 平台机器（mac/iOS/Android） | CI / 真机 |
| 真实硬件（GPU/串口/USB/蓝牙） | 真机 |
| 外部依赖（API/凭据/生产数据） | 联调环境 |
| 长稳/并发/性能规模 | 专用探针 |
| 设计/文案/UX 判断 | 用户 |
| 真机交互（UI/手势/权限弹窗） | 真机 |

可降级到 L1~L4 则降级；不可降级 → 声明盲区 + 指明验证方。禁止在测不了的层反复试。

平台处置：
- 产物被运行中进程锁定：先停程序；停不了 → 声明交 CI，不绕过。
- 运行时/系统库缺失：补；补不了 → 降级 + 声明。
- 重型原生依赖卡死（ONNX/GPU）：单独跑 + 超时；必要时精确 skip 并声明。
- 单个用例卡死阻塞后续：精确 skip 该用例，禁止整轮重跑。

禁止：
1. 单一栈 SOP 当通用规则
2. 本机限制当任务结论
3. 用验证数量冒充判断质量
4. 给零消费方的产物堆验证
5. 测不了反复试且不声明
6. 未改动先验证

## B. agent 自身所在项目

- 只增量 `cargo check`，禁止 build；复用已有 `target/`。
- 多处改动攒一批一次验。
- 触及类型/schema/字段/载荷：契约级验证。
- 禁止新建 target。
- target 被占用：报告用户定时机，不迂回。
- 同一策略连失败 3 次 → 停，换路径。
- 禁止数据副本/临时环境变量/降断言造假绿。
- 完成 = 目标达成 ∧ 已验证 ∧ 证据可追溯 ∧ 下游可集成 ∧ 无错误残留。

## C. 执行纪律

- 任务 &gt;2 分钟：后台进程 + 输出落盘 + 轮询，禁止阻塞等待。
- shell 超时上限 600s；超时取消不杀子进程 → 输出已断 = 废跑。
- 命令莫名挂起：禁用 `-PassThru -NoNewWindow`；杀进程不加管道。

## D. Nuphus 仓库专属（Nuphus 及 nuphus-mcp）

- 只 `cargo check -p &lt;crate&gt;` 增量，禁止全量 build。
- 产物被运行中 nuphus.exe 锁定（sherpa-onnx / onnxruntime dll）：报告用户定时机，禁 kill 自身，禁迂回。
- 测试：Rust `cargo test -p &lt;crate&gt; &lt;模块&gt;`；前端 `npx vitest run &lt;单文件&gt;`（全量仅收尾/发版）；`cargo fmt --all -- --check` + `prettier --check`；`clippy --all-targets -D warnings` + `eslint`。
- `paddle_ocr` ONNX 用例本机卡死 → `-- --skip &lt;case&gt;`，禁整轮重跑。
- macOS RPA/AX 契约（本机非 mac）→ 交 CI `check-macos`。
- 桌面自动化/UIA → 真机或 `#[ignore]`。
- CI：push main 只跑 fmt+tsc+prettier（不编译）；PR/tag 全量；发布集中跑一次。
- `desktop-api` / `nuphus-browser` 在 Nuphus 与 nuphus-mcp 各一份且已分叉：改一处须同步另一处，两侧分别验证。
- nuphus-mcp 门禁：check（3 OS）+ test（win/mac）+ real-Chrome 集成（win，`--ignored --test-threads=1`）+ audit + fmt。