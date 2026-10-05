//! 遮罩视觉规格（**平台无关**）：颜色 / 尺寸 / 字模倍率 / 取色 HUD 参数。
//!
//! 三平台按**同一份数字**、用各自的原生 API 渲染，保证观感一致：
//!
//! - **Windows**（`windows.rs`）：直接写 top-down BGRA 像素缓冲（`UpdateLayeredWindow`）
//! - **macOS**（`macos.rs`）：`CGBitmapContext`（premultiplied BGRA8888）→ `CALayer.contents`
//! - **Linux**（`linux.rs`）：X11 ARGB32 visual + `XPutImage` / XRender
//!
//! # 颜色口径
//!
//! 一律 `0xAARRGGBB`（与 Windows DIB 的内存字节序 B,G,R,A 对应）。
//! 跨平台搬运时注意两点：① macOS 的 premultiplied 上下文需要预乘 alpha
//! （如 `0x33FF_FFFF` 按钮描边）；② Linux ARGB32 visual 的通道序与 DIB 相反，
//! 贴图前需做一次通道交换。
//!
//! 本模块是「视觉规格的单一来源」——改这里 = 改三平台观感；平台差异只允许出现在
//! 各自的**呈现手段**里，不允许出现各自的颜色/尺寸数字。
//!
//! 后续可继续下沉到本模块的纯逻辑（当前仍在 `windows.rs`，落地 macOS/Linux 时一并搬）：
//! 放大镜落点计算（越界翻转 + 夹紧）、色值文本格式化、点阵字模表。

// 平台无关依赖：几何（Rect/Point/节点布局）+ 交互状态机（App/Phase）。
// 本模块**不得**引入任何平台 API —— 三平台共用这里的同一份渲染实现。
use crate::commands::capture::geometry::{
    hit_node_at, node_is_corner, Point, Rect, Sel, NODE_CORNER, NODE_COUNT, NODE_EDGE,
};
use crate::commands::capture::state::{App, Phase};

// ---- 尺寸与透明度 ----

/// 洞外压暗 alpha（70%）。
pub(crate) const MASK_ALPHA: u32 = 0xB4;
/// 主边框宽（亮蓝，1px）。
pub(crate) const BORDER_W: i32 = 1;
/// 按钮圆角半径（对齐宿主 UI 的 6px 圆角）。
pub(crate) const BTN_RADIUS: i32 = 6;

// ---- 颜色（0xAARRGGBB）----

/// 洞外压暗：黑 70%。
pub(crate) const COLOR_MASK: u32 = MASK_ALPHA << 24;
/// 洞内：alpha=0 真透明（透出下层冻结帧）。
pub(crate) const COLOR_HOLE: u32 = 0x0000_0000;
/// 圆角镂空：同为 alpha=0，语义与 `COLOR_HOLE` 不同。
pub(crate) const COLOR_TRANSPARENT: u32 = 0x0000_0000;
/// 主边框亮蓝。
pub(crate) const COLOR_BORDER: u32 = 0xFF3B_82F6;
/// 节点白底。
pub(crate) const COLOR_NODE: u32 = 0xFFFF_FFFF;
/// 角节点蓝底。
pub(crate) const COLOR_NODE_CORNER: u32 = 0xFF3B_82F6;
/// 边节点蓝心。
pub(crate) const COLOR_NODE_INNER: u32 = 0xFF3B_82F6;
/// 角节点白心。
pub(crate) const COLOR_NODE_INNER_W: u32 = 0xFFFF_FFFF;
/// 节点 hover 高亮。
pub(crate) const COLOR_NODE_HOT: u32 = 0xFF93_C5FD;
/// 确认按钮：主题蓝。
pub(crate) const COLOR_BTN_OK: u32 = 0xFF3B_82F6;
/// 确认按钮 hover。
pub(crate) const COLOR_BTN_OK_HOT: u32 = 0xFF25_63EB;
/// 取消按钮：深灰。
pub(crate) const COLOR_BTN_CANCEL_FILL: u32 = 0xFF1F_2937;
/// 取消按钮 hover。
pub(crate) const COLOR_BTN_CANCEL_HOT: u32 = 0xFF37_4148;
/// 按钮文字（OK/X 字模）。
pub(crate) const COLOR_BTN_EDGE: u32 = 0xFFFF_FFFF;
/// 按钮描边：20% 白（替代原先刺眼的纯白 1px 硬边）。
pub(crate) const COLOR_BTN_EDGE_SOFT: u32 = 0x33FF_FFFF;

