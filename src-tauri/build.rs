fn main() {
    // 在 Windows release 模式下设置 /SUBSYSTEM:WINDOWS 避免控制台窗口弹出
    #[cfg(target_os = "windows")]
    if std::env::var("PROFILE")
        .map(|p| p == "release")
        .unwrap_or(false)
    {
        println!("cargo:rustc-link-arg=/SUBSYSTEM:WINDOWS");
    }

    tauri_build::build();

    // macOS：把 sherpa/onnxruntime dylib 的运行时查找路径写进二进制的 @rpath。
    // dev 时 dylib 被 sync_sherpa_libs 拷到 target/<profile>/（与二进制同目录），
    // 打包成 .app 后 dylib 放在 Contents/Frameworks/ 下；@executable_path 与
    // @executable_path/../Frameworks 两条 rpath 分别覆盖这两种布局，缺一不可。
    // 否则 @rpath/libsherpa-onnx-c-api.dylib 无法解析 → 启动即 "Library not loaded"。
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    }

    // ═══ 模型与运行时依赖自动获取 ═══
    // 产品原则：除 STT 语音模型外，其余所有本地模型/运行时都要自动获取——
    // 端用户不懂技术，git clone + cargo build 后必须自愈。所有步骤失败时都只
    // 打 warning 不中断构建（断网 build 仍成功，运行时 bootstrap.rs 会补下载）。
    // 顺序：先 OCR（最常缺）→ sherpa（其 shared 包自带 onnxruntime）→
    // onnxruntime（sherpa 未内置时兜底）→ YOLO → 最后 sync_*。
    download_ocr_models();
    ensure_sherpa_libs();
    #[cfg(target_os = "macos")]
    decouple_onnxruntime_name();
    ensure_onnxruntime_libs();
    ensure_yolo_model();
    sync_models_to_data_dir();
    sync_sherpa_libs();
}

// ─── sherpa-onnx (STT) 预编译库：自动下载 + 链接与运行时库同步 ─────
//
// 链接 sherpa-onnx v1.13.4 预编译 C API（assets 在 src-tauri/desktop/sherpa/）。
// 禁止引入需 cmake 构建 sherpa-onnx 源码的 crate。shared 包自带 onnxruntime
// 1.27，向后兼容 ORT C API，覆盖 desktop/ 的旧版 1.26（OCR 经 ort load-dynamic
// 按名加载同目录 onnxruntime，单进程只能有一份）。

// 平台规格表 ────────────────────────────────────────────────────────
struct SherpaSpec {
    url: &'static str,
    /// 解包后按文件名递归搜索、拷进 desktop/sherpa/ 的库。
    libs: &'static [&'static str],
}

#[cfg(target_os = "windows")]
fn sherpa_spec() -> Option<SherpaSpec> {
    Some(SherpaSpec {
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.4/sherpa-onnx-v1.13.4-win-x64-shared-MD-Release.tar.bz2",
        libs: &[
            "sherpa-onnx-c-api.dll",
            "sherpa-onnx-c-api.lib",
            "onnxruntime.dll",
            "onnxruntime_providers_shared.dll",
        ],
    })
}
#[cfg(target_os = "linux")]
fn sherpa_spec() -> Option<SherpaSpec> {
    Some(SherpaSpec {
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.4/sherpa-onnx-v1.13.4-linux-x64-shared.tar.bz2",
        libs: &[
            "libsherpa-onnx-c-api.so",
            "libonnxruntime.so",
            "libonnxruntime_providers_shared.so",
        ],
    })
}
#[cfg(target_os = "macos")]
fn sherpa_spec() -> Option<SherpaSpec> {
    Some(SherpaSpec {
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.4/sherpa-onnx-v1.13.4-osx-universal2-shared.tar.bz2",
        libs: &[
            "libsherpa-onnx-c-api.dylib",
            "libonnxruntime.dylib",
            "libonnxruntime_providers_shared.dylib",
        ],
    })
}
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn sherpa_spec() -> Option<SherpaSpec> {
    None
}

