//! Candle Embedding 引擎
//!
//! 使用 bge-small-zh (safetensors) 将文本转为 512 维向量，
//! 支持语义相似度搜索。模型不存在时自动下载。

use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokenizers::Tokenizer;

static EMBEDDER_LOCK: OnceLock<Mutex<Option<&'static Embedder>>> = OnceLock::new();

/// 进程内同类初始化错误只记录一次（节流），成功加载后重置
static ERROR_LOGGED: AtomicBool = AtomicBool::new(false);

/// 毒化自愈（删权重重下）每进程只允许一次：加载失败未必是文件损坏（如
/// debug 构建下 Candle 加载瞬时失败），无上限的删 95MB 权重重下必须掐断。
static SELF_HEAL_ATTEMPTED: AtomicBool = AtomicBool::new(false);

fn embedder_lock() -> &'static Mutex<Option<&'static Embedder>> {
    EMBEDDER_LOCK.get_or_init(|| Mutex::new(None))
}

fn write_embed_error(msg: &str) {
    let log_dir = Embedder::model_dir()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let log_path = log_dir.join("embed_error.log");
    // 超过 1MB 时清空重建，避免无限膨胀
    let oversized = std::fs::metadata(&log_path)
        .map(|m| m.len() > 1024 * 1024)
        .unwrap_or(false);
    let open_result = if oversized {
        std::fs::File::create(&log_path)
    } else {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
    };
    if let Ok(mut f) = open_result {
        let _ = writeln!(f, "{}", msg);
    }
}

/// Embedding 引擎
pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    dim: usize,
}

/// bge-small-zh 模型的 query 前缀（用于非对称检索）
const BGE_QUERY_PREFIX: &str = "为这个句子生成表示以用于检索相关文章：";

/// 模型文件列表（Candle 格式）
const MODEL_FILES: &[&str] = &["pytorch_model.bin", "tokenizer.json", "config.json"];
/// Hugging Face 模型 ID
const HF_MODEL_ID: &str = "BAAI/bge-small-zh";
/// 下载镜像（按优先级顺序：国内镜像优先，国外直连兜底）
const MIRRORS: &[&str] = &["https://hf-mirror.com", "https://huggingface.co"];