/// 确认按钮文案（平台层用系统字体渲染，见 `draw_hud` 的说明）。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const BTN_OK_LABEL: &str = "确定";
/// 取消按钮文案。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const BTN_X_LABEL: &str = "取消";

// ---- 取色 HUD（放大镜 + 实时色值条）----

/// 放大镜采样源边长（奇数 → 中心像素恰好在正中）。
pub(crate) const MAG_SRC: i32 = 41;
/// 放大镜倍率。
pub(crate) const MAG_ZOOM: i32 = 4;
/// 放大镜边长（= 源边长 × 倍率）。
pub(crate) const MAG_DST: i32 = MAG_SRC * MAG_ZOOM;
/// 放大镜相对鼠标的偏移（默认右下，越界翻到左上）。
pub(crate) const MAG_OFFSET: i32 = 24;
/// 色值文本点阵倍率。
pub(crate) const HUD_SCALE: i32 = 2;
/// 色值文本字距（字宽 5 + 间隔 1，均按 `HUD_SCALE` 放大）。
pub(crate) const HUD_ADVANCE: i32 = 5 * HUD_SCALE + HUD_SCALE;
/// 色值条底色：黑 90%。
pub(crate) const COLOR_HUD_BG: u32 = 0xE600_0000;
/// HUD 文字 / 放大镜亮边框。
pub(crate) const COLOR_HUD_TEXT: u32 = 0xFFFF_FFFF;
/// 放大镜外沿阴影。
pub(crate) const COLOR_HUD_EDGE: u32 = 0x9900_0000;

// ═══════════════════════════ §4 像素绘制工具 ═══════════════════════════
//
// 全部为纯像素写入：只改 `buf`，不碰 GDI / DC 状态。
// 统一遵守 top-down：y=0 是屏幕最上一行。

/// 填充矩形（自带屏幕内 clamp，越界自动切掉）。
fn fill_rect(buf: &mut [u32], bw: i32, bh: i32, r: Rect, color: u32) {
    let l = r.left.max(0);
    let t = r.top.max(0);
    let ri = r.right.min(bw);
    let bt = r.bottom.min(bh);
    if ri <= l || bt <= t {
        return;
    }
    for y in t..bt {
        let row = (y as usize) * (bw as usize);
        for x in l..ri {
            buf[row + x as usize] = color;
        }
    }
}

/// 描边（四条 1px 边；调用方保证矩形四边顺序正确）。
fn stroke_rect(buf: &mut [u32], bw: i32, bh: i32, r: Rect, color: u32) {
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.top + BORDER_W,
        },
        color,
    );
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: r.left,
            top: r.bottom - BORDER_W,
            right: r.right,
            bottom: r.bottom,
        },
        color,
    );
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: r.left,
            top: r.top,
            right: r.left + BORDER_W,
            bottom: r.bottom,
        },
        color,
    );
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: r.right - BORDER_W,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        },
        color,
    );
}

