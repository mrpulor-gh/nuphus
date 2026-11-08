//! generation — 多提供商模型生成工具（image_generate / video_generate）
//!
//! 分层（自上而下，厂商知识只允许往下沉）：
//! 1. **语义层（本文件）**：`GenerationRequest` 只描述「要生成什么」——模态、
//!    提示词、比例 / 张数 / 时长 / 首帧 / 尺寸 / 返回格式 + `extra`（提供商特定
//!    透传）；`GenerationProtocol` 是协议 trait；按 provider 类型选协议，未实现
//!    的提供商明确报错，**禁止静默回落任何一家**。
//! 2. **协议层（`minimax.rs`）**：端点、请求键名、响应解析、任务轮询只活在这里。
//! 3. **文件层（`artifacts.rs`）**：落盘 / 下载，不认识任何服务商。
//!
//! 新增一家提供商 = 一个协议实现文件 + `GENERATION_PROTOCOLS` 加一行；
//! 语义层、工具契约（description / schema）、workflow schema、UI、skill 文档
//! 都不需要改。
//!
//! 凭证：读模型界面的**能力绑定**（providers.toml `[capabilities]` 的
//! `image_generation` / `video_generation` + 对应 `_provider` 归属）——用户在
//! 模型界面绑定一次，工具默认读绑定；`provider` / `model` 参数只作当次覆盖。
//! 未绑定且未覆盖 → 明确报错并列候选，绝不再自行遍历 provider 挑第一个
//! （用户没选过的模型不该被静默使用）。
//! api_key 只进 Authorization 头，永不写入输出/日志/工作流文件。
//! 产出目录：`~/.nuphus/generated/`（自动创建），文件名带时间戳。

use crate::config::{
    load_registry, resolve_generation_binding, GenerationKind, ModelEntry, ModelRegistry,
    ProviderConfig, ProviderKind,
};
use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;
use minimax::MiniMaxMediaProtocol;
use serde_json::{json, Map, Value};
use std::path::PathBuf;

mod artifacts;
mod media;
mod minimax;

/// 视频轮询默认间隔 / 默认总超时
pub(crate) const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
pub(crate) const DEFAULT_VIDEO_TIMEOUT_SECS: u64 = 300;
/// 视频轮询超时上限（外层 registry 给本工具 900s 桶，见 registry.rs 超时分级）
pub(crate) const MAX_VIDEO_TIMEOUT_SECS: u64 = 600;

// ── 错误归一 ──

/// 生成流程的错误类别。UI 与工作流只看到归一化后的信息（`Display` 只出文案），
/// 不看到任何厂商细节。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GenErrorKind {
    /// 该提供商没有实现生成协议（禁止静默回落）
    UnsupportedProvider,
    /// 网络 / 磁盘 IO 失败（请求发送、响应读取、下载、落盘）
    Transport,
    /// 请求契约 / 配置类失败与响应形态不符（缺密钥、缺端点、params 非对象、
    /// 响应缺字段、业务错误码）
    Protocol,
    /// 异步任务轮询超时
    Timeout,
    /// 协议成功返回但没有任何产物
    EmptyResult,
}

/// 归一化生成错误：类别 + 可读文案。
///
/// `kind` 是归一化错误契约的一部分（调用方据此区分「提供商未实现 / 传输 /
/// 协议 / 超时 / 空产物」），当前由守卫测试与语义层断言消费；生产路径只透出
/// `Display` 文案，因此这里显式标注，避免 dead_code 噪音。
#[derive(Debug, Clone)]
pub(crate) struct GenError {
    #[allow(dead_code)]
    pub kind: GenErrorKind,
    pub message: String,
}

impl GenError {
    pub(crate) fn unsupported_provider(message: impl Into<String>) -> Self {
        Self {
            kind: GenErrorKind::UnsupportedProvider,
            message: message.into(),
        }
    }

    pub(crate) fn transport(message: impl Into<String>) -> Self {
        Self {
            kind: GenErrorKind::Transport,
            message: message.into(),
        }
    }

    pub(crate) fn protocol(message: impl Into<String>) -> Self {
        Self {
            kind: GenErrorKind::Protocol,
            message: message.into(),
        }
    }

    pub(crate) fn timeout(message: impl Into<String>) -> Self {
        Self {
            kind: GenErrorKind::Timeout,
            message: message.into(),
        }
    }

