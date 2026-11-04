# 会话/记忆/存储体系（src/session/ src/memory/ src/store/ src/embed.rs src/segmenter.rs）

> 定位：Nuphus「记住什么、怎么压缩、存到哪里」的完整数据体系。以 Session 内存态会话为中心管理 LLM 上下文窗口，以 SQLite（WAL + FTS5 + HNSW）为唯一持久底座支撑 MemoryEntry 双通路检索（FTS5+BM25 关键词 / Embedding 语义），并以 TenetStore 承载用户不可变教导。
> 规模：15 文件约 7070 行。

## 结构

四层：

| 层 | 文件 | 职责 |
|---|---|---|
| 会话层（纯内存） | `session/types.rs`（221）/ `session.rs`（593）/ `transform.rs`（492）/ `image.rs`（89） | 消息三层结构、生命周期、API 序列化、提炼三件套、图片三态策略 |
| 领域类型层 | `memory/entry.rs`（430）/ `tenets.rs`（706）/ `mod.rs`（86） | MemoryEntry（一维 kind 五分类：conversation/task_trace/distill/pattern/snapshot）、Tenet 体系 |
| 持久层 | `store/db.rs`（460）/ `session.rs`（1138）/ `memory.rs`（1193） | 连接池、sessions 表 + 快照 + 项目归属、记忆条目双通路检索 |
| 检索基础设施 | `embed.rs`（559）/ `segmenter.rs`（238） | Candle + bge-small-zh 本地 Embedding（512 维、自动下载、毒化自愈）；jieba 统一分词 |

## 功能作用

- **上下文窗口管理**：消息推送、工具对清理（strip/seal 有意分工）、`to_api_messages` 序列化（六个关键决策）；逼近上限时三级处理——in-loop 提炼（`replace_with_distill`/`accumulate_distill`）、失败回滚（`truncate_messages`）、TooLong 应急截断。
- **记忆双通路检索**：关键词（FTS5 + BM25 评分）与语义（bge-small-zh 向量 + HNSW 近似最近邻）两条通路；外加 Custom 卡片隔离、项目隔离（session_meta 归属）两层过滤。
- **TenetStore**：用户确认的不可变原则，明确不走演化漏斗、不受遗忘机制影响。
- **统一分词**：segmenter 是 jieba 中文分词统一入口（替代历史上所有 n-gram 实现），FTS 索引侧与查询侧共用同一套分词，保证两侧对称。

## 规则与坑

- 本模块是**纯数据层**：不含任何业务决策（refine 触发判据在 `agent/distill.rs`，ReAct 循环在 `runtime/`）。
- `prompt_snapshot` 永不落盘；SQLite 是唯一记忆存储层，写入走单事务保证主表 + FTS5 强一致。
- `src-tauri/` 仅通过 IPC 命令桥接触本模块。