/// 圆角矩形：四角按距离场做**抗锯齿**圆角（旧实现是阶梯状切块，又糙又廉价）。
///
/// 覆盖率为 0 的角块写 `COLOR_TRANSPARENT`（alpha=0 镂空）而**不是** `COLOR_MASK` ——
/// 压暗色是不透明的，会把圆角重新填成方块。
fn fill_round_rect(buf: &mut [u32], bw: i32, bh: i32, r: Rect, color: u32) {
    let rad = BTN_RADIUS
        .min((r.right - r.left) / 2)
        .min((r.bottom - r.top) / 2)
        .max(1);
    // 中部十字区直接填满（四角留给下面的圆角处理）
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: r.left + rad,
            top: r.top,
            right: r.right - rad,
            bottom: r.bottom,
        },
        color,
    );
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: r.left,
            top: r.top + rad,
            right: r.right,
            bottom: r.bottom - rad,
        },
        color,
    );
    // 四角：按像素中心到圆心的距离算覆盖率，写带 alpha 的像素
    // （层窗口走 ULW_ALPHA，真混合 → 边缘平滑，不再是阶梯状切块）
    let full_a = (color >> 24) & 0xFF;
    let rgb = color & 0x00FF_FFFF;
    let corners = [
        (r.left + rad, r.top + rad, r.left, r.top),
        (r.right - rad, r.top + rad, r.right - rad, r.top),
        (r.left + rad, r.bottom - rad, r.left, r.bottom - rad),
        (r.right - rad, r.bottom - rad, r.right - rad, r.bottom - rad),
    ];
    for &(ccx, ccy, x0, y0) in corners.iter() {
        for y in y0..y0 + rad {
            for x in x0..x0 + rad {
                if x < 0 || y < 0 || x >= bw || y >= bh {
                    continue;
                }
                let px = x as f32 + 0.5 - ccx as f32;
                let py = y as f32 + 0.5 - ccy as f32;
                let d = (px * px + py * py).sqrt();
                let cov = (rad as f32 + 0.5 - d).clamp(0.0, 1.0);
                let a = (full_a as f32 * cov).round() as u32;
                // 覆盖率 0 也要写：把上一帧/内芯填进来的像素重新镂空（alpha=0），
                // 否则圆角会被十字填充重新填成方块。
                let idx = (y as usize) * (bw as usize) + x as usize;
                buf[idx] = if a == 0 {
                    COLOR_TRANSPARENT
                } else {
                    rgb | (a << 24)
                };
            }
        }
    }
}

/// 画一个缩放手柄：角节点更大、蓝底白心；边节点更小、白底蓝心；hover 外扩高亮。
fn draw_node(buf: &mut [u32], bw: i32, bh: i32, p: Point, corner: bool, hot: bool) {
    let size = if corner { NODE_CORNER } else { NODE_EDGE };
    let h = size / 2;
    let outer = if hot { 2 } else { 1 };
    // 外圈：白描边（保证任何背景上都清晰），hover 时换成高亮蓝
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: p.x - h - outer,
            top: p.y - h - outer,
            right: p.x + h + outer,
            bottom: p.y + h + outer,
        },
        if hot { COLOR_NODE_HOT } else { COLOR_NODE },
    );
    // 主体
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: p.x - h,
            top: p.y - h,
            right: p.x + h,
            bottom: p.y + h,
        },
        if corner {
            COLOR_NODE_CORNER
        } else {
            COLOR_NODE
        },
    );
    // 内芯：角节点白心、边节点蓝心（形成层次，不糊成一团）
    let inner_pad = if corner { 3 } else { 2 };
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: p.x - h + inner_pad,
            top: p.y - h + inner_pad,
            right: p.x + h - inner_pad,
            bottom: p.y + h - inner_pad,
        },
        if corner {
            COLOR_NODE_INNER_W
        } else {
            COLOR_NODE_INNER
        },
    );
}

// ── 取色 HUD（放大镜 + 实时色值）──
//
// 对齐迁移前 WebView 链路的取色观感：跟随鼠标的放大镜（局部放大 + 中心像素
// 高亮框 + 十字准星）与色值条（色块 + `#RRGGBB` + R/G/B 十进制）。
// 全部是纯像素写入（与 §4 同一套工具），不碰 GDI / DC 状态 —— 尤其不能用
// TextOutW：DIB 的 alpha 通道 GDI 不负责写，文本会以 alpha=0 呈现成「隐形」。

// 放大镜 / 色值条参数（MAG_* / HUD_* / COLOR_HUD_*）同属视觉规格，
// 已上提到平台无关的 `surface::paint`（见文件顶部的 `use super::paint::*`）。

