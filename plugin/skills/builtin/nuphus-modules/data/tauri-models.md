# Tauri 壳 · 模型聚合服务（src-tauri/src/models/）

> 定位：**模型元数据层**——两条互不相干的支线汇于一个目录：① 首启视觉模型（PaddleOCR + YOLO）的自愈下载与 splash 进度契约（bootstrap.rs）；② OpenRouter 公开模型目录的本地缓存聚合与 (provider, model) 能力匹配（aggregator.rs），是上下文窗口 / 模态 / reasoning efforts / 定价的权威兜底源。
> 规模：3 文件共 1354 行。

## 结构

| 文件 | 规模 | 职责 |
|---|---|---|
| `models/mod.rs` | 5 | 仅模块声明 + 一句话总览注释，无任何再导出（调用方一律写全路径 `crate::models::bootstrap::…` / `crate::models::aggregator::…`） |
| `models/bootstrap.rs` | 741 | **视觉模型自愈下载器**：4 文件 manifest、状态扫描、镜像回退下载、splash 进度折算、kind 标签事件流（vision_files / scan_status / run_download / ensure_vision_models_blocking / MIRRORS / DOWNLOAD_RUNNING） |
| `models/aggregator.rs` | 608 | **OpenRouter 目录聚合器**：目录拉取、双 TTL 磁盘缓存、vendor 映射、两级匹配（精确/退化 + 通用基名）（fetch / ensure_cache / lookup / vendor_for / normalize_base / CONTEXT_TTL_SECS / PRICE_TTL_SECS） |

## 功能作用

- **本地视觉模型自愈分发（bootstrap）**：产品原则是「除 STT 外所有本地模型都必须自动获取，最终用户是非技术用户，绝不手工下载模型」（`bootstrap.rs:3-6`）。PaddleOCR 三件套（det/rec/字典）+ YOLO 图标检测模型共 4 个文件，首启时经 `preload_ocr` 命令同步下载，进度驱动 splash 加载条，事件驱动 ModelsPage 视觉卡片。
- **云端模型能力元数据聚合（aggregator）**：各家 provider 的 `/v1/models` 端点能力信息不一致（智谱只返回裸 id 列表、DeepSeek 只给 provider 级 context_length），而内置 ProviderRegistry 表编译期冻结、新模型永远缺失；OpenRouter 提供免费免 key 的全量模型目录（含 per-model context_length、input_modalities、reasoning efforts、定价），本模块把它拉取成本地缓存文件，并把 Nuphus 的 (provider, model) 对匹配上去。

## 规则与坑

- 模块自身**不持有任何全局状态、不注册 AppState**——bootstrap 只用一个进程级 `AtomicBool` 做下载单例锁（`bootstrap.rs:52`），aggregator 只读写磁盘缓存文件（`aggregator.rs:179-200`）。
- 两条支线的耦合点为零：bootstrap 只依赖 tauri / reqwest::blocking / dirs / crate::splash / nuphus::embed；aggregator 只依赖 reqwest（async）/ serde_json / nuphus::config::registry::normalize_model_id。