    pub(crate) fn empty_result(message: impl Into<String>) -> Self {
        Self {
            kind: GenErrorKind::EmptyResult,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for GenError {
    /// 只出文案：工具返回值与工作流错误信息就是这段文字（与重构前逐字一致）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

// ── 语义契约 ──

/// 生成产物：落盘路径 + 字节数 + 异步任务号（同步协议为 None）。
#[derive(Debug, Clone)]
pub(crate) struct Artifact {
    pub path: PathBuf,
    pub bytes: u64,
    /// 异步任务号（同步返回的协议为 None）；仅用于工具输出文案与排查
    pub task_id: Option<String>,
}

/// 语义生成请求：只描述「要生成什么」，零厂商信息。
///
/// 语义字段（比例 / 张数 / 时长 / 首帧 / 尺寸 / 返回格式）由所选提供商的生成
/// 协议决定请求体键名；`extra` 是提供商特定附加参数原样透传（逃生舱），
/// 显式键优先于语义字段。模态不适用的语义字段由协议自行忽略。
#[derive(Debug, Clone)]
pub(crate) struct GenerationRequest {
    pub kind: GenerationKind,
    pub prompt: String,
    /// 画面比例（image/video 适用）
    pub ratio: Option<String>,
    /// image 一次生成张数
    pub count: Option<u32>,
    /// video 时长（秒）
    pub duration: Option<u32>,
    /// video 首帧图路径（空 = 文生视频）
    pub first_frame: Option<String>,
    /// 图像宽度（像素）
    pub width: Option<u32>,
    /// 图像高度（像素）
    pub height: Option<u32>,
    /// 返回格式（取值由协议决定）
    pub output_format: Option<String>,
    /// 提供商特定附加参数（`params` 原样透传）
    pub extra: Map<String, Value>,
    /// 异步生成轮询间隔秒数（None = 语义缺省）
    pub poll_interval_secs: Option<u64>,
    /// 异步生成轮询总超时秒数（None = 语义缺省）
    pub timeout_secs: Option<u64>,
}

impl GenerationRequest {
    /// 语义请求：kind + prompt 必填，其余字段默认不设置。
    pub(crate) fn new(kind: GenerationKind, prompt: impl Into<String>) -> Self {
        Self {
            kind,
            prompt: prompt.into(),
            ratio: None,
            count: None,
            duration: None,
            first_frame: None,
            width: None,
            height: None,
            output_format: None,
            extra: Map::new(),
            poll_interval_secs: None,
            timeout_secs: None,
        }
    }

    /// `params` JSON → `extra` 透传 map。非对象按既有契约报错。
    pub(crate) fn extra_from_params(
        params: Option<&Value>,
    ) -> Result<Map<String, Value>, GenError> {
        match params {
            None => Ok(Map::new()),
            Some(Value::Object(map)) => Ok(map.clone()),
            Some(_) => Err(GenError::protocol("params must be a JSON object")),
        }
    }
}

// ── 协议适配层 ──

/// 生成协议适配层：一家提供商 = 一个实现 + 一行注册。
///
/// 端点、请求键名、响应解析、轮询细节只允许出现在实现里；语义层与产品契约层
/// （工具 description / workflow schema / UI / skill 文档）看不到任何厂商形态。
pub(crate) trait GenerationProtocol: Send + Sync {
    /// 协议标识（如 "minimax_media"），仅用于报错文案与调试
    fn kind(&self) -> &'static str;
    /// 协议内置默认端点（用户未配置 base_url 时回落）；没有则 None
    fn default_base_url(&self) -> Option<&'static str>;
    /// 图像生成：返回落盘产物（顺序即协议返回顺序，主产物 = 第一条）
    fn image(&self, target: &GenTarget, req: &GenerationRequest)
        -> Result<Vec<Artifact>, GenError>;
    /// 视频生成：异步任务轮询后返回落盘产物
    fn video(&self, target: &GenTarget, req: &GenerationRequest)
        -> Result<Vec<Artifact>, GenError>;
}

/// 已实现的生成协议表：新增一家提供商 = 一个协议实现文件 + 这里加一行。
/// 表里没有的提供商一律显式报错，禁止静默回落任何一家。
static GENERATION_PROTOCOLS: &[(ProviderKind, &dyn GenerationProtocol)] =
    &[(ProviderKind::MiniMax, &MiniMaxMediaProtocol)];

/// 按 provider 类型取生成协议；未实现 → None（调用方报 unsupported_provider）。
fn protocol_for(kind: ProviderKind) -> Option<&'static dyn GenerationProtocol> {
    GENERATION_PROTOCOLS
        .iter()
        .find(|(provider_kind, _)| *provider_kind == kind)
        .map(|(_, protocol)| *protocol)
}

/// 已支持协议标识（报错文案用）
fn supported_protocols() -> String {
    GENERATION_PROTOCOLS
        .iter()
        .map(|(_, protocol)| protocol.kind())
        .collect::<Vec<_>>()
        .join(" / ")
}

// ── 凭证解析（消费模型界面的能力绑定）──

/// 生成请求目标：凭证 + 本次生效的模型名 + 协议归属。
///
/// 含 api_key，刻意不实现 Debug / Display 防泄露（与旧 GenCredential 同口径）。
pub(crate) struct GenTarget {
    /// 服务商配置段名（仅报错文案用）
    provider: String,
    /// 协议类型（选生成协议的唯一依据）
    provider_kind: ProviderKind,
    api_key: String,
    /// 生效 base_url（已配置优先，未配置回落协议默认端点）
    base_url: String,
    model: String,
}

/// 生成能力的候选谓词：注册表用哪个字段声明「这个模型能生成」。
///
/// 图像生成有独立的 `supports_image_generation` 声明；视频生成没有对应字段
/// （模型条目里不存在 `supports_video_generation`），因此不谓词过滤——空口断言
/// 「支持视频」比不过滤更坏，会挡住用户显式绑定的模型。
fn generation_predicate(kind: GenerationKind) -> fn(&ModelEntry) -> bool {
    match kind {
        GenerationKind::Image => |m: &ModelEntry| m.supports_image_generation,
        GenerationKind::Video => |_: &ModelEntry| true,
    }
}

/// 候选清单——**只用于报错文案**，不参与任何模型选择。
///
/// 枚举走既有的 `list_models` + `find_model_for_provider`（精确入口），不新写
/// 一套遍历 providers 的查找：选择路径见 [`resolve_generation_target`]，这里仅把
/// 「用户可以绑什么」摊平成可读清单，没有任何选择权。
/// 空清单 = 注册表里还没有模型声明该能力，文案改为引导去模型行打开开关
/// （与视觉能力同一个出口）。
fn candidate_list(
    registry: &ModelRegistry,
    predicate: impl Fn(&ModelEntry) -> bool,
) -> Vec<String> {
    let mut out: Vec<String> = registry
        .list_models()
        .into_iter()
        .filter(|(provider, model_id)| {
            registry
                .find_model_for_provider(provider, model_id)
                .is_some_and(|(_, entry)| predicate(entry))
        })
        .map(|(provider, model_id)| format!("{provider}/{model_id}"))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 未绑定时的报错：说清「去模型界面绑定」并列出可绑候选。
fn unbound_error(registry: &ModelRegistry, kind: GenerationKind, model: Option<&str>) -> String {
    let predicate = generation_predicate(kind);
    let subject = match model {
        Some(m) => format!("未找到{kind}模型 {m}"),
        None => format!("未绑定{kind}能力"),
    };
    let mut msg = format!(
        "{subject}。\n\
         请去模型界面 → 图像音频模型 → {kind} 绑定一个生成模型（绑定会同时记录模型与服务商归属）。"
    );
    if let Some(m) = model {
        msg.push_str(&format!(
            "\n当前指定的模型 {m} 不在注册表中，或未声明生成能力。"
        ));
    }
    let candidates = candidate_list(registry, predicate);
    if candidates.is_empty() {
        msg.push_str(&format!(
            "\n当前注册表中还没有模型声明「{kind}」能力：请在模型列表里为可生成的模型\
             打开「图像生成」声明后再绑定。"
        ));
    } else {
        msg.push_str(&format!(
            "\n可绑定的候选（{} 个）：{}",
            candidates.len(),
            candidates.join("、")
        ));
    }
    msg
}

/// 解析凭证与生效模型——消费能力绑定，参数只作当次覆盖（零厂商知识）。
///
/// 优先级：
/// 1. 生效模型 = 当次 `model` 参数 > 绑定模型；两者都无 → [`unbound_error`]。
/// 2. 生效 provider = 当次 `provider` 参数 > 绑定 provider。
/// 3. provider 明确 → `find_model_for_provider` **精确**解析（同 id 跨段不串）。
/// 4. provider 未知（旧配置没写归属 / 只传了模型名）→ `find_model_candidates`
///    消歧：按能力谓词过滤，唯一命中即用；多个重名 → 报错列出候选让用户去绑定。
///
/// 全程不遍历 provider 找「第一个能用的」——那正是被删掉的
/// `find_minimax_credential` 的静默发现行为。模型查找只走
/// `find_model_for_provider` / `find_model_candidates` 两个既有入口
/// （`find_model` 段序首触是 legacy，禁用于能力决策）。
fn resolve_generation_target(
    registry: &ModelRegistry,
    kind: GenerationKind,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<GenTarget, GenError> {
    let predicate = generation_predicate(kind);
    let binding = resolve_generation_binding(registry, kind).map_err(GenError::protocol)?;

    let model = model_override
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .or_else(|| (!binding.is_unbound()).then(|| binding.model.clone()))
        .ok_or_else(|| GenError::protocol(unbound_error(registry, kind, None)))?;

    let provider = provider_override
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .or(binding.provider.clone());

    if let Some(name) = provider.as_deref().filter(|p| !p.is_empty()) {
        let (cfg, _entry) = registry
            .find_model_for_provider(name, &model)
            .ok_or_else(|| {
                GenError::protocol(format!(
                    "未找到{kind}模型 {name}/{model}：该服务商下没有这个模型条目。\n{}",
                    unbound_error(registry, kind, Some(&model))
                ))
            })?;
        return target_from(cfg, &model);
    }

    // provider 未知 → 候选消歧（能力谓词过滤）
    let mut candidates: Vec<&ProviderConfig> = Vec::new();
    for (cfg, entry) in registry.find_model_candidates(&model) {
        if predicate(entry) {
            candidates.push(cfg);
        }
    }
    match candidates.len() {
        0 => Err(GenError::protocol(unbound_error(
            registry,
            kind,
            Some(&model),
        ))),
        1 => target_from(candidates[0], &model),
        _ => Err(GenError::protocol(format!(
            "{kind}模型 {model} 在多个服务商下重名（{}），无法确定用哪个。\n\
             请去模型界面 → 图像音频模型 → {kind} 重新绑定——绑定会同时记录服务商归属，\
             之后不再有歧义。",
            candidates
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join("、")
        ))),
    }
}

fn target_from(cfg: &ProviderConfig, model: &str) -> Result<GenTarget, GenError> {
    if cfg.api_key.trim().is_empty() {
        return Err(GenError::protocol(format!(
            "服务商 '{}' 未配置 api_key：请先在模型界面为该服务商填写密钥",
            cfg.name
        )));
    }
    Ok(GenTarget {
        provider: cfg.name.clone(),
        provider_kind: cfg.provider_type,
        api_key: cfg.api_key.clone(),
        base_url: cfg.base_url.trim().trim_end_matches('/').to_string(),
        model: model.to_string(),
    })
}

/// 完整目标：凭证 + 生效模型 + 生成协议 + 生效 base_url。
///
/// 协议未实现的提供商在此显式报错（不静默回落任何一家）；base_url 未配置时
/// 回落协议内置默认端点，协议没有默认端点也显式报错。
fn resolve_target(
    registry: &ModelRegistry,
    kind: GenerationKind,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<(GenTarget, &'static dyn GenerationProtocol), GenError> {
    let mut target = resolve_generation_target(registry, kind, provider_override, model_override)?;
    let protocol = protocol_for(target.provider_kind).ok_or_else(|| {
        GenError::unsupported_provider(format!(
            "提供商 '{}' 尚未实现生成协议（已支持：{}）",
            target.provider,
            supported_protocols()
        ))
    })?;
    if target.base_url.is_empty() {
        target.base_url = protocol
            .default_base_url()
            .ok_or_else(|| {
                GenError::protocol(format!(
                    "服务商 '{}' 未配置 base_url，且其生成协议（{}）没有内置默认端点",
                    target.provider,
                    protocol.kind()
                ))
            })?
            .trim_end_matches('/')
            .to_string();
    }
    Ok((target, protocol))
}

// ── 生成入口（工具与 workflow do.model 共用）──

/// 图像生成：返回落盘产物列表（顺序即协议返回顺序，主产物 = 第一条）。
///
/// 工具层（`image_generate`）与 workflow `do.model`（kind=image）共用同一实现，
/// 禁止任何一方复制生成逻辑。工具层在此之上再包人类可读文本。
pub(crate) fn generate_images(
    req: &GenerationRequest,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<Vec<Artifact>, GenError> {
    let registry =
        load_registry().map_err(|e| GenError::protocol(format!("加载模型配置失败: {e}")))?;
    let (target, protocol) =
        resolve_target(&registry, req.kind, provider_override, model_override)?;
    protocol.image(&target, req)
}

/// 视频生成：异步任务轮询后返回落盘的视频产物（一次任务一个文件）。
///
/// 工具层（`video_generate`）与 workflow `do.model`（kind=video）共用同一实现，
/// 禁止任何一方复制生成逻辑。工具层在此之上再包人类可读文本。
pub(crate) fn generate_video(
    req: &GenerationRequest,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<Artifact, GenError> {
    let registry =
        load_registry().map_err(|e| GenError::protocol(format!("加载模型配置失败: {e}")))?;
    let (target, protocol) =
        resolve_target(&registry, req.kind, provider_override, model_override)?;
    let request = normalize_media_inputs(req)?;
    protocol
        .video(&target, &request)?
        .into_iter()
        .next()
        .ok_or_else(|| GenError::empty_result("视频生成未返回任何产物"))
}

/// 语义层输入归一化：协议层只该看到「服务端可取到的引用」。
///
/// 目前唯一的媒体输入是视频首帧。本地文件由 [`media::image_ref_to_data_uri`]
/// 统一转 data URI；空串归一为「无首帧」（文生视频）。这一步刻意放在语义层，
/// 任何协议实现都不需要知道「本地路径」这种形态。
fn normalize_media_inputs(req: &GenerationRequest) -> Result<GenerationRequest, GenError> {
    let Some(source) = req.first_frame.as_deref() else {
        return Ok(req.clone());
    };
    let mut normalized = req.clone();
    normalized.first_frame = if source.trim().is_empty() {
        None
    } else {
        Some(media::image_ref_to_data_uri(source)?)
    };
    Ok(normalized)
}

// ── 工具输出文案（对外契约，逐字不变）──

/// 图像工具输出文案：`已生成 N 张图像：\n路径 (bytes)`，多张按换行拼接。
fn format_image_tool_text(artifacts: &[Artifact]) -> String {
    let lines: Vec<String> = artifacts
        .iter()
        .map(|artifact| format!("{} ({} bytes)", artifact.path.display(), artifact.bytes))
        .collect();
    format!("已生成 {} 张图像：\n{}", lines.len(), lines.join("\n"))
}

/// 视频工具输出文案：`视频生成完成（task_id=...）：\n路径 (bytes)`。
fn format_video_tool_text(artifact: &Artifact) -> String {
    format!(
        "视频生成完成（task_id={}）：\n{} ({} bytes)",
        artifact.task_id.as_deref().unwrap_or_default(),
        artifact.path.display(),
        artifact.bytes
    )
}

fn run_image_generate(
    prompt: &str,
    provider_override: Option<&str>,
    model_override: Option<&str>,
    params: Option<&Value>,
) -> Result<String, String> {
    let mut request = GenerationRequest::new(GenerationKind::Image, prompt);
    request.extra =
        GenerationRequest::extra_from_params(params).map_err(|error| error.to_string())?;
    let artifacts =
        generate_images(&request, provider_override, model_override).map_err(|e| e.to_string())?;
    Ok(format_image_tool_text(&artifacts))
}

fn run_video_generate(
    prompt: &str,
    provider_override: Option<&str>,
    model_override: Option<&str>,
    params: Option<&Value>,
    first_frame: Option<&str>,
    poll_interval_secs: u64,
    timeout_secs: u64,
) -> Result<String, String> {
    let mut request = GenerationRequest::new(GenerationKind::Video, prompt);
    request.extra =
        GenerationRequest::extra_from_params(params).map_err(|error| error.to_string())?;
    request.first_frame = first_frame.map(|value| value.to_string());
    request.poll_interval_secs = Some(poll_interval_secs);
    request.timeout_secs = Some(timeout_secs);
    let artifact =
        generate_video(&request, provider_override, model_override).map_err(|e| e.to_string())?;
    Ok(format_video_tool_text(&artifact))
}

// ── 工具注册 ──

impl ToolRegistry {
    pub(crate) fn register_image_generate(&mut self) {
        self.register(ToolDef {
            name: "image_generate".to_string(),
            description: "生成图像（同步返回）。产出为本地文件路径（~/.nuphus/generated/），可被后续步骤 capture 引用。模型默认取自模型界面绑定的「图片生成」能力（模型 → 图像音频模型 → 图片生成），未绑定会直接报错并列出可绑候选——不做任何静默发现。model 参数为当次覆盖（可填任意已注册的生成模型）；provider 参数为当次覆盖服务商归属（同模型名跨服务商时用来消歧）。params 为可选 JSON 对象，是提供商特定附加参数：键名与取值由所选提供商的生成协议决定，原样透传合并进请求体，与顶层参数同名时顶层优先。凭证（api_key）来自绑定模型所属的服务商，无需也不应传入密钥。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "图像生成提示词"
                    },
                    "provider": {
                        "type": "string",
                        "description": "可选。当次覆盖绑定的服务商名（同模型名跨服务商时消歧）；不填则用模型界面绑定的服务商归属"
                    },
                    "model": {
                        "type": "string",
                        "description": "可选。当次覆盖绑定的生成模型（可填任意已注册的生成模型）；不填则用模型界面绑定的模型"
                    },
                    "params": {
                        "type": "object",
                        "description": "可选。提供商特定附加参数（JSON 对象）：键名与取值由所选提供商的生成协议决定。原样透传合并进请求体，与顶层参数同名时顶层优先"
                    }
                },
                "required": ["prompt"]
            }),
            category: ToolCategory::WebSearch,
            executor: |params, _ctx| {
                let prompt = params
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if prompt.is_empty() {
                    return Ok(ToolResult::failure("prompt cannot be empty"));
                }
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string());
                // model 只作当次覆盖：不填 → 读模型界面绑定的图片生成模型
                let model = params
                    .get("model")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string());
                let extra = params.get("params").cloned();
                super::run_blocking(move || {
                    match run_image_generate(
                        &prompt,
                        provider.as_deref(),
                        model.as_deref(),
                        extra.as_ref(),
                    ) {
                        Ok(text) => Ok(ToolResult::success(text)),
                        Err(e) => Ok(ToolResult::failure(e)),
                    }
                })
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_video_generate(&mut self) {
        self.register(ToolDef {
            name: "video_generate".to_string(),
            description: "生成视频（异步任务轮询，分钟级耗时）。产出为本地 mp4 文件路径（~/.nuphus/generated/），可被后续步骤 capture 引用。模型默认取自模型界面绑定的「视频生成」能力（模型 → 图像音频模型 → 视频生成），未绑定会直接报错并列出可绑候选——不做任何静默发现。model 参数为当次覆盖（可填任意已注册的视频生成模型）；provider 参数为当次覆盖服务商归属（同模型名跨服务商时用来消歧）。first_frame 为可选首帧图：本地图片路径（自动读取并转 data URI）、公网 URL 或 data URI，留空 = 文生视频。params 为可选 JSON 对象，是提供商特定附加参数（如时长/清晰度）：键名与取值由所选提供商的生成协议决定，原样透传合并进请求体，与顶层参数同名时顶层优先。轮询有超时兜底（timeout_secs），超时/失败错误含任务号便于排查。凭证（api_key）来自绑定模型所属的服务商，无需也不应传入密钥。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "视频生成提示词"
                    },
                    "provider": {
                        "type": "string",
                        "description": "可选。当次覆盖绑定的服务商名（同模型名跨服务商时消歧）；不填则用模型界面绑定的服务商归属"
                    },
                    "model": {
                        "type": "string",
                        "description": "可选。当次覆盖绑定的视频生成模型（可填任意已注册的生成模型）；不填则用模型界面绑定的模型"
                    },
                    "first_frame": {
                        "type": "string",
                        "description": "可选。首帧图：本地图片路径（自动转 data URI）、公网 URL 或 data URI；留空 = 文生视频"
                    },
                    "params": {
                        "type": "object",
                        "description": "可选。提供商特定附加参数（JSON 对象）：键名与取值由所选提供商的生成协议决定。原样透传合并进请求体，与顶层参数同名时顶层优先"
                    },
                    "poll_interval_secs": {
                        "type": "integer",
                        "default": DEFAULT_POLL_INTERVAL_SECS,
                        "description": "轮询间隔秒数，默认 5"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "default": DEFAULT_VIDEO_TIMEOUT_SECS,
                        "description": "轮询总超时秒数，默认 300，上限 600"
                    }
                },
                "required": ["prompt"]
            }),
            category: ToolCategory::WebSearch,
            executor: |params, _ctx| {
                let prompt = params
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if prompt.is_empty() {
                    return Ok(ToolResult::failure("prompt cannot be empty"));
                }
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string());
                let model = params
                    .get("model")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string());
                let extra = params.get("params").cloned();
                let first_frame = params
                    .get("first_frame")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                let poll_interval_secs = params
                    .get("poll_interval_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_POLL_INTERVAL_SECS)
                    .clamp(1, 60);
                let timeout_secs = params
                    .get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_VIDEO_TIMEOUT_SECS)
                    .clamp(30, MAX_VIDEO_TIMEOUT_SECS);
                super::run_blocking(move || {
                    match run_video_generate(
                        &prompt,
                        provider.as_deref(),
                        model.as_deref(),
                        extra.as_ref(),
                        first_frame.as_deref(),
                        poll_interval_secs,
                        timeout_secs,
                    ) {
                        Ok(text) => Ok(ToolResult::success(text)),
                        Err(e) => Ok(ToolResult::failure(e)),
                    }
                })
            },
            depends_on: vec![],
        });
    }
}