/// 取色 HUD 用的 5x7 点阵（**行优先**，每行低 5 位，bit4 = 最左列）。
///
/// 与按钮用的 `glyph()`（列优先、7 列）分开实现：既不动既有 OK/X 的观感，
/// 也让字形数据能照着标准字模直接抄。只需数字 / 十六进制字母 / R G B / #。
fn hud_glyph(ch: char) -> Option<[u8; 7]> {
    Some(match ch {
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b11110, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        '#' => [
            0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
        ],
        _ => return None,
    })
}

/// 画一行 HUD 点阵文本（空格与未收录字符按 1 个字位跳过），返回占用宽度。
fn draw_hud_text(buf: &mut [u32], bw: i32, bh: i32, x0: i32, y0: i32, s: &str, color: u32) -> i32 {
    let mut x = x0;
    for ch in s.chars() {
        if let Some(rows) = hud_glyph(ch) {
            for (r, bits) in rows.iter().enumerate() {
                for c in 0..5 {
                    if (bits >> (4 - c)) & 1 == 1 {
                        let px = x + c * HUD_SCALE;
                        let py = y0 + (r as i32) * HUD_SCALE;
                        fill_rect(
                            buf,
                            bw,
                            bh,
                            Rect {
                                left: px,
                                top: py,
                                right: px + HUD_SCALE,
                                bottom: py + HUD_SCALE,
                            },
                            color,
                        );
                    }
                }
            }
        }
        x += HUD_ADVANCE;
    }
    x - x0
}

