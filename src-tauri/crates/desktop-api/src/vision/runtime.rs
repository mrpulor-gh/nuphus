//! Lazy ONNX Runtime initialization shared by the OCR and YOLO entrypoints.
//! macOS bundles store native libraries in Contents/Frameworks, not beside the executable.

#[cfg(any(target_os = "macos", test))]
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

/// Called immediately before creating a vision session, never during ordinary app startup.
/// Other platforms retain their existing ONNX Runtime search behavior.
pub fn ensure_onnx_runtime() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let executable = std::env::current_exe()
            .map_err(|error| format!("无法定位 ONNX Runtime 所在程序目录: {error}"))?;
        ensure_onnx_runtime_for_executable(&executable)?;
    }
    Ok(())
}

/// Resolve using a specific executable's layout; the bundle smoke test uses the packaged
/// executable here so it exercises the same search without modifying the signed .app.
#[cfg(target_os = "macos")]
pub fn ensure_onnx_runtime_for_executable(executable: &Path) -> Result<(), String> {
    let override_path = std::env::var_os("ORT_DYLIB_PATH");
    let path = macos_runtime_path(executable, override_path.as_deref(), Path::is_file)?;
    // init_from loads and checks the dylib ABI fallibly. Calling Session::builder directly
    // would take ort's default-loader path, which can panic on a missing/incompatible dylib.
    ort::init_from(&path)
        .map_err(|error| format!("无法加载 ONNX Runtime ({}): {error}", path.display()))?
        .commit();
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn macos_runtime_path(
    executable: &Path,
    override_path: Option<&OsStr>,
    is_file: impl Fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    let directory = executable.parent().ok_or("无法定位程序目录")?;
    if let Some(value) = override_path.filter(|value| !value.is_empty()) {
        let explicit = PathBuf::from(value);
        // Match ort's documented explicit-path behavior: relative to the executable first,
        // otherwise pass the caller's path to the loader. Never silently use a bundled
        // library when an explicit (possibly missing) override was requested.
        if !explicit.is_absolute() {
            let sibling = directory.join(&explicit);
            if is_file(&sibling) {
                return Ok(sibling);
            }
        }
        return Ok(explicit);
    }

    const LIBRARY: &str = "libonnxruntime.dylib";
    if directory.file_name() == Some(OsStr::new("MacOS")) {
        if let Some(contents) = directory
            .parent()
            .filter(|parent| parent.file_name() == Some(OsStr::new("Contents")))
        {
            let bundled = contents.join("Frameworks").join(LIBRARY);
            if is_file(&bundled) {
                return Ok(bundled);
            }
        }
    }
    let sibling = directory.join(LIBRARY);
    if is_file(&sibling) {
        return Ok(sibling);
    }
    Err(format!(
        "未找到 ONNX Runtime：请检查 {} 同目录或应用包 Contents/Frameworks 中的 {LIBRARY}，也可通过 ORT_DYLIB_PATH 指定路径",
        executable.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_bundle_prefers_frameworks_over_executable_sibling() {
        let executable = Path::new("Applications/Nuphus.app/Contents/MacOS/nuphus");
        let bundled = Path::new("Applications/Nuphus.app/Contents/Frameworks/libonnxruntime.dylib");
        let sibling = executable.parent().unwrap().join("libonnxruntime.dylib");
        assert_eq!(
            macos_runtime_path(executable, None, |path| path == bundled || path == sibling)
                .unwrap(),
            bundled
        );
    }

    #[test]
    fn dev_build_uses_library_next_to_executable() {
        let executable = Path::new("target/debug/nuphus");
        let library = Path::new("target/debug/libonnxruntime.dylib");
        assert_eq!(
            macos_runtime_path(executable, None, |path| path == library).unwrap(),
            library
        );
    }

    #[test]
    fn explicit_override_never_falls_back_to_bundle() {
        let executable = Path::new("Applications/Nuphus.app/Contents/MacOS/nuphus");
        let explicit = Path::new("custom/missing.dylib");
        let bundled = Path::new("Applications/Nuphus.app/Contents/Frameworks/libonnxruntime.dylib");
        assert_eq!(
            macos_runtime_path(executable, Some(explicit.as_os_str()), |path| path
                == bundled)
            .unwrap(),
            explicit
        );
    }

    #[test]
    fn relative_override_resolves_against_executable_before_loader_search() {
        let executable = Path::new("target/debug/nuphus");
        let library = Path::new("target/debug/custom/runtime.dylib");
        assert_eq!(
            macos_runtime_path(
                executable,
                Some(OsStr::new("custom/runtime.dylib")),
                |path| path == library
            )
            .unwrap(),
            library
        );
    }

    #[test]
    fn empty_override_uses_layout_and_missing_library_is_recoverable() {
        let executable = Path::new("target/debug/nuphus");
        let library = Path::new("target/debug/libonnxruntime.dylib");
        assert_eq!(
            macos_runtime_path(executable, Some(OsStr::new("")), |path| path == library).unwrap(),
            library
        );
        assert!(macos_runtime_path(executable, None, |_| false)
            .unwrap_err()
            .contains("ORT_DYLIB_PATH"));
    }
}
