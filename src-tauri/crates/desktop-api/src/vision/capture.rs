//! 截图实现 - xcap + 自定义裁剪 + 图形后端分派

use crate::core::*;
use xcap::Monitor;
use xcap::Window as XcapWindow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CaptureGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Capture and its desktop geometry are validated as one observation. Callers must not
/// query a new window origin after capture and attach it to an older image.
pub async fn capture_with_geometry(
    target: &Target,
    scope: Scope,
) -> Result<(Frame, CaptureGeometry)> {
    let before = scope_geometry(target, scope)?;
    let frame = capture(target, scope).await?;
    let after = scope_geometry(target, scope)?;
    if before != after {
        return Err(DesktopError::CaptureFailed(
            "窗口或显示器在截图期间发生变化，请重新截图".into(),
        ));
    }
    Ok((frame, before))
}

fn scope_geometry(target: &Target, scope: Scope) -> Result<CaptureGeometry> {
    let error = |e: xcap::XCapError| DesktopError::CaptureFailed(e.to_string());
    match scope {
        Scope::Element { x, y, w, h } => Ok(CaptureGeometry {
            x,
            y,
            width: w,
            height: h,
        }),
        Scope::Point { x, y, radius } => {
            let offset = i32::try_from(radius)
                .map_err(|_| DesktopError::CaptureFailed("invalid radius".into()))?;
            let size = radius
                .checked_mul(2)
                .ok_or_else(|| DesktopError::CaptureFailed("invalid radius".into()))?;
            Ok(CaptureGeometry {
                x: x.checked_sub(offset)
                    .ok_or_else(|| DesktopError::CaptureFailed("invalid x".into()))?,
                y: y.checked_sub(offset)
                    .ok_or_else(|| DesktopError::CaptureFailed("invalid y".into()))?,
                width: size,
                height: size,
            })
        }
        Scope::Fullscreen => {
            let monitor = Monitor::all()
                .map_err(error)?
                .into_iter()
                .find(|m| m.is_primary().unwrap_or(false))
                .ok_or_else(|| DesktopError::CaptureFailed("no primary monitor".into()))?;
            Ok(CaptureGeometry {
                x: monitor.x().map_err(error)?,
                y: monitor.y().map_err(error)?,
                width: monitor.width().map_err(error)?,
                height: monitor.height().map_err(error)?,
            })
        }
        Scope::Window | Scope::ClientArea => {
            let hwnd = match target {
                #[cfg(windows)]
                Target::Window { hwnd, .. } => *hwnd,
                Target::Tui { hwnd, .. } => *hwnd,
                _ => return Err(DesktopError::CaptureFailed("window target required".into())),
            };
            #[cfg(windows)]
            if matches!(scope, Scope::ClientArea) {
                use windows::Win32::{
                    Foundation::{HWND, POINT, RECT},
                    Graphics::Gdi::ClientToScreen,
                    UI::WindowsAndMessaging::GetClientRect,
                };
                let mut rect = RECT::default();
                let mut point = POINT::default();
                unsafe {
                    GetClientRect(HWND(hwnd), &mut rect)
                        .map_err(|e| DesktopError::CaptureFailed(e.to_string()))?;
                    if !ClientToScreen(HWND(hwnd), &mut point).as_bool() {
                        return Err(DesktopError::CaptureFailed("ClientToScreen failed".into()));
                    }
                }
                return Ok(CaptureGeometry {
                    x: point.x,
                    y: point.y,
                    width: (rect.right - rect.left) as u32,
                    height: (rect.bottom - rect.top) as u32,
                });
            }
            let window = xcap::Window::all()
                .map_err(error)?
                .into_iter()
                .find(|w| w.id().ok().map(|id| id as isize) == Some(hwnd))
                .ok_or_else(|| DesktopError::CaptureFailed(format!("window {hwnd} unavailable")))?;
            // xcap's Windows capture removes invisible DWM borders. Its own geometry,
            // not GetClientRect or GetWindowRect, describes the resulting window image.
            Ok(CaptureGeometry {
                x: window.x().map_err(error)?,
                y: window.y().map_err(error)?,
                width: window.width().map_err(error)?,
                height: window.height().map_err(error)?,
            })
        }
    }
}

/// 截图 - 根据目标和范围
pub async fn capture(target: &Target, scope: Scope) -> Result<Frame> {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "CoreGraphics", kind = "framework")]
        extern "C" {
            fn CGPreflightScreenCaptureAccess() -> bool;
        }
        // A denied capture may otherwise return an image with private windows
        // omitted, which is not a valid observation of the requested target.
        if !unsafe { CGPreflightScreenCaptureAccess() } {
            return Err(DesktopError::CaptureFailed(
                "截图需要录屏权限，请在系统设置→隐私与安全性→录屏中授权 Nuphus 后重试".into(),
            ));
        }
    }
    match scope {
        Scope::Fullscreen => capture_fullscreen().await,
        Scope::Window => capture_window(target).await,
        Scope::ClientArea => capture_client_area(target).await,
        Scope::Element { x, y, w, h } => capture_region(x, y, w, h).await,
        Scope::Point { x, y, radius } => {
            let geometry = scope_geometry(target, Scope::Point { x, y, radius })?;
            capture_region(geometry.x, geometry.y, geometry.width, geometry.height).await
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
    #[cfg(not(windows))]
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

    Err(DesktopError::CaptureFailed(
        "window target required; refusing fullscreen substitution".into(),
    ))
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
    let target = Target::Window {
        hwnd,
        title: String::new(),
        verified: false,
        gfx_backend: GfxBackend::Unknown,
    };
    let geometry = scope_geometry(&target, Scope::Window)?;
    capture_region(geometry.x, geometry.y, geometry.width, geometry.height).await
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
    capture_desktop_region(x, y, w, h)
}

/// Normalize physical capture pixels to the logical desktop grid used by AX and Enigo.
/// This keeps all existing OCR/YOLO/template offsets correct without asking models to scale.
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

fn capture_desktop_region(x: i32, y: i32, w: u32, h: u32) -> Result<Frame> {
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