/// 取色 HUD：放大镜（冻结帧局部放大 + 中心像素框）+ 鼠标十字 + 色值条。
///
/// 取色真值 = 鼠标所在像素（屏幕物理像素，与 toolbar 侧 `color_picker` 分支同口径）；
/// HUD 只是视觉定位，不参与真值计算。
fn draw_color_picker_hud(app: &App, frozen: &[u32], buf: &mut [u32], bw: i32, bh: i32) {
    let (mx, my) = (app.last_mouse.x, app.last_mouse.y);
    if mx < 0 || my < 0 || mx >= bw || my >= bh {
        return; // 首帧鼠标尚未进屏 → 只显示原画面
    }
    if frozen.len() < (bw as usize) * (bh as usize) {
        return; // 冻结帧不可用（防御：平台层未提供采样源）
    }

    let pix = frozen[(my as usize) * (bw as usize) + mx as usize];
    let (rr, gg, bb) = ((pix >> 16) & 0xFF, (pix >> 8) & 0xFF, pix & 0xFF);

    // 放大镜落点：默认鼠标右下；越界翻到左上；最后夹进屏内。
    let mut lx = mx + MAG_OFFSET;
    let mut ly = my + MAG_OFFSET;
    if lx + MAG_DST > bw {
        lx = mx - MAG_OFFSET - MAG_DST;
    }
    if ly + MAG_DST > bh {
        ly = my - MAG_OFFSET - MAG_DST;
    }
    lx = lx.clamp(0, (bw - MAG_DST).max(0));
    ly = ly.clamp(0, (bh - MAG_DST).max(0));

    // 放大镜内容：以鼠标为中心 MAG_SRC×MAG_SRC → MAG_DST×MAG_DST（最近邻放大）。
    let half = MAG_SRC / 2;
    for dy in 0..MAG_DST {
        let sy = (my - half + dy / MAG_ZOOM).clamp(0, bh - 1);
        let src_row = (sy as usize) * (bw as usize);
        let dst_row = ((ly + dy) as usize) * (bw as usize);
        for dx in 0..MAG_DST {
            let sx = (mx - half + dx / MAG_ZOOM).clamp(0, bw - 1);
            // 强制不透明：抓屏 alpha 可能是 0，原样写入会让放大镜整块透明。
            buf[dst_row + (lx + dx) as usize] =
                0xFF00_0000 | (frozen[src_row + sx as usize] & 0x00FF_FFFF);
        }
    }
    // 外沿：深色阴影 + 白框（在压暗底和冻结帧上都清晰）
    stroke_rect(
        buf,
        bw,
        bh,
        Rect {
            left: lx - 1,
            top: ly - 1,
            right: lx + MAG_DST + 1,
            bottom: ly + MAG_DST + 1,
        },
        COLOR_HUD_EDGE,
    );
    stroke_rect(
        buf,
        bw,
        bh,
        Rect {
            left: lx - 2,
            top: ly - 2,
            right: lx + MAG_DST + 2,
            bottom: ly + MAG_DST + 2,
        },
        COLOR_HUD_TEXT,
    );
    // 中心像素高亮框（放大镜正中 = 鼠标那一个像素）
    let c0 = lx + half * MAG_ZOOM;
    let c1 = ly + half * MAG_ZOOM;
    stroke_rect(
        buf,
        bw,
        bh,
        Rect {
            left: c0 - 2,
            top: c1 - 2,
            right: c0 + MAG_ZOOM + 2,
            bottom: c1 + MAG_ZOOM + 2,
        },
        COLOR_HUD_TEXT,
    );
    stroke_rect(
        buf,
        bw,
        bh,
        Rect {
            left: c0 - 1,
            top: c1 - 1,
            right: c0 + MAG_ZOOM + 1,
            bottom: c1 + MAG_ZOOM + 1,
        },
        0xFF00_0000,
    );

    // 鼠标点十字准星：中间留空 3px，不盖住被取色的那个像素。
    for i in 2..=8 {
        for (x, y) in [(mx - i, my), (mx + i, my), (mx, my - i), (mx, my + i)] {
            if x >= 0 && x < bw && y >= 0 && y < bh {
                buf[(y as usize) * (bw as usize) + x as usize] = 0xFFFF_FFFF;
            }
        }
    }

    // 色值条：色块 + `#RRGGBB R59 G130 B246`
    let text = format!("#{rr:02X}{gg:02X}{bb:02X} R{rr} G{gg} B{bb}");
    let sw = 16; // 色块边长
    let pad = 6;
    let tw = text.chars().count() as i32 * HUD_ADVANCE;
    let bar_w = pad * 3 + sw + tw;
    let bar_h = 24;
    // 默认贴在放大镜下方；放不下就翻到上方；最后夹进屏内。
    let mut bx = lx;
    let mut by = ly + MAG_DST + 6;
    if by + bar_h > bh {
        by = ly - 6 - bar_h;
    }
    bx = bx.clamp(0, (bw - bar_w).max(0));
    by = by.clamp(0, (bh - bar_h).max(0));
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: bx,
            top: by,
            right: bx + bar_w,
            bottom: by + bar_h,
        },
        COLOR_HUD_BG,
    );
    stroke_rect(
        buf,
        bw,
        bh,
        Rect {
            left: bx,
            top: by,
            right: bx + bar_w,
            bottom: by + bar_h,
        },
        0x33FF_FFFF,
    );
    let sy0 = by + (bar_h - sw) / 2;
    fill_rect(
        buf,
        bw,
        bh,
        Rect {
            left: bx + pad,
            top: sy0,
            right: bx + pad + sw,
            bottom: sy0 + sw,
        },
        0xFF00_0000 | (rr << 16) | (gg << 8) | bb,
    );
    stroke_rect(
        buf,
        bw,
        bh,
        Rect {
            left: bx + pad,
            top: sy0,
            right: bx + pad + sw,
            bottom: sy0 + sw,
        },
        0x8CFF_FFFF,
    );
    draw_hud_text(
        buf,
        bw,
        bh,
        bx + pad + sw + pad,
        by + (bar_h - 7 * HUD_SCALE) / 2,
        &text,
        COLOR_HUD_TEXT,
    );
}

// ═══════════════════════ 三平台共用的像素渲染（唯一实现） ═══════════════════════

