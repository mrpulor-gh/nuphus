//! desktop_perceive 端到端冒烟（验证步骤 3 完整版）
//!
//! 用「社区 12MB onnx」冒充 icon_detect.onnx，跑一次真实截图 OCR + YOLO，
//! 确认社区模型在完整 perceive_image 链路上工作（输入尺寸/输出结构与
//! yolo.rs 匹配）。模型目录用 NUPHUS_MODELS_DIR 指向临时目录，不污染本机。
//!
//! 运行：
//!   cargo run -p desktop-api --example perceive_smoke
//!
//! 环境变量（可选）：
//!   YOLO_SMOKE_COMMUNITY=<12MB onnx>    PS_SMOKE_OCR=<OCR 模型目录>
//! 退出码：0 = OCR+YOLO 全链路通过；1 = 失败

use std::path::PathBuf;

fn main() {
    // ── 定位文件 ─────────────────────────────────────────────
    let src_tauri = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let community = env_path("YOLO_SMOKE_COMMUNITY", || {
        std::env::temp_dir().join("yolo_smoke/community_icon_detect.onnx")
    });
    let ocr_dir = env_path("PS_SMOKE_OCR", || src_tauri.join("desktop/models"));

    // onnxruntime.dll 所在目录（load-dynamic 需要）
    let ort_dir = src_tauri.join("desktop/sherpa");
    if let Ok(cur) = std::env::var("PATH") {
        std::env::set_var("PATH", format!("{};{}", ort_dir.display(), cur));
    }
    let ort_dll = ort_dir.join("onnxruntime.dll");
    std::env::set_var("ORT_DYLIB_PATH", &ort_dll);

    if !community.exists() {
        eprintln!("[FATAL] 社区 onnx 不存在: {}", community.display());
        std::process::exit(2);
    }
    if !ocr_dir.join("ch_PP-OCRv4_det.onnx").exists() {
        eprintln!("[FATAL] OCR 模型目录缺 det: {}", ocr_dir.display());
        std::process::exit(2);
    }

    // ── 建临时模型目录：OCR 原样 + YOLO 换成社区模型 ──────────
    let models_dir = std::env::temp_dir().join("yolo_smoke/perceive_models");
    let _ = std::fs::remove_dir_all(&models_dir);
    std::fs::create_dir_all(&models_dir).expect("mkdir");
    for f in [
        "ch_PP-OCRv4_det.onnx",
        "ch_PP-OCRv4_rec.onnx",
        "ch_PP-OCR_keys_v1.txt",
    ] {
        std::fs::copy(ocr_dir.join(f), models_dir.join(f))
            .unwrap_or_else(|e| panic!("copy {f}: {e}"));
    }
    std::fs::copy(&community, models_dir.join("icon_detect.onnx"))
        .unwrap_or_else(|e| panic!("copy icon_detect.onnx: {e}"));
    std::env::set_var("NUPHUS_MODELS_DIR", &models_dir);
    println!("模型目录(临时): {}", models_dir.display());
    println!("icon_detect.onnx ← {}", community.display());

    // ── 真实截图 ─────────────────────────────────────────────
    let runtime = tokio::runtime::Runtime::new().expect("tokio");
    // Scope::Fullscreen 时 capture 忽略 target，传占位即可
    let dummy_target = desktop_api::Target::Browser {
        page_id: String::new(),
        url: String::new(),
    };
    let frame = runtime
        .block_on(desktop_api::vision::capture::capture(
            &dummy_target,
            desktop_api::Scope::Fullscreen,
        ))
        .expect("screenshot");

    // Frame → PNG（perceive_image 走文件路径）
    let png_path = models_dir.join("screenshot.png");
    image::DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(frame.width, frame.height, frame.pixels).expect("rgba"),
    )
    .save(&png_path)
    .expect("save png");
    println!(
        "截图: {} ({}x{})",
        png_path.display(),
        frame.width,
        frame.height
    );

    // ── 端到端 perceive_image（OCR + YOLO 合并）──────────────
    let out = match desktop_api::vision::perceive::perceive_image(&png_path.to_string_lossy()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[FAIL] perceive_image 错误: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "\n[RESULT] elements={} ocr={} yolo={} yolo_available={}",
        out.elements.len(),
        out.ocr_count,
        out.yolo_count,
        out.yolo_available
    );
    for e in out.elements.iter().take(10) {
        println!(
            "  #{} {:?} {:?} text={:?} conf={:.2}",
            e.id, e.kind, e.source, e.text, e.confidence
        );
    }

    let pass = out.yolo_available && out.yolo_count > 0 && out.ocr_count > 0;
    if pass {
        println!("\n[PASS] 社区 onnx + OCR 全链路工作，desktop_perceive 结构匹配");
    } else {
        println!(
            "\n[FAIL] ocr={} yolo={} yolo_available={} —— 有环节缺失",
            out.ocr_count, out.yolo_count, out.yolo_available
        );
    }
    std::process::exit(if pass { 0 } else { 1 });
}

fn env_path(key: &str, default: impl Fn() -> PathBuf) -> PathBuf {
    std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(default)
}
