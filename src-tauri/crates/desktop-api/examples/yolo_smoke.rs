//! YOLO onnx 兼容性冒烟测试（验证步骤 3）
//!
//! 对比「本地 80MB 模型」与「社区 12MB 模型」（build.rs 与运行时 bootstrap 自动
//! 下载的源）。ort rc.12 的 Outlet 不公开 shape，因此用真实推理验证 schema：
//!   - 用 [1,3,640,640] f32 输入跑一次零帧推理（ORT 运行时会校验输入 shape，
//!     形状不符直接报错）→ 输入兼容性成立
//!   - 提取输出 array，检查结构是否为 [1,5,N]（每列 cx,cy,w,h,conf，yolo.rs 索引
//!     [0,4,i] 取 conf）；若为 [1,N,5] 则需调代码
//!
//! 运行（需 onnxruntime.dll 可加载，load-dynamic）：
//!   cargo run -p desktop-api --example yolo_smoke
//!
//! 用环境变量覆盖路径（默认取本机仓库布局）：
//!   YOLO_SMOKE_ORT=<onnxruntime.dll>    YOLO_SMOKE_COMMUNITY=<12MB onnx>
//!   YOLO_SMOKE_LOCAL=<80MB onnx>
//!
//! 退出码：0 = schema 兼容 + 推理跑通；1 = schema 不匹配（需调代码或换模型变体）。

use std::path::PathBuf;

fn main() {
    // ── 定位文件 ─────────────────────────────────────────────
    // CARGO_MANIFEST_DIR = …/src-tauri/crates/desktop-api → 上溯两级到 src-tauri
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_tauri = root.parent().unwrap().parent().unwrap();
    let ort_dll = env_or("YOLO_SMOKE_ORT", {
        src_tauri.join("desktop/sherpa/onnxruntime.dll")
    });
    let community = env_or("YOLO_SMOKE_COMMUNITY", {
        std::env::temp_dir().join("yolo_smoke/community_icon_detect.onnx")
    });
    let local = env_or("YOLO_SMOKE_LOCAL", {
        src_tauri.join("desktop/models/icon_detect.onnx")
    });

    for (name, p) in [
        ("ort", &ort_dll),
        ("community", &community),
        ("local", &local),
    ] {
        if !p.exists() {
            eprintln!("[FATAL] {name} 不存在: {}", p.display());
            std::process::exit(2);
        }
    }
    println!("onnxruntime : {}", ort_dll.display());
    println!("community   : {}", community.display());
    println!("local       : {}", local.display());

    // ── 设置 load-dynamic 搜索路径 ────────────────────────────
    // ort load-dynamic 在 Windows 上优先读 ORT_DYLIB_PATH。
    std::env::set_var("ORT_DYLIB_PATH", &ort_dll);
    // 兜底：把 dll 所在目录塞进 PATH，保证 onnxruntime_providers_shared.dll 也找得到。
    if let Some(dir) = ort_dll.parent() {
        if let Ok(cur) = std::env::var("PATH") {
            std::env::set_var("PATH", format!("{};{}", dir.display(), cur));
        }
    }

    // ── 加载两个模型并对比 schema ────────────────────────────
    let mut ok = true;
    ok &= inspect(&local, "local (80MB)");
    ok &= inspect(&community, "community (12MB)");

    if ok {
        println!("\n[PASS] schema 兼容：输入 [1,3,640,640] f32，输出 [1,5,N] f32");
    } else {
        println!(
            "\n[FAIL] schema 不兼容 —— 需调 yolo.rs 或换 fp16/int8 变体，最坏回退手动导出指引"
        );
    }
    std::process::exit(if ok { 0 } else { 1 });
}

/// 加载模型 → 打印 schema → 用 640×640 零帧跑一次推理 → 断言结构
fn inspect(path: &PathBuf, label: &str) -> bool {
    println!("\n===== {label}: {} =====", path.display());
    let mut session = match ort::session::Session::builder()
        .map_err(|e| format!("builder: {e}"))
        .and_then(|mut b| b.commit_from_file(path).map_err(|e| format!("load: {e}")))
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[FAIL] {label} 加载失败: {e}");
            return false;
        }
    };

    let inputs = session.inputs();
    let outputs = session.outputs();
    if inputs.is_empty() || outputs.is_empty() {
        eprintln!("[FAIL] {label} 无输入或无输出");
        return false;
    }
    let input_name = inputs[0].name().to_string();
    let output_name = outputs[0].name().to_string();
    println!(
        "  input  : name={} dtype={:?}",
        input_name,
        inputs[0].dtype()
    );
    println!(
        "  output : name={} dtype={:?}",
        output_name,
        outputs[0].dtype()
    );
    // inputs/outputs 引用到此为止，NLL 在 session.run(&mut) 前自动释放借用

    // ── 真实推理（640×640 零帧）──────────────────────────────
    // 输入 shape 正确性由 ORT 运行时校验：shape 不符直接 Err。
    let zeros = vec![0.0f32; 3 * 640 * 640];
    let arr = match ndarray::Array4::from_shape_vec((1, 3, 640, 640), zeros) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[FAIL] {label} ndarray 构建失败: {e}");
            return false;
        }
    };
    let input_value = match ort::value::TensorRef::from_array_view(arr.view()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[FAIL] {label} tensor 构建失败: {e}");
            return false;
        }
    };

    let start = std::time::Instant::now();
    let result = session.run(ort::inputs![input_name.as_str() => input_value]);
    match result {
        Ok(outputs_map) => {
            let out_value = match outputs_map.get(output_name.as_str()) {
                Some(v) => v,
                None => {
                    eprintln!("[FAIL] {label} 输出 key '{output_name}' 未找到");
                    return false;
                }
            };
            match out_value.try_extract_array::<f32>() {
                Ok(view) => {
                    let s = view.shape().to_vec();
                    let elapsed = start.elapsed().as_millis();
                    println!("  infer  : 成功, {}ms, out_shape={:?}", elapsed, s);
                    check_output_structure(&s, label)
                }
                Err(e) => {
                    eprintln!("[FAIL] {label} 输出提取为 f32 失败: {e}");
                    false
                }
            }
        }
        Err(e) => {
            eprintln!("[FAIL] {label} 推理失败: {e}");
            false
        }
    }
}

/// 检查输出结构是否为 [1,5,N]（与 yolo.rs 的 `view[[0,4,i]]` 索引一致）
fn check_output_structure(s: &[usize], _label: &str) -> bool {
    if s.len() == 3 && s[1] == 5 {
        println!("  schema : [1,5,N] ✅ 与 yolo.rs 索引一致 (N={})", s[2]);
        true
    } else if s.len() == 3 && s[2] == 5 {
        eprintln!("  schema : [1,N,5] ❌ 转置变体 —— 需改 yolo.rs 索引（conf 在 [0,i,4]）");
        false
    } else {
        eprintln!("  schema : {:?} ❌ 非预期输出结构", s);
        false
    }
}

fn env_or(key: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(key).map(PathBuf::from).unwrap_or(default)
}