// ── 单测（不真调 API；真实调用验证走 ignored 测试手动执行）──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{ModelEntry, ModelSource, ProviderConfig};
    use crate::config::Capabilities;

    // ── fixtures ──

    fn model_entry(id: &str, supports_image_generation: bool) -> ModelEntry {
        ModelEntry {
            transport: None,
            responses: None,
            id: id.to_string(),
            alias: vec![],
            max_tokens: None,
            context_window: None,
            supports_streaming: true,
            supports_vision: false,
            supports_audio: false,
            supports_image_generation,
            reasoning_efforts: vec![],
            default_effort: None,
            cost_per_million_in: None,
            cost_per_million_out: None,
            source: ModelSource::Auto,
            context_window_source: None,
            max_output_tokens: None,
            max_output_tokens_source: None,
        }
    }

    fn provider(
        name: &str,
        kind: ProviderKind,
        api_key: &str,
        base_url: &str,
        models: Vec<ModelEntry>,
    ) -> ProviderConfig {
        ProviderConfig {
            name: name.to_string(),
            provider_type: kind,
            api_key: api_key.to_string(),
            base_url: base_url.to_string(),
            auth_header: String::new(),
            auth_prefix: String::new(),
            timeout_secs: 300,
            models,
            reasoning_effort: None,
            extra_headers: std::collections::BTreeMap::new(),
            oauth: None,
        }
    }

    fn minimax_provider(name: &str, api_key: &str, models: Vec<ModelEntry>) -> ProviderConfig {
        provider(name, ProviderKind::MiniMax, api_key, "", models)
    }

    fn registry_with(providers: Vec<ProviderConfig>, caps: Capabilities) -> ModelRegistry {
        let mut registry = ModelRegistry::default();
        registry.providers = providers;
        registry.capabilities = caps;
        registry
    }

    fn bound_caps(model: &str, provider: &str) -> Capabilities {
        Capabilities {
            image_generation: model.to_string(),
            image_generation_provider: provider.to_string(),
            ..Default::default()
        }
    }

    /// 两个段都发布 `image-01`（同名跨段），绑定指向 custom-gw；
    /// custom-gw 另有一个 `image-02` 供「当次覆盖模型」测试。
    fn duplicate_id_registry() -> ModelRegistry {
        registry_with(
            vec![
                minimax_provider(
                    "minimax",
                    "minimax-key",
                    vec![model_entry("image-01", true)],
                ),
                provider(
                    "custom-gw",
                    ProviderKind::Custom,
                    "gw-key",
                    "https://gw.example/v1",
                    vec![model_entry("image-01", true), model_entry("image-02", true)],
                ),
            ],
            bound_caps("image-01", "custom-gw"),
        )
    }

    /// 未绑定任何生成能力，但注册表里有声明了生成的模型（候选可列）。
    fn unbound_image_registry() -> ModelRegistry {
        registry_with(
            vec![minimax_provider(
                "minimax",
                "minimax-key",
                vec![model_entry("image-01", true)],
            )],
            Capabilities::default(),
        )
    }

    /// 未绑定 + 同模型名跨段：只给模型名时无法确定归属。
    fn unbound_duplicate_id_registry() -> ModelRegistry {
        registry_with(
            vec![
                minimax_provider(
                    "minimax",
                    "minimax-key",
                    vec![model_entry("image-01", true)],
                ),
                provider(
                    "custom-gw",
                    ProviderKind::Custom,
                    "gw-key",
                    "https://gw.example/v1",
                    vec![model_entry("image-01", true)],
                ),
            ],
            Capabilities::default(),
        )
    }

    // ── extra 透传（params → GenerationRequest.extra）──

    #[test]
    fn extra_from_params_accepts_object_and_none() {
        assert!(GenerationRequest::extra_from_params(None)
            .expect("None means no extra")
            .is_empty());
        let extra =
            GenerationRequest::extra_from_params(Some(&json!({"seed": 7}))).expect("object ok");
        assert_eq!(extra.get("seed"), Some(&json!(7)));
    }

    #[test]
    fn extra_from_params_rejects_non_object() {
        let err = GenerationRequest::extra_from_params(Some(&json!("not-an-object")))
            .expect_err("non-object params must be rejected");
        assert_eq!(err.kind, GenErrorKind::Protocol);
        assert_eq!(err.to_string(), "params must be a JSON object");
    }

    // ── 协议选择 ──

    /// 未实现协议的 provider → unsupported_provider，且错误信息不含任何端点。
    #[test]
    fn unsupported_provider_is_reported_without_endpoint() {
        let registry = registry_with(
            vec![provider(
                "custom-gw",
                ProviderKind::Custom,
                "k1",
                "https://gw.example/v1",
                vec![model_entry("image-01", true)],
            )],
            bound_caps("image-01", "custom-gw"),
        );
        let err = match resolve_target(&registry, GenerationKind::Image, None, None) {
            // GenTarget 含 api_key 刻意不实现 Debug，匹配后只取错误
            Ok(_) => panic!("provider without a protocol must fail loudly"),
            Err(e) => e,
        };
        assert_eq!(err.kind, GenErrorKind::UnsupportedProvider);
        let text = err.to_string();
        assert!(
            text.contains("custom-gw"),
            "error should name the provider: {text}"
        );
        assert!(
            text.contains("尚未实现生成协议"),
            "error should say why it failed: {text}"
        );
        assert!(
            text.contains("minimax_media"),
            "error should list the supported protocols: {text}"
        );
        for leak in ["http://", "https://", "/v1", "/v2", "api.minimaxi"] {
            assert!(
                !text.contains(leak),
                "error must not leak endpoint detail ({leak}): {text}"
            );
        }
    }

    /// MiniMax 段未配 base_url → 回落协议内置默认端点。
    #[test]
    fn minimax_provider_gets_protocol_default_base_url() {
        let registry = registry_with(
            vec![minimax_provider(
                "minimax",
                "k1",
                vec![model_entry("image-01", true)],
            )],
            bound_caps("image-01", "minimax"),
        );
        let (target, protocol) = resolve_target(&registry, GenerationKind::Image, None, None)
            .expect("minimax resolves with the protocol default endpoint");
        assert_eq!(protocol.kind(), "minimax_media");
        assert_eq!(
            target.base_url,
            MiniMaxMediaProtocol
                .default_base_url()
                .expect("minimax has a default endpoint")
        );
    }

    // ── 工具输出文案契约（结构化重构后必须逐字不变） ──

    #[test]
    fn image_tool_text_matches_verbatim_contract() {
        let one = std::path::PathBuf::from("/tmp/generated/image_1_0.png");
        let two = std::path::PathBuf::from("/tmp/generated/image_1_1.png");
        let single = format_image_tool_text(&[Artifact {
            path: one.clone(),
            bytes: 2048,
            task_id: None,
        }]);
        assert_eq!(
            single,
            format!("已生成 1 张图像：\n{} (2048 bytes)", one.display())
        );
        let multi = format_image_tool_text(&[
            Artifact {
                path: one.clone(),
                bytes: 2048,
                task_id: None,
            },
            Artifact {
                path: two.clone(),
                bytes: 4096,
                task_id: None,
            },
        ]);
        assert_eq!(
            multi,
            format!(
                "已生成 2 张图像：\n{} (2048 bytes)\n{} (4096 bytes)",
                one.display(),
                two.display()
            )
        );
    }

    #[test]
    fn video_tool_text_matches_verbatim_contract() {
        let text = format_video_tool_text(&Artifact {
            path: std::path::PathBuf::from("/tmp/generated/video_1.mp4"),
            bytes: 99,
            task_id: Some("task-42".to_string()),
        });
        assert_eq!(
            text,
            "视频生成完成（task_id=task-42）：\n/tmp/generated/video_1.mp4 (99 bytes)"
        );
    }

    // ── 凭证解析：消费能力绑定（不再自行发现任何 provider）──

    /// 不用 unwrap：GenTarget 含 api_key，刻意不实现 Debug 防泄露
    fn resolve_err(
        registry: &ModelRegistry,
        kind: GenerationKind,
        provider: Option<&str>,
        model: Option<&str>,
    ) -> String {
        match resolve_generation_target(registry, kind, provider, model) {
            Ok(_) => panic!("expected an error, got a credential — silent discovery is forbidden"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn credential_comes_from_the_capability_binding() {
        let registry = duplicate_id_registry();
        let target = resolve_generation_target(&registry, GenerationKind::Image, None, None)
            .expect("binding should resolve");
        // 凭证取自**绑定**的那个段，而不是段序第一个段
        assert_eq!(target.api_key, "gw-key");
        assert_eq!(target.base_url, "https://gw.example/v1");
        assert_eq!(target.model, "image-01");
    }

    #[test]
    fn credential_resolves_the_exact_bound_provider_across_duplicate_ids() {
        // 同名模型跨段：绑定点名 custom-gw → 必须用 gw-key，绝不能被 minimax 抢先
        let registry = duplicate_id_registry();
        let target =
            resolve_generation_target(&registry, GenerationKind::Image, None, None).unwrap();
        assert_eq!(target.api_key, "gw-key");

        // 绑定改成 minimax → 换成 minimax-key（同一份注册表，绑定即真相）
        let mut rebound = duplicate_id_registry();
        rebound.capabilities.image_generation_provider = "minimax".to_string();
        let target =
            resolve_generation_target(&rebound, GenerationKind::Image, None, None).unwrap();
        assert_eq!(target.api_key, "minimax-key");
        // 该段没配 base_url → 生效端点由协议默认提供（语义层不写死任何端点）
        let (target, _protocol) =
            resolve_target(&rebound, GenerationKind::Image, None, None).unwrap();
        assert_eq!(
            target.base_url,
            MiniMaxMediaProtocol
                .default_base_url()
                .expect("minimax default endpoint")
        );
    }

    #[test]
    fn credential_model_override_beats_binding() {
        let registry = duplicate_id_registry();
        let target =
            resolve_generation_target(&registry, GenerationKind::Image, None, Some("image-02"))
                .expect("override model should resolve");
        assert_eq!(target.model, "image-02");
        // 绑定 provider 仍是归属来源
        assert_eq!(target.api_key, "gw-key");
    }

    #[test]
    fn credential_provider_override_beats_binding() {
        let registry = duplicate_id_registry();
        let target =
            resolve_generation_target(&registry, GenerationKind::Image, Some("minimax"), None)
                .expect("provider override should resolve");
        assert_eq!(target.api_key, "minimax-key");
        assert_eq!(target.model, "image-01");
    }

    #[test]
    fn credential_video_binding_is_independent_of_image() {
        let registry = registry_with(
            vec![minimax_provider(
                "minimax",
                "minimax-key",
                vec![model_entry("MiniMax-H3", false)],
            )],
            Capabilities {
                image_generation: "image-01".to_string(),
                image_generation_provider: "minimax".to_string(),
                video_generation: "MiniMax-H3".to_string(),
                video_generation_provider: "minimax".to_string(),
                ..Default::default()
            },
        );
        // 悬空绑定（模型不在该 provider 段里）必须报错，而不是拿 model id 去打请求。
        // 不用 expect_err：GenTarget 含 api_key，刻意不实现 Debug 防泄露。
        let image = match resolve_generation_target(&registry, GenerationKind::Image, None, None) {
            Ok(_) => panic!("stale image binding must not resolve"),
            Err(e) => e.to_string(),
        };
        assert!(
            image.contains("image-01"),
            "error should name the model: {image}"
        );

        let video = resolve_generation_target(&registry, GenerationKind::Video, None, None)
            .expect("video binding should resolve");
        assert_eq!(video.model, "MiniMax-H3");
        assert_eq!(video.api_key, "minimax-key");
    }

    #[test]
    fn credential_unbound_reports_binding_hint_and_candidates() {
        let registry = unbound_image_registry();
        let err = resolve_err(&registry, GenerationKind::Image, None, None);
        assert!(
            err.contains("模型界面"),
            "error should point at the model UI: {err}"
        );
        assert!(
            err.contains("图片生成"),
            "error should name the capability: {err}"
        );
        assert!(
            err.contains("minimax/image-01"),
            "error should list the bindable candidate: {err}"
        );
    }

    #[test]
    fn credential_ambiguous_same_id_without_provider_is_reported() {
        // 没绑定、又只给了模型名 → 重名必须报错并列候选，绝不挑段序第一个
        let registry = unbound_duplicate_id_registry();
        let err = resolve_err(&registry, GenerationKind::Image, None, Some("image-01"));
        assert!(
            err.contains("重名"),
            "error should surface the ambiguity: {err}"
        );
        assert!(
            err.contains("minimax"),
            "error should list the first candidate: {err}"
        );
        assert!(
            err.contains("custom-gw"),
            "error should list the second candidate: {err}"
        );
        assert!(
            err.contains("模型界面"),
            "error should tell the user how to fix it: {err}"
        );
    }

    #[test]
    fn credential_unknown_provider_name_is_reported() {
        let registry = duplicate_id_registry();
        let err = resolve_err(&registry, GenerationKind::Image, Some("nope"), None);
        assert!(
            err.contains("nope"),
            "error should name the provider: {err}"
        );
        assert!(
            err.contains("没有这个模型条目"),
            "error should say why it failed: {err}"
        );
    }

    #[test]
    fn credential_unbound_with_no_candidate_says_how_to_declare_one() {
        // 注册表里没有任何模型声明生成能力 → 文案引导去模型行打开开关，
        // 而不是静默退回某个 provider。
        let registry = registry_with(
            vec![minimax_provider(
                "minimax",
                "minimax-key",
                vec![model_entry("MiniMax-M3", false)],
            )],
            Capabilities::default(),
        );
        let err = resolve_err(&registry, GenerationKind::Image, None, None);
        assert!(
            err.contains("模型界面"),
            "error should point at the model UI: {err}"
        );
        assert!(
            err.contains("还没有模型声明"),
            "error should explain the empty candidate set: {err}"
        );
    }

    #[test]
    fn credential_unbound_video_candidate_list_is_not_capability_gated() {
        // 视频生成在注册表里没有能力声明字段 → 候选不按谓词过滤（不空口断言支持）
        let registry = registry_with(
            vec![minimax_provider(
                "minimax",
                "minimax-key",
                vec![model_entry("MiniMax-H3", false)],
            )],
            Capabilities::default(),
        );
        let target =
            resolve_generation_target(&registry, GenerationKind::Video, None, Some("MiniMax-H3"))
                .expect("explicit video model should resolve without a capability flag");
        assert_eq!(target.model, "MiniMax-H3");
    }

    #[test]
    fn credential_empty_api_key_rejected() {
        let registry = registry_with(
            vec![minimax_provider(
                "minimax",
                "  ",
                vec![model_entry("image-01", true)],
            )],
            Capabilities {
                image_generation: "image-01".to_string(),
                image_generation_provider: "minimax".to_string(),
                ..Default::default()
            },
        );
        assert!(resolve_generation_target(&registry, GenerationKind::Image, None, None).is_err());
    }

    #[test]
    fn credential_base_url_trims_trailing_slash() {
        let registry = registry_with(
            vec![provider(
                "custom-gw",
                ProviderKind::Custom,
                "k1",
                "https://gw.example/v1/",
                vec![model_entry("image-01", true)],
            )],
            Capabilities {
                image_generation: "image-01".to_string(),
                image_generation_provider: "custom-gw".to_string(),
                ..Default::default()
            },
        );
        let target =
            resolve_generation_target(&registry, GenerationKind::Image, None, None).unwrap();
        assert_eq!(target.base_url, "https://gw.example/v1");
    }

    // ── wf_tools 包含性：注册表含两工具、prompt 必填、不在工作流排除名单 ──

    #[test]
    fn workflow_tool_inclusion() {
        let registry = ToolRegistry::builtin();
        for name in ["image_generate", "video_generate"] {
            let def = registry
                .get(name)
                .unwrap_or_else(|| panic!("{name} should be registered"));
            let required = def
                .parameters
                .get("required")
                .and_then(|v| v.as_array())
                .expect("input_schema should declare required");
            assert_eq!(
                required
                    .iter()
                    .map(|v| v.as_str().unwrap_or_default())
                    .collect::<Vec<_>>(),
                vec!["prompt"],
                "{name} input_schema should require prompt only (provider/model are overrides)"
            );
            for optional in ["provider", "model"] {
                let prop = def
                    .parameters
                    .get("properties")
                    .and_then(|p| p.get(optional))
                    .unwrap_or_else(|| panic!("{name} should keep the {optional} property"));
                assert!(
                    prop.get("required").is_none(),
                    "{name}.{optional} must not be required"
                );
                // 不得再带 schema 级 default：客户端会自动回填 default，从而
                // 无声地覆盖模型界面的绑定。
                assert!(
                    prop.get("default").is_none(),
                    "{name}.{optional} must not declare a default that would silently override the binding"
                );
            }
        }
    }

    /// `provider` / `model` 都是**可选**参数：默认读模型界面的能力绑定，
    /// 只有 LLM 想当次覆盖时才传。required 必须回到只有 `["prompt"]`。
    #[test]
    fn generation_schema_only_requires_prompt() {
        let registry = ToolRegistry::builtin();
        for name in ["image_generate", "video_generate"] {
            let def = registry
                .get(name)
                .unwrap_or_else(|| panic!("{name} should be registered"));
            let required = def
                .parameters
                .get("required")
                .and_then(|v| v.as_array())
                .expect("input_schema should declare required");
            assert_eq!(
                required
                    .iter()
                    .map(|v| v.as_str().unwrap_or_default())
                    .collect::<Vec<_>>(),
                vec!["prompt"],
                "{name} input_schema should require prompt only (provider/model are overrides)"
            );
            for optional in ["provider", "model"] {
                let prop = def
                    .parameters
                    .get("properties")
                    .and_then(|p| p.get(optional))
                    .unwrap_or_else(|| panic!("{name} should keep the {optional} property"));
                assert!(
                    prop.get("required").is_none(),
                    "{name}.{optional} must not be required"
                );
                // 不得再带 schema 级 default：客户端会自动回填 default，从而
                // 无声地覆盖模型界面的绑定。
                assert!(
                    prop.get("default").is_none(),
                    "{name}.{optional} must not declare a default that would silently override the binding"
                );
            }
        }
    }

    // ── 契约层零厂商泄漏守卫 ──

    /// 工具 description / workflow schema 的 DoModel 段 / model 表单 locale 文案
    /// 不得出现任何厂商标识、端点、厂商参数名或型号示例——这是「语义层与厂商
    /// 协议层分离」的回归护栏。
    #[test]
    fn contract_layer_has_no_vendor_leak() {
        const FORBIDDEN: &[&str] = &[
            "MiniMax",
            "minimax",
            "aspect_ratio",
            "image_urls",
            "files/retrieve",
            "image-01",
            "Hailuo",
            "S2V-01",
            "512-2048",
            "512–2048",
            "V2 端点",
            "response_format",
        ];
        let mut leaks: Vec<String> = Vec::new();
        let scan = |surface: &str, text: &str, leaks: &mut Vec<String>| {
            for pattern in FORBIDDEN {
                if text.contains(pattern) {
                    leaks.push(format!("{surface}: {pattern}"));
                }
            }
        };

        // 1. 工具对外契约：description + 参数 schema 描述
        let registry = ToolRegistry::builtin();
        for name in ["image_generate", "video_generate"] {
            let def = registry
                .get(name)
                .unwrap_or_else(|| panic!("{name} should be registered"));
            let mut text = def.description.clone();
            text.push_str(&serde_json::to_string(&def.parameters).unwrap_or_default());
            scan(name, &text, &mut leaks);
        }

        // 2. workflow schema 的 DoModel 段
        let schema_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("workflow/step_schema.json");
        let schema_text = std::fs::read_to_string(&schema_path).expect("step_schema.json readable");
        let schema: Value = serde_json::from_str(&schema_text).expect("valid json schema");
        let do_model =
            serde_json::to_string(&schema["$defs"]["DoModel"]).expect("DoModel serializable");
        scan("step_schema.json:$defs.DoModel", &do_model, &mut leaks);

        // 3. model 表单相关 locale 文案（工作流编辑器语言包，model 表单词条都在这里）
        let locale_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../frontend/src/locales/workflowEditor.ts");
        let locale = std::fs::read_to_string(&locale_path).expect("workflowEditor.ts readable");
        scan("locales/workflowEditor.ts", &locale, &mut leaks);

        assert!(
            leaks.is_empty(),
            "契约层厂商泄漏（{}）：{}",
            leaks.len(),
            leaks.join("; ")
        );
    }

    // ── 真实 API 验证（手动执行，需 registry 已配置生成能力绑定）──
    // cargo test -p nuphus generation::tests::image_generate_real_api -- --ignored --nocapture

    #[test]
    #[ignore = "real generation API call — run manually"]
    fn image_generate_real_api() {
        let registry = ToolRegistry::builtin();
        let def = registry.get("image_generate").unwrap();
        let result = (def.executor)(
            &json!({"prompt": "a cute cat sitting on a windowsill, warm morning light"}),
            &crate::tools::registry::ToolCtx::default(),
        )
        .unwrap();
        assert!(result.success, "image_generate failed: {:?}", result.error);
        println!("{}", result.output.unwrap());
    }
}