/// 平台链接库名（desktop/sherpa/ 下存在即视为已就绪）。
#[cfg(target_os = "windows")]
fn platform_sherpa_link_lib() -> &'static str {
    "sherpa-onnx-c-api.lib"
}
#[cfg(target_os = "linux")]
fn platform_sherpa_link_lib() -> &'static str {
    "libsherpa-onnx-c-api.so"
}
#[cfg(target_os = "macos")]
fn platform_sherpa_link_lib() -> &'static str {
    "libsherpa-onnx-c-api.dylib"
}
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn platform_sherpa_link_lib() -> &'static str {
    "unsupported-platform.link"
}

/// 运行时随 exe 分发的 sherpa 库（拷到 target/<profile>/ 与 deps/）。
#[cfg(target_os = "windows")]
fn platform_sherpa_runtime_libs() -> &'static [&'static str] {
    &[
        "sherpa-onnx-c-api.dll",
        "onnxruntime.dll",
        "onnxruntime_providers_shared.dll",
    ]
}
#[cfg(target_os = "linux")]
fn platform_sherpa_runtime_libs() -> &'static [&'static str] {
    &[
        "libsherpa-onnx-c-api.so",
        "libonnxruntime.so",
        "libonnxruntime_providers_shared.so",
    ]
}
#[cfg(target_os = "macos")]
fn platform_sherpa_runtime_libs() -> &'static [&'static str] {
    &[
        "libsherpa-onnx-c-api.dylib",
        "libonnxruntime.dylib",
        "libonnxruntime_providers_shared.dylib",
    ]
}
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn platform_sherpa_runtime_libs() -> &'static [&'static str] {
    &[]
}

fn tar_cmd() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "tar.exe" // Windows 10 1803+ 内置 bsdtar，支持 bz2
    }
    #[cfg(not(target_os = "windows"))]
    {
        "tar"
    }
}

/// 解包 archive 到 extract_dir。入口分发：
/// - Windows 上 .tar.bz2 → 两步（bzip2 -dc 解出纯 tar，再 tar -xf）。
///   实测直读 .tar.bz2 时 tar 内部解压子进程在某些环境会被杀
///   （"Child process exited with status 143"），解成纯 tar 后两种 tar 都稳。
/// - 其余（纯 tar / zip / tgz）→ 直接 tar -xf（tar 原生认 zip 与 gzip）。
///
/// 路径一律用相对路径避开 Windows 的两个坑：路径含冒号（C:/…）会被 tar 当
/// 远程主机规格去连接；含反斜杠会被当转义符损坏路径。把 cwd 设为共同父目录、
/// 只传 basename → 任意 tar（GNU/bsdtar）都无歧义，也无需 --force-local。
fn run_tar(archive: &std::path::Path, extract_dir: &std::path::Path) -> bool {
    #[cfg_attr(not(windows), allow(unused_variables))]
    let is_bz2 = archive
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("bz2"))
        .unwrap_or(false);
    #[cfg(target_os = "windows")]
    {
        if is_bz2 {
            return extract_bz2_windows(archive, extract_dir);
        }
    }
    extract_tar_direct(archive, extract_dir)
}

