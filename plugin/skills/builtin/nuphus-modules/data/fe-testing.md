# 前端 · 测试体系（src/**/*.test.{ts,tsx} + vite.config.ts test 段）

> 定位：Nuphus 前端的质量护栏——以 Vitest + jsdom 为底座，用「纯函数单测 / RTL 组件测试 / CSS 与源码断言 / SSR 渲染快照 / 回归复现钉」五种范式，把设计宪法、事件契约、渲染管线和历史 bug 全部钉进 CI。前端测试不是「补齐覆盖率」的附属品，而是**文档化的设计契约执行机制**。
> 规模：159 个测试文件 + vite.config.ts test 段 + src/test/setup.ts。

## 结构（五种范式）

| 范式 | 规模 | 内容 |
|---|---|---|
| 纯函数单元测试（主力） | 约 90 个文件 | sessionGroups / turnRailModel / validate / projection / irEdit / scopedEdit / variableRename / runtimeFields / intentText 等纯逻辑层 |
| RTL 组件行为测试 | 约 45 个文件 | 组件层双向覆盖，**只断言可观察结果**（DOM、类名、回调实参、hook stage），不断言内部状态 |
| 源码/CSS 断言测试（本项目特色） | 约 14 个文件 | 读源码/CSS 文本落断言，如 opacity-system.test.ts 钉「滑块 → 变量 → 消费者」三段链路 |
| SSR 渲染快照 | — | react-dom/server 渲染快照 |
| 回归复现钉（Repro Pins） | — | 每个已修 bug 配一个「复现测试」，注释里写清原缺陷链 |

## 功能作用（四类职责）

1. **契约钉**：把 DESIGN-RATIONALE 设计宪法、事件流时序（ExecutionStage）、Token 体系落成断言。
2. **回归钉**：每个已修 bug 配复现测试，注释写清原缺陷链——如 execution-finalizing.test.tsx 记录「收尾期消息被静默入队 → 永不执行 → 队列锁死会话切换」的完整因果。
3. **防静默失败**：针对 jsdom 的能力边界（无布局/级联计算、`?raw` 返回空串、读空文件导致断言恒真）专门设计防御。
4. **行为规格**：纯函数层与组件层双向覆盖。

## 规则与坑

- 测试基础设施在 `frontend/vite.config.ts` 的 test 段 + `frontend/src/test/setup.ts`；与构建配置联动。
- 组件测试只断言可观察结果——这是本项目测试纪律，避免测试与实现细节共舞。