/// 渲染一帧遮罩像素：压暗底 → 洞（alpha=0）→ 洞外描边环 → HUD → 取色 HUD。
///
/// 平台层只做两件事：① 提供 `buf`（top-down BGRA 缓冲）与 `frozen`（冻结帧采样源）；
/// ② 渲染完把 `buf` 贴到屏幕（Windows `UpdateLayeredWindow` / macOS `CALayer.contents`
/// / Linux `XPutImage`）。**渲染规则全部在这里** —— 改这里 = 改三平台观感。
///
/// `app` 需要 `&mut`：③ 会在绘制前重算 HUD 布局（`recompute_hud`）。
/// 渲染一帧遮罩像素（**Alpha 模式**：Windows 分层窗口）。
///
/// 前提：`buf` 是 `UpdateLayeredWindow` 的逐像素 alpha 缓冲，窗口下方还有一层
/// 「冻结帧静态层」，因此洞内**必须保持 alpha=0**、由下沉层透出冻结帧。
///
/// macOS / Linux 没有等价的分层 alpha 呈现，用 [`render_opaque`]。
pub(crate) fn render(
    buf: &mut [u32],
    bw: i32,
    bh: i32,
    app: &mut App,
    point_pick: bool,
    pick_color: bool,
    frozen: &[u32],
) {
    // ① 洞外压暗底
    for px in buf.iter_mut() {
        *px = COLOR_MASK;
    }

    // ② 洞内真透明 + 洞外描边环。
    //
    // ⚠️ 铁律 H2：洞内一个字节都不许写。洞内必须保持 alpha=0 的纯净透明，
    // 否则半透明色会叠在冻结帧上让选区「变色」。
    // 绘制一律以**屏内可见部分**（vis）为基准，选区越界时沿屏幕边缘自然裁掉。
    let vis = app.sel.clamped(bw, bh);
    if let Some(s) = vis {
        fill_rect(buf, bw, bh, rect_of(&s), COLOR_HOLE);
        if !point_pick {
            draw_selection_frame(buf, bw, bh, &s);
        }
    }

    draw_hud(buf, bw, bh, app, point_pick, pick_color, frozen);
}

/// 渲染一帧遮罩像素（**Opaque 模式**：macOS / Linux 的原生窗口）。
///
/// 这些平台没有「逐像素 alpha 分层窗口」这种呈现手段（或代价过高），改为：
/// **冻结帧就是窗口背景**，遮罩/洞/描边/HUD 全部由本函数画在它之上。
///
/// 与原 Alpha 版的等价性：
/// - Windows：整屏压暗 + 洞内 alpha=0（透出下层静态层）；
/// - Opaque：背景先铺冻结帧 → 洞外按 `MASK_ALPHA` **逐像素混合压暗** → 洞内保持原像素。
///   而描边/HUD/节点/按钮本来就画在洞外或其上，两者观感一致。
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
pub(crate) fn render_opaque(
    buf: &mut [u32],
    bw: i32,
    bh: i32,
    app: &mut App,
    point_pick: bool,
    pick_color: bool,
    frozen: &[u32],
) {
    let n = (bw as usize) * (bh as usize);

    // ① 背景 = 冻结帧（不可用时退化为纯黑，避免花屏）
    if frozen.len() >= n && buf.len() >= n {
        buf[..n].copy_from_slice(&frozen[..n]);
        for px in buf[..n].iter_mut() {
            *px |= 0xFF00_0000; // 不透明化：本平台不做逐像素 alpha 合成
        }
    } else {
        for px in buf.iter_mut() {
            *px = 0xFF00_0000;
        }
    }

    // ② 洞外压暗（四块，避开洞）
    let vis = app.sel.clamped(bw, bh);
    match vis {
        Some(s) => {
            let (l, t, r, b) = (s.x, s.y, s.right(), s.bottom());
            fill_rect_blend(
                buf,
                bw,
                bh,
                Rect {
                    left: 0,
                    top: 0,
                    right: bw,
                    bottom: t,
                },
                COLOR_MASK,
            );
            fill_rect_blend(
                buf,
                bw,
                bh,
                Rect {
                    left: 0,
                    top: b,
                    right: bw,
                    bottom: bh,
                },
                COLOR_MASK,
            );
            fill_rect_blend(
                buf,
                bw,
                bh,
                Rect {
                    left: 0,
                    top: t,
                    right: l,
                    bottom: b,
                },
                COLOR_MASK,
            );
            fill_rect_blend(
                buf,
                bw,
                bh,
                Rect {
                    left: r,
                    top: t,
                    right: bw,
                    bottom: b,
                },
                COLOR_MASK,
            );
            if !point_pick {
                draw_selection_frame(buf, bw, bh, &s);
            }
        }
        None => {
            fill_rect_blend(
                buf,
                bw,
                bh,
                Rect {
                    left: 0,
                    top: 0,
                    right: bw,
                    bottom: bh,
                },
                COLOR_MASK,
            );
        }
    }

    draw_hud(buf, bw, bh, app, point_pick, pick_color, frozen);
}