/// 直接 `tar -xf`（纯 tar / zip / tgz 通吃）。相对路径 + current_dir 见 run_tar。
fn extract_tar_direct(archive: &std::path::Path, extract_dir: &std::path::Path) -> bool {
    let Some(parent) = archive.parent() else {
        return false;
    };
    let Some(archive_name) = archive.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let Some(extract_name) = extract_dir.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let _ = std::fs::create_dir_all(extract_dir);
    let out = std::process::Command::new(tar_cmd())
        .current_dir(parent)
        .arg("-xf")
        .arg(archive_name)
        .arg("-C")
        .arg(extract_name)
        .output();
    match out {
        Ok(o) => {
            if !o.status.success() && !o.stderr.is_empty() {
                println!(
                    "cargo:warning=tar 解包 stderr: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            o.status.success()
        }
        Err(e) => {
            println!("cargo:warning=tar 启动失败: {e}");
            false
        }
    }
}

/// Windows 两步解包 .tar.bz2：bzip2 -dc 解成同目录 .tar（stdout 直写文件，
/// 不经过管道子进程），再走 extract_tar_direct。bzip2.exe 随 Git for Windows
/// 自带；缺失时返回 false，由调用方 warning。
#[cfg(target_os = "windows")]
fn extract_bz2_windows(archive: &std::path::Path, extract_dir: &std::path::Path) -> bool {
    let Some(stem) = archive.file_stem() else {
        return false;
    };
    let tar_path = archive.with_file_name(stem); // sherpa-x.tar.bz2 → sherpa-x.tar
    let file = match std::fs::File::create(&tar_path) {
        Ok(f) => f,
        Err(e) => {
            println!("cargo:warning=bzip2 创建临时 tar 失败: {e}");
            return false;
        }
    };
    let status = std::process::Command::new("bzip2.exe")
        .args(["-dc"])
        .arg(archive)
        .stdout(file)
        .stderr(std::process::Stdio::piped())
        .status();
    match status {
        Ok(s) if s.success() => extract_tar_direct(&tar_path, extract_dir),
        Ok(s) => {
            println!("cargo:warning=bzip2 解压退出码: {:?}", s.code());
            false
        }
        Err(e) => {
            println!("cargo:warning=bzip2 启动失败（需 Git for Windows 自带的 bzip2.exe）: {e}");
            false
        }
    }
}

/// 在 root 下递归查找名为 name 的文件并拷到 dest_dir。sherpa/onnxruntime 包
/// 布局随版本变化，用文件名搜索而非写死路径。
fn find_and_copy(root: &std::path::Path, name: &str, dest_dir: &std::path::Path) -> bool {
    fn walk(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
        let entries = std::fs::read_dir(dir).ok()?;
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(f) = walk(&p, name) {
                    return Some(f);
                }
            } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
                return Some(p);
            }
        }
        None
    }
    match walk(root, name) {
        Some(src) => std::fs::copy(&src, dest_dir.join(name)).is_ok(),
        None => false,
    }
}

/// 缺平台库时自动下载 sherpa shared 包并解包到 desktop/sherpa/。
fn ensure_sherpa_libs() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let sherpa_dir = std::path::PathBuf::from(&manifest_dir)
        .join("desktop")
        .join("sherpa");

    let Some(spec) = sherpa_spec() else {
        println!("cargo:warning=sherpa-onnx: 不支持的平台，跳过 STT 链接库自动下载");
        return;
    };
    let marker = sherpa_dir.join(platform_sherpa_link_lib());
    if marker.exists() {
        println!(
            "cargo:warning=sherpa-onnx: 平台库已就绪 ({})",
            platform_sherpa_link_lib()
        );
        return;
    }

    println!("cargo:warning=sherpa-onnx: ⬇  downloading prebuilt libs …");
    println!("cargo:warning=  {}", spec.url);
    let Some(out_dir) = std::env::var("OUT_DIR").ok() else {
        return;
    };
    let out_dir = std::path::PathBuf::from(&out_dir);
    let archive = out_dir.join("sherpa-onnx-shared.tar.bz2");
    let extract_dir = out_dir.join("sherpa-onnx-extract");

    if !download_file(spec.url, &archive) {
        println!("cargo:warning=sherpa-onnx: ✗ 下载失败，STT 不可用");
        println!(
            "cargo:warning=  手动下载平台 shared 包解包到 {}",
            sherpa_dir.display()
        );
        return;
    }
    let _ = std::fs::remove_dir_all(&extract_dir);
    let _ = std::fs::create_dir_all(&extract_dir);
    if !run_tar(&archive, &extract_dir) {
        println!(
            "cargo:warning=sherpa-onnx: ✗ 解包失败: {}",
            archive.display()
        );
        return;
    }
    let _ = std::fs::create_dir_all(&sherpa_dir);
    let mut copied = 0;
    for name in spec.libs {
        if find_and_copy(&extract_dir, name, &sherpa_dir) {
            copied += 1;
        } else {
            println!("cargo:warning=sherpa-onnx: 解包中未找到 {name}");
        }
    }
    println!(
        "cargo:warning=sherpa-onnx: ✓ 已拷贝 {copied} 个库到 {}",
        sherpa_dir.display()
    );
}