impl Embedder {
    fn model_dir() -> PathBuf {
        std::env::var("NUPHUS_EMBED_MODEL_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::data_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("nuphus")
                    .join("models")
                    .join("bge-small-zh")
            })
    }

    fn download_model(
        dir: &PathBuf,
        on_progress: &mut (dyn FnMut(u64, u64, &str) + Send),
    ) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建模型目录失败: {}", e))?;
        tracing::info!("[Embed] 正在下载模型到: {}", dir.display());

        // 直连下载（使用 hf-mirror.com 等中国大陆镜像，无需代理）
        let agent = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

        for file in MODEL_FILES {
            let path = dir.join(file);
            let min_size = Self::min_file_size(file);
            // 跳过判定带体积下限（反毒化）：与 bootstrap.rs 视觉模型同型——
            // 纯 exists() 会让截断/错误页文件被永远跳过、永不自愈
            if std::fs::metadata(&path)
                .map(|m| m.len() >= min_size)
                .unwrap_or(false)
            {
                tracing::info!("[Embed]  跳过已存在的: {}", file);
                continue;
            }

            let mut last_err = String::new();
            let mut downloaded = false;

            // 进度节流 ~1 MiB（与 bootstrap.rs 视觉模型一致），文件完成必报；
            // 跨镜像/重试保留 last_emit，避免同一文件重复刷新进度。
            let mut last_emit = 0u64;
            let mut file_cb = |downloaded: u64, total: u64| {
                if downloaded == total || downloaded - last_emit >= 1024 * 1024 {
                    last_emit = downloaded;
                    on_progress(downloaded, total, file);
                }
            };

            'mirrors: for &mirror in MIRRORS {
                let url = format!("{}/{}/resolve/main/{}", mirror, HF_MODEL_ID, file);

                // 每个镜像最多 3 次尝试（首次 + 2 次重试，指数退避 1s/2s）
                for attempt in 0..3u32 {
                    if attempt > 0 {
                        std::thread::sleep(Duration::from_secs(1 << (attempt - 1)));
                    }
                    tracing::info!("[Embed]  下载 {} ← {} (第 {} 次)", file, url, attempt + 1);

                    match Self::download_once(&agent, &url, &path, &mut file_cb) {
                        Ok(size) if size >= min_size => {
                            tracing::info!("[Embed]  完成: {} ({} bytes)", file, size);
                            downloaded = true;
                            break 'mirrors;
                        }
                        Ok(size) => {
                            // 体积不达标：疑似错误页/毒化文件，删除后换镜像
                            last_err = format!(
                                "{}: {} 体积异常 ({} bytes < 期望至少 {} bytes)",
                                mirror, file, size, min_size
                            );
                            tracing::warn!("[Embed]  {}", last_err);
                            let _ = std::fs::remove_file(&path);
                            break;
                        }
                        Err(e) => {
                            last_err = format!("{}: {}", mirror, e);
                            tracing::warn!(
                                "[Embed]  镜像 {} 第 {} 次下载失败: {}",
                                mirror,
                                attempt + 1,
                                e
                            );
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }

            if !downloaded {
                // 清理残留部分文件，避免下次被当作"已存在"跳过
                let _ = std::fs::remove_file(&path);
                return Err(format!("所有镜像下载 {} 均失败: {}", file, last_err));
            }
        }

        tracing::info!("[Embed] 模型下载完成");
        Ok(())
    }

    /// 单次下载尝试：检查 HTTP 状态码并流式写入磁盘，返回写入字节数。
    /// `on_progress(downloaded, total)` 随写入推进（调用方负责节流）。
    fn download_once(
        agent: &reqwest::blocking::Client,
        url: &str,
        path: &PathBuf,
        on_progress: &mut (dyn FnMut(u64, u64) + Send),
    ) -> Result<u64, String> {
        let mut resp = agent
            .get(url)
            .timeout(Duration::from_secs(300))
            .send()
            .map_err(|e| format!("请求失败: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(format!(
                "HTTP {} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            ));
        }

        let total = resp.content_length().unwrap_or(0);
        let mut f = std::fs::File::create(path).map_err(|e| format!("创建文件失败: {}", e))?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut downloaded = 0u64;
        loop {
            let n = resp
                .read(&mut buf)
                .map_err(|e| format!("读取数据失败: {}", e))?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n])
                .map_err(|e| format!("写入文件失败: {}", e))?;
            downloaded += n as u64;
            on_progress(downloaded, total);
        }
        Ok(downloaded)
    }

    /// 模型文件的最小合法体积，用于识别错误页/毒化文件
    fn min_file_size(file: &str) -> u64 {
        match file {
            "config.json" => 100,
            "tokenizer.json" => 100 * 1024,
            "pytorch_model.bin" => 10 * 1024 * 1024,
            _ => 1,
        }
    }

    pub fn init(
        model_dir: Option<PathBuf>,
        on_progress: &mut (dyn FnMut(u64, u64, &str) + Send),
    ) -> Result<Self, String> {
        let dir = model_dir.unwrap_or_else(Self::model_dir);

        // 按文件检查完整性（体积下限而非纯存在性），空目录/部分缺失/截断残留
        // 都会触发补下载；download_model 内部跳过达标文件，天然支持断点续传
        let file_ok = |f: &str| {
            std::fs::metadata(dir.join(f))
                .map(|m| m.len() >= Self::min_file_size(f))
                .unwrap_or(false)
        };
        if MODEL_FILES.iter().any(|f| !file_ok(f)) {
            tracing::info!("[Embed] 模型文件不完整，尝试自动下载: {}", dir.display());
            Self::download_model(&dir, on_progress)?;
        }

        match Self::load_from_dir(&dir) {
            Ok(embedder) => Ok(embedder),
            Err(first_err) => {
                // 毒化文件自愈：删除权重文件，重新下载并再加载一次。
                // 每进程只做一次——第二次失败说明问题大概率不在文件（如 debug
                // 构建的 Candle 瞬时失败），继续重下只会无限循环。
                if SELF_HEAL_ATTEMPTED.swap(true, Ordering::SeqCst) {
                    let msg = format!("模型加载失败且毒化自愈已尝试过，不再重下: {first_err}");
                    write_embed_error(&msg);
                    return Err(msg);
                }
                tracing::warn!("[Embed] 模型加载失败（{}），删除权重后重新下载", first_err);
                let _ = std::fs::remove_file(dir.join("pytorch_model.bin"));
                Self::download_model(&dir, on_progress)
                    .map_err(|e| format!("{}；重新下载失败: {}", first_err, e))?;
                Self::load_from_dir(&dir)
            }
        }
    }

    /// 从目录加载模型与分词器（不触发下载）
    fn load_from_dir(dir: &Path) -> Result<Self, String> {
        let weights_path = dir.join("pytorch_model.bin");
        let tokenizer_path = dir.join("tokenizer.json");
        let config_path = dir.join("config.json");

        if !weights_path.exists() {
            return Err(format!("pytorch_model.bin not found in {}", dir.display()));
        }
        if !tokenizer_path.exists() {
            return Err(format!("tokenizer.json not found in {}", dir.display()));
        }
        if !config_path.exists() {
            return Err(format!("config.json not found in {}", dir.display()));
        }

        tracing::info!("[Embed] 加载模型: {}", weights_path.display());

        let device = Device::Cpu;
        let config_str = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("读取 config.json 失败: {}", e))?;
        let config: BertConfig = serde_json::from_str(&config_str)
            .map_err(|e| format!("解析 BERT config 失败: {}", e))?;

        let tensor_vec: Vec<(String, Tensor)> = candle_core::pickle::read_all(&weights_path)
            .map_err(|e| format!("加载 pytorch_model.bin 失败: {}", e))?;
        let tensors: HashMap<String, Tensor> = tensor_vec.into_iter().collect();
        let vb = VarBuilder::from_tensors(tensors, candle_core::DType::F32, &device);
        let model =
            BertModel::load(vb, &config).map_err(|e| format!("构建 BERT 模型失败: {}", e))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("加载 tokenizer 失败: {}", e))?;

        // bge-small-zh 实际输出 512 维
        let dim = config.hidden_size;
        tracing::info!("[Embed] 模型加载成功，dim={}", dim);

        Ok(Embedder {
            model,
            tokenizer,
            device,
            dim,
        })
    }

    /// 获取全局 Embedder 引用。首次调用自动加载模型，失败后下次调用自动重试。
    pub fn get() -> Option<&'static Embedder> {
        Self::get_with_progress(&mut |_: u64, _: u64, _: &str| {})
    }

    /// 带进度回调的全局加载：首次调用自动下载模型，下载推进时回调
    /// `on_progress(downloaded, total, filename)`（调用方负责节流与转发）。
    /// 已加载/已就绪时立即返回且不回调。用于 splash 展示真实下载进度。
    pub fn get_with_progress(
        on_progress: &mut (dyn FnMut(u64, u64, &str) + Send),
    ) -> Option<&'static Embedder> {
        // Mutex 中毒恢复（与 preload() 一致）：避免历史上一次 panic 导致进程内永久静默不可用
        let mut guard = embedder_lock().lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            // init（模型下载 + 加载）是重阻塞操作，且下载使用 reqwest::blocking：
            // 在 tokio runtime 线程上直接调用，debug 构建下会在 reqwest 内部
            // wait::enter()（debug_assertions 检测 runtime 上下文）panic，
            // release 下也会长时间阻塞异步调度。统一放到独立 OS 线程执行并 join——
            // 子线程 panic 仅表现为 join 返回 Err，不会令本锁中毒。
            let init_result =
                std::thread::scope(|s| s.spawn(|| Embedder::init(None, on_progress)).join());
            match init_result {
                Ok(Ok(embedder)) => {
                    let leaked: &'static Embedder = Box::leak(Box::new(embedder));
                    *guard = Some(leaked);
                    // 成功加载后重置节流标记，允许后续真实新错误再记录
                    ERROR_LOGGED.store(false, Ordering::SeqCst);
                }
                Ok(Err(err)) => {
                    // 进程内同类错误只记录一次，避免每次调用都刷日志/写文件
                    if !ERROR_LOGGED.swap(true, Ordering::SeqCst) {
                        let msg = format!(
                            "{} [Embed] init failed: {}",
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                            err
                        );
                        tracing::error!("[Embed] 模型初始化失败: {}，语义搜索将不可用", err);
                        write_embed_error(&msg);
                    }
                }
                Err(_) => {
                    if !ERROR_LOGGED.swap(true, Ordering::SeqCst) {
                        let msg = format!(
                            "{} [Embed] init panicked in worker thread",
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                        );
                        tracing::error!("[Embed] 模型初始化线程 panic，语义搜索将不可用");
                        write_embed_error(&msg);
                    }
                }
            }
        }
        *guard
    }

    /// 外部预加载（Tauri setup 后台线程调用），避免首次使用时阻塞消息处理
    pub fn preload(embedder: &'static Embedder) {
        let mut guard = embedder_lock().lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(embedder);
        }
    }

    /// 查询模型是否已加载（不触发加载，供降级判断）
    pub fn is_loaded() -> bool {
        embedder_lock().lock().map(|g| g.is_some()).unwrap_or(false)
    }

    pub fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        self.embed_inner(text, true)
    }

    pub fn embed_passage(&self, text: &str) -> Result<Vec<f32>, String> {
        self.embed_inner(text, false)
    }

    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    fn embed_inner(&self, text: &str, is_query: bool) -> Result<Vec<f32>, String> {
        let input = if is_query {
            format!("{}{}", BGE_QUERY_PREFIX, text)
        } else {
            text.to_string()
        };

        let encoding = self
            .tokenizer
            .encode(input, false)
            .map_err(|e| format!("Tokenization failed: {}", e))?;

        let ids = encoding.get_ids();
        let mask = encoding.get_attention_mask();
        let ids_len = ids.len();

        let input_ids = Tensor::from_slice(ids, (1, ids_len), &self.device)
            .map_err(|e| format!("创建 input_ids tensor 失败: {}", e))?;
        let token_type_ids = Tensor::zeros((1, ids_len), candle_core::DType::I64, &self.device)
            .map_err(|e| format!("创建 token_type_ids tensor 失败: {}", e))?;
        let attention_mask = Tensor::from_slice(mask, (1, ids_len), &self.device)
            .map_err(|e| format!("创建 attention_mask tensor 失败: {}", e))?;

        let output = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))
            .map_err(|e| format!("Candle inference failed: {}", e))?;

        // 取 CLS token (第一个 token) 的输出
        let cls = output
            .narrow(1, 0, 1)
            .map_err(|e| format!("narrow CLS failed: {}", e))?
            .squeeze(1)
            .map_err(|e| format!("squeeze CLS failed: {}", e))?
            .squeeze(0)
            .map_err(|e| format!("squeeze batch dim failed: {}", e))?;

        let vec: Vec<f32> = cls
            .to_vec1()
            .map_err(|e| format!("tensor 转 vec 失败: {}", e))?;

        Ok(Self::l2_normalize(&vec))
    }

    fn l2_normalize(v: &[f32]) -> Vec<f32> {
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            v.iter().map(|x| x / norm).collect()
        } else {
            v.to_vec()
        }
    }

    pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        let denom = norm_a * norm_b;
        if denom > 0.0 {
            dot / denom
        } else {
            0.0
        }
    }

    pub fn dim(&self) -> usize {
        self.dim
    }
}