/// 选区外发光 + 亮蓝主边框（两种模式共用）。
fn draw_selection_frame(buf: &mut [u32], bw: i32, bh: i32, s: &Sel) {
    // a) 外发光：3 层 1px 描边环，由浓到淡向外扩散（只画洞外）
    const GLOW: [(i32, u32); 3] = [
        (1, (0x66u32 << 24) | 0x3B_82F6),
        (2, (0x44u32 << 24) | 0x3B_82F6),
        (3, (0x26u32 << 24) | 0x3B_82F6),
    ];
    for (off, color) in GLOW {
        stroke_rect(
            buf,
            bw,
            bh,
            Rect {
                left: s.x - off,
                top: s.y - off,
                right: s.right() + off,
                bottom: s.bottom() + off,
            },
            color,
        );
    }
    // b) 主边框：亮蓝 1px（最外沿，保证清晰）
    stroke_rect(
        buf,
        bw,
        bh,
        Rect {
            left: s.x - 1,
            top: s.y - 1,
            right: s.right() + 1,
            bottom: s.bottom() + 1,
        },
        COLOR_BORDER,
    );
}

/// 确认/调整态 HUD（8 节点 + OK/X）与取色 HUD —— 两种模式共用。
fn draw_hud(
    buf: &mut [u32],
    bw: i32,
    bh: i32,
    app: &mut App,
    point_pick: bool,
    pick_color: bool,
    frozen: &[u32],
) {
    // ③ 确认/调整态 HUD：8 节点 + OK/X（节点中心已外移到洞外压暗区）
    if !point_pick && matches!(app.phase, Phase::Confirm | Phase::Moving | Phase::Adjusting) {
        // HUD 基于可见区排版；可见区为空（选区完全在屏外）则不画。
        app.recompute_hud(bw, bh);
        let hover = hit_node_at(&app.node_pts, app.last_mouse.x, app.last_mouse.y);
        for i in 0..NODE_COUNT {
            draw_node(
                buf,
                bw,
                bh,
                app.node_pts[i],
                node_is_corner(i),
                Some(i) == hover,
            );
        }
        // 确认：主题蓝实心圆角；取消：深灰实心圆角；hover 各自提亮
        let ok_fill = if app.btn_ok_hot {
            COLOR_BTN_OK_HOT
        } else {
            COLOR_BTN_OK
        };
        let x_fill = if app.btn_x_hot {
            COLOR_BTN_CANCEL_HOT
        } else {
            COLOR_BTN_CANCEL_FILL
        };
        // 描边贴合圆角：整块先画描边色，再用内缩 1px 的填充色覆盖。
        let inset = |r: Rect| Rect {
            left: r.left + 1,
            top: r.top + 1,
            right: r.right - 1,
            bottom: r.bottom - 1,
        };
        fill_round_rect(buf, bw, bh, app.btn_ok, COLOR_BTN_EDGE_SOFT);
        fill_round_rect(buf, bw, bh, inset(app.btn_ok), ok_fill);
        fill_round_rect(buf, bw, bh, app.btn_x, COLOR_BTN_EDGE_SOFT);
        fill_round_rect(buf, bw, bh, inset(app.btn_x), x_fill);
        // 按钮文字**不在这里画**：中文（确定 / 取消）需要系统字体，自绘点阵画不出；
        // 由平台层在本函数之后叠加（Windows: `windows.rs::draw_btn_text` 用 GDI；
        // macOS / Linux 待接入各自的文本 API —— 未接入时按钮只有底色，点击判定不受影响）。
        // 文案见 `BTN_OK_LABEL` / `BTN_X_LABEL`。
    }

    // ③.5 取色 HUD：跟着鼠标的放大镜 + 实时色值条（仅取色模式）。
    //      HUD 像素不参与取色真值（真值由 session/toolbar 侧从冻结帧数据取），只做视觉定位。
    if point_pick && pick_color && app.phase != Phase::Exiting {
        draw_color_picker_hud(app, frozen, buf, bw, bh);
    }
}