/// 三平台链接 + 运行时库同步：平台链接库存在即链接，并把运行时库拷到
/// target/<profile>/（exe 同目录）与 deps/（cargo test 二进制目录）。
fn sync_sherpa_libs() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let sherpa_dir = std::path::PathBuf::from(&manifest_dir)
        .join("desktop")
        .join("sherpa");

    let link_lib = platform_sherpa_link_lib();
    let lib = sherpa_dir.join(link_lib);
    if !lib.exists() {
        println!(
            "cargo:warning=sherpa-onnx: {} 不存在，跳过链接配置",
            lib.display()
        );
        println!("cargo:warning=  STT 功能不可用；build.rs 已尝试自动下载，请检查网络或手动从 v1.13.4 release 下载平台 shared 包");
        return;
    }
    println!("cargo:rustc-link-search=native={}", sherpa_dir.display());
    println!("cargo:rustc-link-lib=dylib=sherpa-onnx-c-api");

    let runtime_libs = platform_sherpa_runtime_libs();
    let out_dir = match std::env::var("OUT_DIR") {
        Ok(o) => o,
        Err(_) => return,
    };
    let target_profile = std::path::PathBuf::from(&out_dir)
        .parent() // build/<hash>
        .and_then(|p| p.parent()) // build/
        .and_then(|p| p.parent()) // <profile>/
        .map(|p| p.to_path_buf());
    let Some(profile_dir) = target_profile else {
        return;
    };

    for dest_dir in [profile_dir.clone(), profile_dir.join("deps")] {
        for name in runtime_libs {
            let src = sherpa_dir.join(name);
            let dest = dest_dir.join(name);
            if !src.exists() {
                continue;
            }
            // 尺寸不同即覆盖（onnxruntime 1.26 → 1.27 升级路径）
            let need_copy = match std::fs::metadata(&dest) {
                Ok(m) => m.len() != src.metadata().map(|s| s.len()).unwrap_or(0),
                Err(_) => true,
            };
            if need_copy {
                match std::fs::copy(&src, &dest) {
                    Ok(_) => println!("cargo:warning=sherpa-onnx: {name} → {}", dest_dir.display()),
                    Err(e) => println!("cargo:warning=sherpa-onnx: 拷贝 {name} 失败: {e}"),
                }
            }
        }
    }
}

// macOS：解除 sherpa c-api 对 onnxruntime 的「带版本号」依赖耦合。
// k2-fsa 的 osx-universal2 包里 c-api 引用的是 libonnxruntime.1.27.0.dylib，
// 但文件名是 libonnxruntime.dylib，且版本号会随 sherpa 升级变化。这里用
// install_name_tool 把 c-api 的依赖统一改成不带版本号的名字，使打包只关心
// libonnxruntime.dylib，无需在 tauri.conf.json / 代码里硬编码版本。
#[cfg(target_os = "macos")]
fn decouple_onnxruntime_name() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let sherpa_dir = std::path::PathBuf::from(&manifest_dir)
        .join("desktop")
        .join("sherpa");
    let c_api = sherpa_dir.join("libsherpa-onnx-c-api.dylib");
    if !c_api.exists() {
        return;
    }
    let Ok(out) = std::process::Command::new("otool")
        .args(["-L"])
        .arg(&c_api)
        .output()
    else {
        return;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let old = text.lines().map(str::trim).find_map(|l| {
        l.strip_prefix("@rpath/libonnxruntime.").map(|rest| {
            format!(
                "@rpath/libonnxruntime.{}",
                rest.split_whitespace().next().unwrap_or("")
            )
        })
    });
    let Some(old) = old else {
        println!("cargo:warning=sherpa-onnx: 未发现带版本号的 onnxruntime 依赖，跳过解耦");
        return;
    };
    let new = "@rpath/libonnxruntime.dylib";
    if old == new {
        return;
    }
    let _ = std::process::Command::new("install_name_tool")
        .args(["-change", &old, new])
        .arg(&c_api)
        .status();
    println!("cargo:warning=sherpa-onnx: 已解除 onnxruntime 版本耦合: {old} -> {new}");
}

