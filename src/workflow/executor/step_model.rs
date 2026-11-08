//! model 步骤执行 — 一次模型调用（text / image / video）
//!
//! 与其它步骤的边界：
//! - 区别于 chat：单轮调用，无工具、无多轮循环（chat 是 LLM+工具决策循环）
//! - 区别于 tool：默认**不重试**（abort 即单次），仅 on_error 显式 retry 才重试
//!   —— 生成调用有成本且多带副作用（落盘/计费），默认 2 次重试会放大浪费
//! - image/video 不复制生成逻辑：复用 `tools::builtin::generation` 的结构化函数，
//!   与 `image_generate` / `video_generate` 工具同一实现。这里只构造语义请求
//!   （`GenerationRequest`），语义字段 → 厂商请求体键名的映射只活在生成协议层
use super::*;
use crate::config::GenerationKind;
use crate::tools::builtin::generation::{self, GenerationRequest};

/// text 模态缺省系统提示：一句中性引导。
///
/// 刻意不复用 `BASE_CHAT_AGENT_PROMPT`——那是「决策 + 工具循环」节点的身份定义
/// （工具使用规范、FAILED 前缀约定等），model 节点只是单轮生成，套上会误导输出。
pub(super) const MODEL_TEXT_SYSTEM_PROMPT: &str =
    "你是工作流中的内容生成节点。根据用户指令直接产出最终结果，不调用工具，不输出与结果无关的解释。";

impl Executor {
    /// 执行 model 步骤 — 单次模型调用（text / image / video）
    ///
    /// capture 语义：text=正文，image=第一张图片绝对路径，video=视频绝对路径。
    /// image 模态额外写入 `<capture>_files`（全部图片绝对路径数组），
    /// 让 count>1 时被主产物契约丢弃的其余图片仍可被下游取到。
    pub(super) async fn execute_model_step(
        &self,
        step: &Step,
        prompt: &str,
        opts: &ModelOpts,
        variables: &mut HashMap<String, serde_json::Value>,
        llm: Option<&dyn ApiClient>,
        events: &EventBus,
        workflow_id: &str,
    ) -> crate::Result<String> {
        // ── 变量替换：do.model 与 with 内所有字符串（含 params 内嵌）──
        let resolved_prompt = super::variables::resolve_vars_str(prompt, variables);
        let resolved_opts = resolve_model_opts(opts, variables);

        // ── 重试策略：只认 on_error 的显式 retry；abort/skip/allow_codes 均单次 ──
        let (mut max_retries, backoff_ms) = match &step.on_error {
            OnError::Retry {
                max, backoff_ms, ..
            } => (*max, *backoff_ms),
            _ => (0, 0),
        };
        if crate::workflow::debug::current().is_some_and(|session| !session.use_retry_policy) {
            max_retries = 0;
        }

        let mut last_error = String::new();
        for attempt in 0..=max_retries {
            if attempt > 0 {
                events.emit(WorkflowEvent::StepRunRetry {
                    step_id: step.id.clone(),
                    attempt,
                });
                events.emit(WorkflowEvent::Error {
                    message: format!(
                        "Model '{}' ({}): attempt {}/{} after error: {}",
                        resolved_opts.kind.as_str(),
                        step.name,
                        attempt,
                        max_retries + 1,
                        last_error
                    ),
                });
                let delay = if backoff_ms > 0 {
                    Duration::from_millis(backoff_ms * 2u64.pow(attempt - 1))
                } else {
                    Duration::from_millis(500)
                };
                tokio::time::sleep(delay).await;
                // 重试前检查取消信号
                self.check_cancel(workflow_id).await?;
            }

            match self
                .model_call_once(step, &resolved_prompt, &resolved_opts, llm)
                .await
            {
                Ok(output) => {
                    // ── 变量捕获：主产物 + image 全部产物清单 ──
                    super::variables::capture_output(&step.capture, &output.primary, variables)?;
                    if resolved_opts.kind == ModelKind::Image {
                        write_artifact_list(&step.capture, &output.artifacts, variables);
                    }
                    return Ok(output.primary);
                }
                Err(error) => last_error = error.to_string(),
            }
        }

        Err(crate::NuphusError::agent(format!(
            "Model step '{}' ({}): failed after {} attempt(s): {}",
            step.name,
            step.id(),
            max_retries + 1,
            last_error
        )))
    }

    /// 单次模型调用（不含重试）
    async fn model_call_once(
        &self,
        step: &Step,
        prompt: &str,
        opts: &ModelOpts,
        llm: Option<&dyn ApiClient>,
    ) -> crate::Result<ModelCallOutput> {
        match opts.kind {
            ModelKind::Text => {
                let text = self.model_text_once(step, prompt, opts, llm).await?;
                Ok(ModelCallOutput {
                    primary: text,
                    artifacts: Vec::new(),
                })
            }
            ModelKind::Image => Self::model_image_once(prompt, opts),
            ModelKind::Video => Self::model_video_once(prompt, opts),
        }
    }