/// 选区 → 矩形（`Sel` 是 x/y/w/h，`Rect` 是 left/top/right/bottom）。
fn rect_of(s: &Sel) -> Rect {
    Rect {
        left: s.x,
        top: s.y,
        right: s.right(),
        bottom: s.bottom(),
    }
}

/// 半透明前景混合到不透明背景上（Opaque 模式专用：目标平台没有逐像素 alpha）。
fn blend_over(under: u32, over: u32) -> u32 {
    let a = (over >> 24) & 0xFF;
    if a == 255 {
        return over | 0xFF00_0000;
    }
    if a == 0 {
        return under | 0xFF00_0000;
    }
    let inv = 255 - a;
    let mix = |shift: u32| -> u32 {
        let u = (under >> shift) & 0xFF;
        let o = (over >> shift) & 0xFF;
        (u * inv + o * a) / 255
    };
    (0xFF << 24) | (mix(16) << 16) | (mix(8) << 8) | mix(0)
}

/// 填充矩形 + 逐像素混合（`fill_rect` 的 Opaque 版本）。
fn fill_rect_blend(buf: &mut [u32], bw: i32, bh: i32, r: Rect, over: u32) {
    let l = r.left.max(0);
    let t = r.top.max(0);
    let ri = r.right.min(bw);
    let bt = r.bottom.min(bh);
    if ri <= l || bt <= t {
        return;
    }
    for y in t..bt {
        let row = (y as usize) * (bw as usize);
        for x in l..ri {
            let i = row + x as usize;
            buf[i] = blend_over(buf[i], over);
        }
    }
}

#[cfg(test)]
mod hud_tests {
    use super::*;

    /// 把字形渲染成 ASCII（行优先 5 位 → 5 列），便于人工核对与结构校验。
    fn art(ch: char) -> String {
        let rows = hud_glyph(ch).expect("字形必须存在");
        let mut s = String::new();
        for r in rows {
            for c in (0..5).rev() {
                s.push(if (r >> c) & 1 == 1 { '#' } else { '.' });
            }
            s.push('\n');
        }
        s
    }

    /// 所有 HUD 字形都必须是 5 列 × 7 行且非空（数据手抄，防止漏行/超 5 位）。
    #[test]
    fn hud_glyphs_are_well_formed() {
        for ch in "0123456789ABCDEF#RGB".chars() {
            let a = art(ch);
            assert_eq!(a.lines().count(), 7, "{ch} 行数不是 7");
            for line in a.lines() {
                assert_eq!(line.chars().count(), 5, "{ch} 列数不是 5");
            }
            assert!(a.contains('#'), "{ch} 是空字形");
        }
    }

    /// 关键字形打印（`--nocapture` 下人工核对）。
    #[test]
    fn hud_glyphs_print_for_review() {
        for ch in "0348AF#RGB".chars() {
            println!("---- {ch} ----\n{}", art(ch));
        }
    }

    /// 文本绘制必须落在调用方给定区域内，且逐字推进（字距 = 字宽 + 间隔）。
    #[test]
    fn draw_hud_text_advances_and_writes() {
        let (bw, bh) = (160, 32);
        let mut buf = vec![0u32; (bw * bh) as usize];
        let w = draw_hud_text(&mut buf, bw, bh, 2, 2, "#A0 R1", 0xFFFF_FFFF);
        assert_eq!(w, 6 * HUD_ADVANCE, "占用宽度 = 字符数 × 字距");
        let lit = buf.iter().filter(|p| **p == 0xFFFF_FFFF).count();
        assert!(lit > 0, "必须有像素写出");
        // 越界保护：右下角起步不应 panic，也不该写到缓冲外（fill_rect 自带 clamp）
        let _ = draw_hud_text(&mut buf, bw, bh, bw - 3, bh - 3, "AB", 0xFFFF_FFFF);
    }
}