// ─── OCR Model files ─────────────────────────────────────────────

struct ModelFile {
    url: &'static str,
    name: &'static str,
    hint: &'static str,
}

fn download_ocr_models() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let models_dir = std::path::PathBuf::from(&manifest_dir)
        .join("desktop")
        .join("models");

    std::fs::create_dir_all(&models_dir).ok();

    let files = &[
        ModelFile {
            url: "https://hf-mirror.com/SWHL/RapidOCR/resolve/main/PP-OCRv4/ch_PP-OCRv4_det_infer.onnx",
            name: "ch_PP-OCRv4_det.onnx",
            hint: "4.7 MB",
        },
        ModelFile {
            url: "https://hf-mirror.com/SWHL/RapidOCR/resolve/main/PP-OCRv4/ch_PP-OCRv4_rec_infer.onnx",
            name: "ch_PP-OCRv4_rec.onnx",
            hint: "10.8 MB",
        },
        ModelFile {
            url: "https://gitee.com/paddlepaddle/PaddleOCR/raw/main/ppocr/utils/ppocr_keys_v1.txt",
            name: "ch_PP-OCR_keys_v1.txt",
            hint: "26 KB",
        },
    ];

    let mut ok = 0u32;
    let mut fail = 0u32;

    for f in files {
        let path = models_dir.join(f.name);
        if file_ok(&path) {
            ok += 1;
            continue; // 已有有效文件，跳过
        }

        println!(
            "cargo:warning=PaddleOCR: ⬇  downloading {} ({}) …",
            f.name, f.hint
        );

        if download_file(f.url, &path) {
            ok += 1;
        } else {
            fail += 1;
            println!("cargo:warning=PaddleOCR: ✗  failed to download {}", f.name);
        }
    }

    let total = files.len() as u32;
    if fail == 0 {
        println!(
            "cargo:warning=PaddleOCR: ✓  {ok}/{total} model files ready at {}",
            models_dir.display()
        );
    } else {
        println!("cargo:warning=PaddleOCR: ⚠  {ok}/{total} downloaded, {fail} failed");
        println!("cargo:warning=   Manually download from:");
        println!("cargo:warning=     https://hf-mirror.com/SWHL/RapidOCR");
        println!("cargo:warning=     https://gitee.com/paddlepaddle/PaddleOCR");
        println!("cargo:warning=   Place files in: {}", models_dir.display());
    }
}

/// 文件已存在且 > 100 字节即认为有效
fn file_ok(p: &std::path::Path) -> bool {
    p.exists() && std::fs::metadata(p).map(|m| m.len() > 100).unwrap_or(false)
}

/// 跨平台下载：unix 用系统 curl；Windows 用 curl.exe → PowerShell 兜底。
/// （原实现只试 curl.exe/powershell.exe，mac/linux 上 OCR 下载从未生效。）
fn download_file(url: &str, dest: &std::path::Path) -> bool {
    download_file_platform(url, dest)
}