    /// text：单轮 LLM 调用，无工具、无多轮；system 取 with.system，缺省中性提示。
    async fn model_text_once(
        &self,
        step: &Step,
        prompt: &str,
        opts: &ModelOpts,
        llm: Option<&dyn ApiClient>,
    ) -> crate::Result<String> {
        // 0. 模型路由：与 chat 步骤同一套规则（step_chat_agent.rs 的 with.model 路由）——
        //    显式 provider 必须精确创建；无 provider 时仅唯一候选可推断，多候选直接报错；
        //    未注册的裸模型名保留兼容语义（回退主 client）。
        let mut routed_client: Option<std::sync::Arc<dyn ApiClient>> = None;
        if let Some(ref model_id) = opts.model {
            if let Some(ref factory) = self.client_factory {
                let client_result = if let Some(ref provider) = opts.provider {
                    Some(factory.create_client_for(provider, model_id))
                } else {
                    let registry = factory.registry()?;
                    match registry.find_model_candidates(model_id).as_slice() {
                        // Preserve the legacy bare-model fallback for workflows whose
                        // model was never registered; ambiguity is never guessed.
                        [] => None,
                        [(provider, model)] => {
                            Some(factory.create_client_for(&provider.name, &model.id))
                        }
                        candidates => Some(Err(crate::NuphusError::llm(format!(
                            "model '{}' is ambiguous across providers ({} candidates); specify provider",
                            model_id,
                            candidates.len()
                        )))),
                    }
                };
                if let Some(client_result) = client_result {
                    match client_result {
                        Ok(client) => {
                            tracing::info!(
                                "[Model] step '{}': routed to {}/{}",
                                step.name,
                                opts.provider.as_deref().unwrap_or("unique provider"),
                                model_id
                            );
                            routed_client = Some(client);
                        }
                        Err(err) => return Err(err),
                    }
                } else {
                    tracing::warn!(
                        "[Model] step '{}': model '{}' not in registry, fallback to main client (bare model name)",
                        step.name,
                        model_id
                    );
                }
            }
        }
        let llm: &dyn ApiClient = match routed_client.as_deref() {
            Some(client) => client,
            None => llm.ok_or_else(|| {
                crate::NuphusError::agent("Model step (text) requires LLM client".to_string())
            })?,
        };

        // 1. 单轮请求：system 走专用字段（transport 负责前置），user = 替换后的任务描述
        let system = opts
            .system
            .clone()
            .unwrap_or_else(|| MODEL_TEXT_SYSTEM_PROMPT.to_string());
        let model_name = opts
            .model
            .clone()
            .unwrap_or_else(|| llm.model_name().to_string());
        let mut request = crate::api::MessageRequest::new(
            model_name,
            vec![serde_json::json!({"role": "user", "content": prompt})],
        )
        .with_system(system);
        if let Some(max_tokens) = opts.max_tokens {
            request.max_tokens = Some(max_tokens);
        }
        if let Some(temperature) = opts.temperature {
            request.temperature = Some(temperature);
        }

        // 2. 调用并拼接文本增量（无工具调用分支：model 节点不消费 ToolUse）
        let events = llm.stream(request).await.map_err(|e| {
            crate::NuphusError::agent(format!(
                "Model step '{}': LLM call failed: {}",
                step.name, e
            ))
        })?;
        let mut response = String::new();
        for event in &events {
            if let crate::api::AssistantEvent::TextDelta(text) = event {
                response.push_str(text);
            }
        }
        Ok(response)
    }

    /// image：复用生成实现；主产物 = 第一张图片的绝对路径，artifacts = 全部图片路径。
    fn model_image_once(prompt: &str, opts: &ModelOpts) -> crate::Result<ModelCallOutput> {
        let request =
            model_generation_request(prompt, opts, GenerationKind::Image).map_err(|error| {
                crate::NuphusError::agent(format!("Model image generation failed: {error}"))
            })?;
        let provider = opts.provider.clone();
        let model = opts.model.clone();
        let paths = crate::tools::builtin::run_blocking(move || {
            generation::generate_images(&request, provider.as_deref(), model.as_deref())
                .map(|artifacts| {
                    artifacts
                        .into_iter()
                        .map(|artifact| artifact.path)
                        .collect::<Vec<_>>()
                })
                .map_err(|error| error.to_string())
        })
        .map_err(|error| {
            crate::NuphusError::agent(format!("Model image generation failed: {error}"))
        })?;
        let primary = primary_artifact_path(&paths).ok_or_else(|| {
            crate::NuphusError::agent("Model image generation returned no artifact".to_string())
        })?;
        let artifacts = paths
            .iter()
            .cloned()
            .map(ensure_absolute)
            .map(|path| path.display().to_string())
            .collect();
        Ok(ModelCallOutput {
            primary: primary.display().to_string(),
            artifacts,
        })
    }

