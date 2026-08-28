//! Preload commands — warm up models and resources at startup

use tauri::AppHandle;

/// Load the Candle embedding model (bge-small-zh) at startup, BLOCKING until
/// the model is ready (downloads it first if missing). Download progress is
/// reported to the splash via `splash:progress`.
/// Returns true once loaded (or if already loaded).
///
/// 重阻塞（模型下载 + Candle 加载）必须放到 spawn_blocking：
/// 直接在主线程执行会让进度事件排队到返回后才投递，splash 进度就死了。
#[tauri::command]
pub async fn preload_model(app: AppHandle) -> Result<bool, String> {
    tracing::info!("[Preload] Starting embedding model load...");

    let worker_app = app.clone();
    // 返回 bool 而非 &'static Embedder，避免 spawn_blocking 的 Send 边界
    // 依赖 Embedder: Sync；is_some() 即代表全局已就绪。
    let loaded = tauri::async_runtime::spawn_blocking(move || {
        let mut on_progress = |downloaded: u64, total: u64, file: &str| {
            let pct =
                (total > 0).then(|| ((downloaded as f64 / total as f64) * 100.0).round() as u8);
            let msg = match pct {
                Some(p) => format!("正在下载模型… {file} {p}%"),
                None => format!("正在下载模型… {file}"),
            };
            crate::splash::emit_splash_progress(&worker_app, pct, &msg);
        };
        // get_with_progress: 首次调用自动下载（进度回调推进）；已加载则立即返回不回调。
        nuphus::embed::Embedder::get_with_progress(&mut on_progress).is_some()
    })
    .await
    .map_err(|e| format!("嵌入模型预加载任务失败: {e}"))?;

    if loaded {
        tracing::info!("[Preload] Embedding model loaded successfully");
        // 阶段完成信号：pct=null 强制 splash 收尾（撤下载载条与「后台下载」
        // 按钮）。没有这条，下载路径的最后一个数值 pct 会把加载条/按钮
        // 留在屏幕上直到窗口关闭——"下载完了还挂着后台下载"的根源之一。
        crate::splash::emit_splash_progress(&app, None, "嵌入模型就绪");
        Ok(true)
    } else {
        tracing::warn!("[Preload] Embedding model failed to load (will lazy-init on first use)");
        crate::splash::emit_splash_progress(&app, None, "继续启动…");
        Ok(false)
    }
}

/// Ensure vision models (PaddleOCR + YOLO) are present, BLOCKING until done:
/// if any file is missing it downloads synchronously (progress via
/// `splash:progress`), and returns only when all models are ready or the
/// download terminally fails. This is what keeps the splash alive through the
/// first-run model download.
///
/// Also serves as the ModelsPage "retry" entry point — the frontend invokes
/// this with no arguments; `AppHandle` is injected by Tauri.
#[tauri::command]
pub async fn preload_ocr(app: AppHandle) -> Result<bool, String> {
    let worker_app = app.clone();
    // 阻塞下载放到 spawn_blocking：保持主线程事件循环空闲，进度事件实时投递。
    let inner = tauri::async_runtime::spawn_blocking(move || {
        crate::models::bootstrap::ensure_vision_models_blocking(&worker_app)
    })
    .await
    .map_err(|e| format!("视觉模型预加载任务失败: {e}"))?;
    inner?;
    // 阶段完成信号：与 preload_model 同理，覆盖 skip / 内置采用 / 真实下载
    // 全部路径——视觉模型阶段结束时 splash 必须收尾。
    crate::splash::emit_splash_progress(&app, None, "视觉模型就绪");
    Ok(true)
}
