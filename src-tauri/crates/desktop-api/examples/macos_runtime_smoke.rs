//! Verify ONNX dylib resolution and ABI initialization against a packaged .app.
//!
//! On macOS:
//!   cargo build -p desktop-api --example macos_runtime_smoke
//!   target/debug/examples/macos_runtime_smoke /absolute/path/to/Nuphus.app
//!
//! This does not modify the signed bundle, load OCR models, request desktop permissions,
//! or rely on the developer's working directory. ORT_DYLIB_PATH remains an explicit override;
//! leave it unset when verifying the bundled library.

#[cfg(target_os = "macos")]
fn run() -> Result<(), String> {
    let app = std::env::args_os()
        .nth(1)
        .ok_or("usage: macos_runtime_smoke /path/to/Nuphus.app")?;
    let app =
        std::fs::canonicalize(app).map_err(|error| format!("invalid bundle path: {error}"))?;
    let executable = app.join("Contents/MacOS/nuphus");
    if !executable.is_file() {
        return Err(format!(
            "packaged executable missing: {}",
            executable.display()
        ));
    }
    desktop_api::vision::runtime::ensure_onnx_runtime_for_executable(&executable)?;
    let _builder = ort::session::Session::builder()
        .map_err(|error| format!("ONNX Runtime session initialization failed: {error}"))?;
    println!(
        "ONNX Runtime bundle resolution and session initialization passed: {}",
        app.display()
    );
    Ok(())
}

fn main() {
    #[cfg(target_os = "macos")]
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("macos_runtime_smoke requires macOS and a macOS application bundle");
        std::process::exit(2);
    }
}