    /// video：复用生成实现；主产物 = 视频绝对路径；timeout_secs 缺省 300。
    fn model_video_once(prompt: &str, opts: &ModelOpts) -> crate::Result<ModelCallOutput> {
        use generation::{
            DEFAULT_POLL_INTERVAL_SECS, DEFAULT_VIDEO_TIMEOUT_SECS, MAX_VIDEO_TIMEOUT_SECS,
        };
        let mut request =
            model_generation_request(prompt, opts, GenerationKind::Video).map_err(|error| {
                crate::NuphusError::agent(format!("Model video generation failed: {error}"))
            })?;
        request.poll_interval_secs = Some(DEFAULT_POLL_INTERVAL_SECS);
        request.timeout_secs = Some(
            opts.timeout_secs
                .unwrap_or(DEFAULT_VIDEO_TIMEOUT_SECS)
                .clamp(30, MAX_VIDEO_TIMEOUT_SECS),
        );
        let provider = opts.provider.clone();
        let model = opts.model.clone();
        let path = crate::tools::builtin::run_blocking(move || {
            generation::generate_video(&request, provider.as_deref(), model.as_deref())
                .map(|artifact| artifact.path)
                .map_err(|error| error.to_string())
        })
        .map_err(|error| {
            crate::NuphusError::agent(format!("Model video generation failed: {error}"))
        })?;
        let primary = ensure_absolute(path).display().to_string();
        Ok(ModelCallOutput {
            primary,
            artifacts: Vec::new(),
        })
    }
}

/// 单次 model 调用的产物集合。
///
/// `primary` 是 capture 主产物（既有契约：text=正文 / image=首图路径 /
/// video=视频路径）；`artifacts` 是本次调用的全部落盘产物（text 为空、video 单个、
/// image 为 API 返回顺序的全部图片）。主产物契约不变，多产物通过派生变量暴露。
struct ModelCallOutput {
    primary: String,
    artifacts: Vec<String>,
}

/// image 模态的产物清单：`<capture>_files` = 全部图片绝对路径数组。
///
/// 主 capture 保持「第一张路径」的既有契约（下游 first_frame 等字段直接消费）；
/// 清单是 additive 派生变量，让 count>1 时其余图片也有名字可取。capture 名支持
/// `{{var}}` 模板，与 [`super::variables::capture_output`] 同一套解析；无 capture
/// 或无产物时不写。
pub(super) fn write_artifact_list(
    capture: &Option<String>,
    artifacts: &[String],
    variables: &mut HashMap<String, serde_json::Value>,
) {
    let Some(capture) = capture.as_deref() else {
        return;
    };
    if artifacts.is_empty() {
        return;
    }
    let name = super::variables::resolve_vars_str(capture, variables);
    variables.insert(format!("{name}_files"), serde_json::json!(artifacts));
}

/// 构造语义生成请求：具名字段原样进 `GenerationRequest`（语义字段 → 厂商请求体
/// 键名的映射只活在生成协议层，这里不出现任何厂商键名），`params` 原样进
/// `extra`（非对象由语义层按既有契约报错）。
///
/// 模态归属由协议自行判定：`ratio` 两种模态通用，`count` 仅 image，
/// `duration` / `first_frame` 仅 video——跨模态字段协议层会忽略（compiler 已给
/// warning）。全部未设置时 extra 为空 map，调用方拿到的就是「没有 params」，
/// 与改动前一致。
pub(super) fn model_generation_request(
    prompt: &str,
    opts: &ModelOpts,
    kind: GenerationKind,
) -> std::result::Result<GenerationRequest, String> {
    let mut request = GenerationRequest::new(kind, prompt);
    request.ratio = opts.ratio.clone();
    request.count = opts.count;
    request.duration = opts.duration;
    request.first_frame = opts.first_frame.clone();
    request.extra = GenerationRequest::extra_from_params(opts.params.as_ref())
        .map_err(|error| error.to_string())?;
    Ok(request)
}

/// with 内所有字符串（含 params 内嵌字符串）做 {{var}} 替换。
///
/// 走 serde 往返 + `resolve_vars`：ModelOpts 新增字段时无需改这里，
/// params 的嵌套字符串与 `"{{var}}"` 定型替换（数字/布尔/对象）一并覆盖。
pub(super) fn resolve_model_opts(
    opts: &ModelOpts,
    variables: &HashMap<String, serde_json::Value>,
) -> ModelOpts {
    let raw = serde_json::to_value(opts).unwrap_or_default();
    let resolved = Executor::resolve_vars(&raw, variables);
    serde_json::from_value(resolved).unwrap_or_else(|_| opts.clone())
}

/// 生成产物的绝对路径保障：产出目录由 home_dir 拼出本就是绝对路径，
/// 这里对异常相对路径按当前工作目录兜底——capture 出去的路径必须能被
/// 下游直接消费（如 video_generate 的首帧图参数）。
pub(super) fn ensure_absolute(path: std::path::PathBuf) -> std::path::PathBuf {
    if path.is_absolute() {
        return path;
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path),
        Err(_) => path,
    }
}

/// image 主产物 = 生成结果的第一条路径（API 返回顺序即主图顺序）。
/// 空结果由调用方报错（生成实现在无产物时已先行失败），这里只做选取与绝对化。
pub(super) fn primary_artifact_path(paths: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    paths.first().map(|path| ensure_absolute(path.clone()))
}
