//! 截图实现 - xcap + 自定义裁剪 + 图形后端分派

use crate::core::*;
use xcap::{Monitor, Window as XcapWindow};

/// 截图 - 根据目标和范围
pub async fn capture(target: &Target, scope: Scope) -> Result<Frame> {
    match scope {
        Scope::Fullscreen => capture_fullscreen().await,
        Scope::Window => capture_window(target).await,
        Scope::ClientArea => capture_client_area(target).await,
        Scope::Element { x, y, w, h } => capture_region(x, y, w, h).await,
        Scope::Point { x, y, radius } => {
            let size = radius * 2;
            capture_region(x - radius as i32, y - radius as i32, size, size).await
        }
    }
}

/// 全盘截图
async fn capture_fullscreen() -> Result<Frame> {
    let monitors = Monitor::all().map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
    let primary = monitors
        .into_iter()
        .next()
        .ok_or_else(|| DesktopError::CaptureFailed("no monitor found".to_string()))?;

    let image = primary
        .capture_image()
        .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
    convert_to_frame(image, Scope::Fullscreen, FrameSource::Screenshot)
}

/// 窗口截图 - 根据图形后端分派策略
async fn capture_window(target: &Target) -> Result<Frame> {
    #[cfg(windows)]
    {
        if let Target::Window {
            hwnd, gfx_backend, ..
        } = target
        {
            return capture_window_by_backend(*hwnd, *gfx_backend).await;
        }
    }

    // 非 Windows：Target::Window 变体不存在（cfg(windows)），直接回退全屏。
    // 跨平台窗口截图由 xcap 全屏 + 裁剪路径覆盖（capture_fullscreen_and_crop 仅 Windows）。

    // 回退: 全屏截图 (所有平台)
    capture_fullscreen().await
}

/// 按图形后端分派截图策略
async fn capture_window_by_backend(hwnd: isize, gfx: GfxBackend) -> Result<Frame> {
    match gfx {
        GfxBackend::Gdi => capture_window_gdi(hwnd).await,
        GfxBackend::DirectX | GfxBackend::Unknown => {
            // 先尝试 GDI，失败则降级到全屏+裁剪
            match capture_window_gdi(hwnd).await {
                Ok(frame) => Ok(frame),
                Err(_) => capture_fullscreen_and_crop(hwnd).await,
            }
        }
        GfxBackend::OpenGl | GfxBackend::Vulkan => {
            // OGL/Vulkan 窗口 GDI 截出黑屏，直接全屏+裁剪
            capture_fullscreen_and_crop(hwnd).await
        }
    }
}

/// GDI 窗口截图 (xcap)
async fn capture_window_gdi(hwnd: isize) -> Result<Frame> {
    let windows = XcapWindow::all().map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
    let win = windows
        .into_iter()
        .find(|w| w.id().ok().map(|id| id as isize) == Some(hwnd))
        .ok_or_else(|| DesktopError::CaptureFailed(format!("window {} not found", hwnd)))?;

    let image = win
        .capture_image()
        .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
    convert_to_frame(image, Scope::Window, FrameSource::WindowCapture)
}

/// 全屏截图 + 按窗口位置裁剪 (降级策略)
async fn capture_fullscreen_and_crop(hwnd: isize) -> Result<Frame> {
    let frame = capture_fullscreen().await?;

    #[cfg(windows)]
    {
        use ::windows::Win32::Foundation::{HWND, RECT};
        use ::windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

        let mut rect = RECT::default();
        let _ = unsafe { GetWindowRect(HWND(hwnd), &mut rect) };

        let x = rect.left.max(0) as u32;
        let y = rect.top.max(0) as u32;
        let w = (rect.right - rect.left) as u32;
        let h = (rect.bottom - rect.top) as u32;

        frame
            .crop(x, y, w, h)
            .ok_or_else(|| DesktopError::CaptureFailed("fullscreen crop failed".to_string()))
    }

    #[cfg(not(windows))]
    {
        // macOS/Linux: 使用 xcap 获取窗口位置进行裁剪（xcap 0.9: id/width/height 返回 Result）
        let windows = XcapWindow::all().map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
        let win = windows
            .into_iter()
            .find(|w| w.id().map(|id| id as isize).unwrap_or(-1) == hwnd)
            .ok_or_else(|| DesktopError::CaptureFailed(format!("window {} not found", hwnd)))?;

        let x = win.x().unwrap_or(0).max(0) as u32;
        let y = win.y().unwrap_or(0).max(0) as u32;
        let w = win.width().unwrap_or(0) as u32;
        let h = win.height().unwrap_or(0) as u32;

        frame
            .crop(x, y, w, h)
            .ok_or_else(|| DesktopError::CaptureFailed("fullscreen crop failed".to_string()))
    }
}

/// 客户区截图 (去掉标题栏边框)
async fn capture_client_area(target: &Target) -> Result<Frame> {
    #[cfg(windows)]
    {
        use ::windows::Win32::Foundation::{HWND, POINT, RECT};
        use ::windows::Win32::Graphics::Gdi::ClientToScreen;
        use ::windows::Win32::UI::WindowsAndMessaging::GetClientRect;

        if let Target::Window { hwnd, .. } = target {
            let hwnd = HWND(*hwnd);
            let mut client_rect = RECT::default();
            let mut point = POINT { x: 0, y: 0 };

            unsafe {
                let _ = GetClientRect(hwnd, &mut client_rect);
                let _ = ClientToScreen(hwnd, &mut point);
            }

            let x = point.x;
            let y = point.y;
            let w = (client_rect.right - client_rect.left) as u32;
            let h = (client_rect.bottom - client_rect.top) as u32;

            return capture_region(x, y, w, h).await;
        }
    }

    // 回退
    capture_window(target).await
}

/// 区域截图
async fn capture_region(x: i32, y: i32, w: u32, h: u32) -> Result<Frame> {
    let monitors = Monitor::all().map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
    let primary = monitors
        .into_iter()
        .next()
        .ok_or_else(|| DesktopError::CaptureFailed("no monitor".to_string()))?;

    let image = primary
        .capture_image()
        .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
    let frame = convert_to_frame(image, Scope::Fullscreen, FrameSource::Screenshot)?;

    let x = x.max(0) as u32;
    let y = y.max(0) as u32;
    // 越界坐标直接报错，避免 `frame.width - x` u32 下溢：debug 构建 panic 崩溃、
    // release 构建回绕成巨值。宁可失败也不产生错误截图。
    if x >= frame.width || y >= frame.height {
        return Err(DesktopError::CaptureFailed(format!(
            "capture region out of bounds: x={x}, y={y}, screen={}x{}",
            frame.width, frame.height
        )));
    }
    let w = w.min(frame.width - x);
    let h = h.min(frame.height - y);

    frame
        .crop(x, y, w, h)
        .ok_or_else(|| DesktopError::CaptureFailed("crop failed".to_string()))
}

/// 将 xcap 图像转换为 Frame
fn convert_to_frame(
    image: xcap::image::RgbaImage,
    scope: Scope,
    source: FrameSource,
) -> Result<Frame> {
    let width = image.width();
    let height = image.height();
    let pixels = image.into_raw();

    Ok(Frame {
        id: uuid::Uuid::new_v4(),
        pixels,
        width,
        height,
        scope,
        timestamp: chrono::Utc::now(),
        source,
    })
}
