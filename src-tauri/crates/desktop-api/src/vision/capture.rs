//! 截图实现 - xcap + 自定义裁剪 + 图形后端分派

use crate::core::*;
use xcap::Monitor;
#[cfg(any(windows, target_os = "macos"))]
use xcap::Window as XcapWindow;

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
        .find(|monitor| monitor.is_primary().unwrap_or(false))
        .ok_or_else(|| DesktopError::CaptureFailed("no monitor found".to_string()))?;

    let image = primary
        .capture_image()
        .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
    #[cfg(target_os = "macos")]
    let image = logical_image(
        image,
        primary
            .width()
            .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?,
        primary
            .height()
            .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?,
    )?;
    convert_to_frame(image, Scope::Fullscreen, FrameSource::Screenshot)
}

/// 窗口截图 - 根据图形后端分派策略
#[cfg_attr(not(windows), allow(unused_variables))]
async fn capture_window(target: &Target) -> Result<Frame> {
    #[cfg(target_os = "macos")]
    if let Target::Tui { hwnd, .. } = target {
        let windows = XcapWindow::all().map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
        let window = windows
            .into_iter()
            .find(|window| window.id().ok().map(|id| id as isize) == Some(*hwnd))
            .ok_or_else(|| {
                DesktopError::CaptureFailed(format!(
                    "window {hwnd} unavailable; check Screen Recording permission"
                ))
            })?;
        let image = window
            .capture_image()
            .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
        let image = logical_image(
            image,
            window
                .width()
                .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?,
            window
                .height()
                .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?,
        )?;
        return convert_to_frame(image, Scope::Window, FrameSource::WindowCapture);
    }
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

/// 按图形后端分派截图策略（仅 Windows：Target::Window 变体与后端枚举是 Windows 概念）
#[cfg(windows)]
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

/// GDI 窗口截图 (xcap，仅 Windows)
#[cfg(windows)]
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

/// 全屏截图 + 按窗口位置裁剪 (降级策略，仅 Windows 调用链)
#[cfg(windows)]
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
        let w = win.width().unwrap_or(0);
        let h = win.height().unwrap_or(0);

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
    #[cfg(target_os = "macos")]
    {
        capture_macos_region(x, y, w, h)
    }
    #[cfg(not(target_os = "macos"))]
    {
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
}

/// Normalize physical capture pixels to the logical desktop grid used by AX and Enigo.
/// This keeps all existing OCR/YOLO/template offsets correct without asking models to scale.
#[cfg(any(target_os = "macos", test))]
fn logical_image(
    image: xcap::image::RgbaImage,
    width: u32,
    height: u32,
) -> Result<xcap::image::RgbaImage> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 64_000_000 {
        return Err(DesktopError::CaptureFailed(
            "invalid logical capture dimensions".into(),
        ));
    }
    if image.width() == width && image.height() == height {
        return Ok(image);
    }
    Ok(xcap::image::imageops::resize(
        &image,
        width,
        height,
        xcap::image::imageops::FilterType::Triangle,
    ))
}

#[cfg(any(target_os = "macos", test))]
fn intersection(
    region: (i32, i32, u32, u32),
    monitor: (i32, i32, u32, u32),
) -> Option<(u32, u32, u32, u32, u32, u32)> {
    let (x, y, w, h) = region;
    let (mx, my, mw, mh) = monitor;
    let left = i64::from(x).max(i64::from(mx));
    let top = i64::from(y).max(i64::from(my));
    let right = (i64::from(x) + i64::from(w)).min(i64::from(mx) + i64::from(mw));
    let bottom = (i64::from(y) + i64::from(h)).min(i64::from(my) + i64::from(mh));
    (right > left && bottom > top).then_some((
        (left - i64::from(mx)) as u32,
        (top - i64::from(my)) as u32,
        (right - left) as u32,
        (bottom - top) as u32,
        (left - i64::from(x)) as u32,
        (top - i64::from(y)) as u32,
    ))
}

#[cfg(target_os = "macos")]
fn capture_macos_region(x: i32, y: i32, w: u32, h: u32) -> Result<Frame> {
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 64_000_000 {
        return Err(DesktopError::CaptureFailed(
            "invalid capture region dimensions".into(),
        ));
    }
    let monitors = Monitor::all().map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
    let mut result = xcap::image::RgbaImage::new(w, h);
    let mut captured = false;
    for monitor in monitors {
        let geometry = (monitor.x(), monitor.y(), monitor.width(), monitor.height());
        let (Ok(mx), Ok(my), Ok(mw), Ok(mh)) = geometry else {
            continue;
        };
        let Some((sx, sy, cw, ch, dx, dy)) = intersection((x, y, w, h), (mx, my, mw, mh)) else {
            continue;
        };
        let image = monitor
            .capture_image()
            .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
        let image = logical_image(image, mw, mh)?;
        let crop = xcap::image::imageops::crop_imm(&image, sx, sy, cw, ch).to_image();
        xcap::image::imageops::overlay(&mut result, &crop, i64::from(dx), i64::from(dy));
        captured = true;
    }
    if !captured {
        return Err(DesktopError::CaptureFailed(
            "capture region outside connected displays".into(),
        ));
    }
    convert_to_frame(
        result,
        Scope::Element { x, y, w, h },
        FrameSource::Screenshot,
    )
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn retina_pixels_are_normalized_before_vision_coordinates() {
        let image =
            xcap::image::RgbaImage::from_pixel(200, 100, xcap::image::Rgba([10, 20, 30, 255]));
        let image = logical_image(image, 100, 50).unwrap();
        assert_eq!(image.dimensions(), (100, 50));
        assert_eq!(image.get_pixel(50, 25).0, [10, 20, 30, 255]);
        assert!(logical_image(image, 0, 50).is_err());
    }

    #[test]
    fn negative_origin_and_cross_display_regions_keep_offsets() {
        assert_eq!(
            intersection((-100, 20, 200, 100), (-1920, 0, 1920, 1080)),
            Some((1820, 20, 100, 100, 0, 0))
        );
        assert_eq!(
            intersection((-100, 20, 200, 100), (0, 0, 1920, 1080)),
            Some((0, 20, 100, 100, 100, 0))
        );
        assert_eq!(intersection((4000, 0, 50, 50), (0, 0, 1920, 1080)), None);
    }
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
