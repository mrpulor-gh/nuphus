# 知识索引引擎（crates/nuphus-index/）

> 定位：一个零新增外部依赖的纯内存 HashMap 知识库索引引擎——递归扫描 `plugin/knowledge/` 下的 .md 文件，手写解析 YAML frontmatter 提取 title/tags/正文，以加权子串匹配 + 严格 AND 多词评分做相关性排序，并用单个 JSON 文件持久化，同时服务于 Tauri 桌面前端（知识库页面）与 Agent 工具的 `knowledge_search`。
> 规模：8 文件（6 源 + 1 example + Cargo.toml），src 约 1052 行。

## 结构

三层管道 + 共享词汇表：

| 文件 | 职责 |
|---|---|
| `types.rs`（105） | 全部公开数据类型：FileMeta / QueryRequest / KnowledgeHit / IndexConfig |
| `scanner.rs`（249） | 目录遍历：全量（scan_directory）与增量（scan_modified）两条路径，构建 `rel_path → FileMeta` 内存映射 + 反向标签索引 |
| `parser.rs`（166） | .md 解析：手写迷你 YAML frontmatter KV 提取、title/tags 推导、正文切分 |
| `index.rs`（205） | 引擎本体：`RwLock<HashMap>` 双索引、启动加载或全量重建、整库序列化为单个 `knowledge_index.json` |
| `query.rs`（293） | 检索与相关性评分（含 7 个内联单测）：标题/标签/正文三段加权子串匹配，多关键词严格 AND |
| `lib.rs`（34） | crate 门面：模块声明 + 公开再导出 + 用法文档示例 |

## 功能作用

- 用户在 `plugin/knowledge/` 目录下随手丢 `.md` 文件即可自动收录（前端 KnowledgePage 提示语即「在 plugin/knowledge/ 下创建 .md 文件即可自动收录」）。
- 检索评分按标题/标签/正文三段加权，多关键词按空白切分、严格 AND 语义，按分数降序返回 KnowledgeHit。
- 被两条独立链路消费：Tauri 壳的 IPC 命令（前端知识库页面）和 Agent 进程的 `knowledge_search` 工具。

## 规则与坑

- **叶子 crate**：Cargo.toml 只依赖 serde / serde_json / tracing，不反向依赖 src（nuphus 主 crate）或 src-tauri；`docs_root`/`index_path` 全部由调用方注入。
- `FileMeta.embedding` 是**无注入方的预留字段**，与桌面视觉（src/desktop/vision）和记忆库向量检索（src/embed.rs）均无任何代码接线——这一点已被 desc-impl-audit-2026-10-25 录为文档偏差（M2 条目）。
- 设计哲学（lib.rs:3-4）：「零新增外部依赖，纯内存 HashMap + JSON 持久化。仅索引 plugin/knowledge/ 下的 .md 文件。」