#[cfg(target_os = "windows")]
fn download_file_platform(url: &str, dest: &std::path::Path) -> bool {
    // 优先 curl.exe（Windows 10 1803+ 内置）；--max-time 防止不可达主机挂死构建
    let curl_ok = std::process::Command::new("curl.exe")
        .args(["-fSL", "--retry", "3", "--max-time", "120", "-o"])
        .arg(dest)
        .arg(url)
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if curl_ok {
        return true;
    }

    // 回退 PowerShell Invoke-WebRequest
    let ps_script = format!(
        "Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing -TimeoutSec 120",
        url,
        dest.display()
    );

    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &ps_script])
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn download_file_platform(url: &str, dest: &std::path::Path) -> bool {
    std::process::Command::new("curl")
        .args(["-fSL", "--retry", "3", "--max-time", "120", "-o"])
        .arg(dest)
        .arg(url)
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ─── ONNX Runtime 库自动获取 ─────────────────────────────────────

// 平台规格表
struct OrtSpec {
    /// 平台 onnxruntime 主库文件名（desktop/ 下）。
    lib: &'static str,
    /// 官方 release 下载 URL（sherpa 未内置时兜底）。
    url: &'static str,
}

#[cfg(target_os = "windows")]
fn ort_spec() -> Option<OrtSpec> {
    Some(OrtSpec {
        lib: "onnxruntime.dll",
        url: "https://www.nuget.org/api/v2/package/Microsoft.ML.OnnxRuntime/1.27.0",
    })
}
#[cfg(target_os = "linux")]
fn ort_spec() -> Option<OrtSpec> {
    Some(OrtSpec {
        lib: "libonnxruntime.so",
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.27.0/onnxruntime-linux-x64-1.27.0.tgz",
    })
}
#[cfg(target_os = "macos")]
fn ort_spec() -> Option<OrtSpec> {
    Some(OrtSpec {
        lib: "libonnxruntime.dylib",
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.27.0/onnxruntime-osx-arm64-1.27.0.tgz",
    })
}
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn ort_spec() -> Option<OrtSpec> {
    None
}

/// 确保桌面端 onnxruntime 运行时库在 desktop/ 下（gitignore，fresh clone 必缺）。
/// sherpa shared 包自带同版本 onnxruntime → 优先复用；否则从官方源下载解包。
fn ensure_onnxruntime_libs() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let desktop_dir = std::path::PathBuf::from(&manifest_dir).join("desktop");

    let Some(spec) = ort_spec() else {
        println!("cargo:warning=ONNX Runtime: 不支持的平台，跳过自动下载");
        return;
    };
    let target = desktop_dir.join(spec.lib);
    if target.exists() {
        println!("cargo:warning=ONNX Runtime: {} 已就绪", spec.lib);
        return;
    }

    // sherpa 包自带同版本 onnxruntime → 复用（先 ensure_sherpa_libs 已跑）
    let sherpa_ort = desktop_dir.join("sherpa").join(spec.lib);
    if sherpa_ort.exists() {
        match std::fs::copy(&sherpa_ort, &target) {
            Ok(_) => {
                println!(
                    "cargo:warning=ONNX Runtime: ✓ 从 sherpa 包复用 {}",
                    spec.lib
                );
                return;
            }
            Err(e) => println!("cargo:warning=ONNX Runtime: 从 sherpa 复用失败: {e}"),
        }
    }

    println!("cargo:warning=ONNX Runtime: ⬇  downloading {} …", spec.lib);
    println!("cargo:warning=  {}", spec.url);
    let Some(out_dir) = std::env::var("OUT_DIR").ok() else {
        return;
    };
    let out_dir = std::path::PathBuf::from(&out_dir);
    let archive = out_dir.join("onnxruntime-dl.bin");
    if !download_file(spec.url, &archive) {
        println!("cargo:warning=ONNX Runtime: ✗ 下载失败，OCR/STT 将不可用");
        return;
    }
    let extract_dir = out_dir.join("onnxruntime-extract");
    let _ = std::fs::remove_dir_all(&extract_dir);
    let _ = std::fs::create_dir_all(&extract_dir);
    // nupkg（zip）与 tgz 均可被 tar 处理
    if !run_tar(&archive, &extract_dir) {
        println!(
            "cargo:warning=ONNX Runtime: ✗ 解包失败: {}",
            archive.display()
        );
        return;
    }
    if find_and_copy(&extract_dir, spec.lib, &desktop_dir) {
        println!(
            "cargo:warning=ONNX Runtime: ✓ {}",
            desktop_dir.join(spec.lib).display()
        );
    } else {
        println!("cargo:warning=ONNX Runtime: ✗ 解包中未找到 {}", spec.lib);
    }
}

// ─── YOLO model 自动下载 ─────────────────────────────────────────

/// 确保 YOLO icon_detect.onnx 模型存在（缺失时自动下载，失败只 warning）。
/// 官方 OmniParser 仓只有 .pt；onnx-community 已导出 640×640 fp32 onnx (~12 MB)。
fn ensure_yolo_model() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let model_path = std::path::PathBuf::from(&manifest_dir)
        .join("desktop")
        .join("models")
        .join("icon_detect.onnx");

    if model_path.exists() && file_ok(&model_path) {
        println!("cargo:warning=YOLO model found: icon_detect.onnx");
        return;
    }

    let url = "https://hf-mirror.com/onnx-community/OmniParser-icon_detect_640x640/resolve/main/onnx/model.onnx";
    println!("cargo:warning=YOLO: ⬇  downloading icon_detect.onnx (~12 MB) …");
    println!("cargo:warning=  {url}");
    if download_file(url, &model_path) && file_ok(&model_path) {
        println!("cargo:warning=YOLO: ✓ icon_detect.onnx ready");
    } else {
        let _ = std::fs::remove_file(&model_path);
        println!("cargo:warning=YOLO: ✗ 下载失败，UI 元素检测将禁用（不影响 OCR）");
        println!("cargo:warning=  手动导出指引: 从 https://hf-mirror.com/microsoft/OmniParser-v2.0/resolve/main/icon_detect/model.pt 导出 ONNX 后放入 {}", model_path.display());
    }
}

/// 将 src-tauri/desktop/models/ 下的所有模型文件同步到 data_dir/Nuphus/models/
/// 保证 resolve_models_dir() 第2步（用户数据目录）可以直接命中，无需 fallback。
fn sync_models_to_data_dir() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let src = std::path::PathBuf::from(&manifest_dir)
        .join("desktop")
        .join("models");

    let data_dir = match std::env::var("APPDATA") {
        Ok(d) => std::path::PathBuf::from(d).join("Nuphus").join("models"),
        Err(_) => {
            println!("cargo:warning=无法获取 APPDATA，跳过模型同步");
            return;
        }
    };

    // 检查源目录
    if !src.exists() {
        println!(
            "cargo:warning=源模型目录不存在: {}，跳过同步",
            src.display()
        );
        return;
    }

    // 创建目标目录
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        println!(
            "cargo:warning=无法创建目标模型目录 {}: {e}",
            data_dir.display()
        );
        return;
    }

    // 遍历源目录，复制 .onnx 和 .txt 模型文件
    let entries = match std::fs::read_dir(&src) {
        Ok(e) => e,
        Err(e) => {
            println!("cargo:warning=无法读取源模型目录: {e}");
            return;
        }
    };

    let mut synced = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "onnx" && ext != "txt" {
            continue;
        }

        let fname = path.file_name().unwrap();
        let dest = data_dir.join(fname);

        // 目标已存在且更新 → 跳过
        let should_copy = match (path.metadata(), dest.metadata()) {
            (Ok(src_m), Ok(dst_m)) => src_m.modified().ok() != dst_m.modified().ok(),
            _ => true,
        };

        if should_copy {
            match std::fs::copy(&path, &dest) {
                Ok(bytes) => {
                    println!(
                        "cargo:warning=模型同步: {} → {} ({bytes} bytes)",
                        fname.to_string_lossy(),
                        dest.display()
                    );
                    synced += 1;
                }
                Err(e) => {
                    println!(
                        "cargo:warning=模型同步失败 {}: {e}",
                        fname.to_string_lossy()
                    );
                }
            }
        }
    }

    if synced > 0 {
        println!(
            "cargo:warning=模型同步完成: {synced} 个文件 → {}",
            data_dir.display()
        );
    } else {
        println!(
            "cargo:warning=模型已是最新，无需同步: {}",
            data_dir.display()
        );
    }
}
