//! 截图遮罩的交互状态机（**平台无关**）。
//!
//! 移植自 shot 项目 `shot/src/main.rs` 的 `Phase` / `Op` / `App` 及
//! `app_ldown` / `app_move` / `app_lup` / `app_set_hover_target`。
//!
//! 与 shot 的差异：
//!   · **无全局状态**：屏幕尺寸由调用方传入；动作通过返回值 `Action` 表达，
//!     不在状态机里直接调退出/保存（那些属平台/编排层）。
//!   · 纯计算：不碰 GDI、不碰文件、不做任何 IO。

use super::geometry::{
    apply_node, hit_node_at, layout_buttons, layout_nodes, normalize, sel_from_anchor, Kind, Point,
    Rect, Sel, DRAG_START_PX, NODE_COUNT, NODE_KINDS,
};

/// 交互相位。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// 选区跟随鼠标 hover 到的桌面窗口
    Hover,
    /// 洞内按下并拖动 → 重划选区
    Dragging,
    /// 选区已固定，右下角 OK/X + 8 节点
    Confirm,
    /// 确认态选区内拖动 → 原尺寸平移
    Moving,
    /// 确认态拖动 8 节点之一 → 改大小
    Adjusting,
    /// 已点 OK/X：HUD 已撤、遮罩仍在，等待冷却后销毁。
    /// 存在这一相是为了**不把松手/余点击漏给下层应用**。
    Exiting,
}

/// 确认态下的复合操作目标。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    None,
    Node(usize),
    Move { dx: i32, dy: i32 },
}

/// 状态机对外请求的动作（由编排层执行）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    /// 点 OK / 回车：从冻结帧裁剪选区并保存，随后退出
    ConfirmSave,
    /// 点 X / ESC / 右键：取消并退出
    Cancel,
}

/// 唯一交互状态。
#[derive(Clone, Copy)]
pub struct App {
    pub phase: Phase,
    pub sel: Sel,
    pub anchor: Point,
    pub pending: bool,
    pub started: bool,
    pub op: Op,
    pub last_hover_ms: u32,
    pub last_mouse: Point,
    pub btn_ok_hot: bool,
    pub btn_x_hot: bool,
    pub node_pts: [Point; NODE_COUNT],
    pub btn_ok: Rect,
    pub btn_x: Rect,
    pub dirty: bool,
}

impl App {
    /// 初始态：Hover 相位 + 全屏选区（首帧 = 整屏冻结帧，无压暗洞）。
    pub const fn new(screen_w: i32, screen_h: i32) -> Self {
        Self {
            phase: Phase::Hover,
            sel: Sel::new(0, 0, screen_w, screen_h),
            anchor: Point { x: 0, y: 0 },
            pending: false,
            started: false,
            op: Op::None,
            last_hover_ms: 0,
            last_mouse: Point { x: -1, y: -1 },
            btn_ok_hot: false,
            btn_x_hot: false,
            node_pts: [Point { x: 0, y: 0 }; NODE_COUNT],
            btn_ok: Rect::ZERO,
            btn_x: Rect::ZERO,
            dirty: true,
        }
    }

    /// 用 hover 探测到的窗口边界更新选区（仅 Hover 相位 + 未按下时生效）。
    /// `hit = None` → 回退到全屏。
    pub fn set_hover_target(&mut self, hit: Option<Sel>, screen_w: i32, screen_h: i32) {
        if self.phase != Phase::Hover || self.pending {
            return;
        }
        let next = normalize(hit.unwrap_or_else(|| Sel::new(0, 0, screen_w, screen_h)));
        if next != self.sel {
            self.sel = next;
            self.dirty = true;
        }
    }

    /// 单点拾取（取色 / 鼠标坐标模式）：把选区设成该点（1x1）并进入确认相位。
    /// 调用方随后 `request_exit(Confirm)`，走与矩形模式完全相同的收尾路径。
    pub fn pick_point_at(&mut self, px: i32, py: i32) {
        self.sel = Sel::new(px, py, 1, 1);
        self.phase = Phase::Confirm;
        self.dirty = true;
    }

    /// 鼠标按下。
    pub fn on_ldown(&mut self, px: i32, py: i32, screen_w: i32, screen_h: i32) -> Action {
        self.anchor = Point { x: px, y: py };
        self.pending = true;
        self.started = false;
        match self.phase {
            Phase::Hover => {}
            Phase::Exiting => {} // Exiting 吞掉一切输入
            Phase::Confirm => {
                // 重新计算按钮/节点命中位置（可能与上一帧尺寸不同）
                self.recompute_hud(screen_w, screen_h);
                if crate::commands::capture::geometry::hit_rect(&self.btn_ok, px, py) {
                    self.phase = Phase::Exiting;
                    self.op = Op::None;
                    self.pending = false;
                    self.started = false;
                    self.dirty = true;
                    return Action::ConfirmSave;
                }
                if crate::commands::capture::geometry::hit_rect(&self.btn_x, px, py) {
                    self.phase = Phase::Exiting;
                    self.op = Op::None;
                    self.pending = false;
                    self.started = false;
                    self.dirty = true;
                    return Action::Cancel;
                }
                if let Some(i) = hit_node_at(&self.node_pts, px, py) {
                    self.op = Op::Node(i);
                    self.phase = Phase::Adjusting;
                } else if self.sel.contains(px, py) {
                    self.op = Op::Move {
                        dx: self.sel.x - px,
                        dy: self.sel.y - py,
                    };
                    self.phase = Phase::Moving;
                }
                // 压暗区按下：松手无拖 = 无操作
            }
            Phase::Dragging | Phase::Moving | Phase::Adjusting => {
                // 理论上不会到这（按下必在 Confirm/Hover）；防御性复位。
                self.pending = false;
            }
        }
        self.dirty = true;
        Action::None
    }

    /// 鼠标移动（拖拽实时跟随：只更新参数 + 置脏）。
    pub fn on_move(&mut self, px: i32, py: i32, screen_w: i32, screen_h: i32) {
        if self.pending
            && !self.started
            && ((px - self.anchor.x).abs() > DRAG_START_PX
                || (py - self.anchor.y).abs() > DRAG_START_PX)
        {
            self.started = true;
            match self.phase {
                Phase::Hover => self.phase = Phase::Dragging,
                Phase::Confirm => {
                    if let Op::Move { .. } = self.op {
                        self.phase = Phase::Moving;
                    } else {
                        // 压暗区按下起手 → 同样重划
                        self.phase = Phase::Dragging;
                        self.op = Op::None;
                    }
                }
                _ => {}
            }
        }
        match self.phase {
            Phase::Dragging => {
                let next = sel_from_anchor(self.anchor, px, py, screen_w, screen_h);
                if next != self.sel {
                    self.sel = next;
                    self.dirty = true;
                }
            }
            Phase::Moving => {
                if let Op::Move { dx, dy } = self.op {
                    // 平移允许越界（越界部分渲染与裁剪会沿屏幕边缘切掉）。
                    let next = normalize(Sel::new(px + dx, py + dy, self.sel.w, self.sel.h));
                    if next != self.sel {
                        self.sel = next;
                        self.dirty = true;
                    }
                }
            }
            Phase::Adjusting => {
                if let Op::Node(i) = self.op {
                    let next = apply_node(self.sel, NODE_KINDS[i], px, py, screen_w, screen_h);
                    if next != self.sel {
                        self.sel = next;
                        self.dirty = true;
                    }
                }
            }
            _ => {}
        }
    }

    /// 鼠标松开（单击与拖拽的分界）。
    pub fn on_lup(&mut self, _px: i32, _py: i32) {
        if self.pending && !self.started {
            // 单击：按下点在洞内 = 固定当前外框。
            if self.sel.contains(self.anchor.x, self.anchor.y) && self.phase == Phase::Hover {
                self.phase = Phase::Confirm;
                self.op = Op::None;
            }
        }
        self.pending = false;
        self.started = false;
        if matches!(
            self.phase,
            Phase::Dragging | Phase::Moving | Phase::Adjusting
        ) {
            self.phase = Phase::Confirm;
            self.op = Op::None;
        }
        self.dirty = true;
    }

    /// 进入 Exiting（外部请求退出时调用）。
    pub fn enter_exiting(&mut self) {
        self.phase = Phase::Exiting;
        self.op = Op::None;
        self.pending = false;
        self.started = false;
        self.dirty = true;
    }

    /// 依据当前选区重算 HUD（节点 + 按钮）位置。
    pub fn recompute_hud(&mut self, screen_w: i32, screen_h: i32) {
        let s = self.sel.clamped(screen_w, screen_h).unwrap_or(self.sel);
        layout_nodes(s, &mut self.node_pts);
        let (ok, x) = layout_buttons(s, screen_w, screen_h);
        self.btn_ok = ok;
        self.btn_x = x;
    }

    /// 当前相位下，光标应落在哪个语义位置（供平台层设置光标形状）。
    pub fn cursor_kind(&self, px: i32, py: i32) -> CursorKind {
        use crate::commands::capture::geometry::hit_rect;
        match self.phase {
            Phase::Hover | Phase::Dragging | Phase::Exiting => CursorKind::Cross,
            Phase::Confirm => {
                if hit_rect(&self.btn_ok, px, py) || hit_rect(&self.btn_x, px, py) {
                    CursorKind::Hand
                } else if let Some(i) = hit_node_at(&self.node_pts, px, py) {
                    CursorKind::Resize(NODE_KINDS[i])
                } else if self.sel.contains(px, py) {
                    CursorKind::SizeAll
                } else {
                    CursorKind::Cross
                }
            }
            Phase::Moving => CursorKind::SizeAll,
            Phase::Adjusting => match self.op {
                Op::Node(i) => CursorKind::Resize(NODE_KINDS[i]),
                _ => CursorKind::SizeAll,
            },
        }
    }
}

/// 光标语义（平台层映射为各自的系统光标）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorKind {
    Cross,
    Hand,
    SizeAll,
    Resize(Kind),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_then_click_enters_confirm() {
        let mut app = App::new(1920, 1080);
        app.set_hover_target(Some(Sel::new(100, 100, 300, 200)), 1920, 1080);
        assert_eq!(app.sel, Sel::new(100, 100, 300, 200));
        // 洞内单击
        app.on_ldown(200, 200, 1920, 1080);
        app.on_lup(200, 200);
        assert_eq!(app.phase, Phase::Confirm);
    }

    #[test]
    fn click_ok_returns_confirm_save() {
        let mut app = App::new(1920, 1080);
        app.set_hover_target(Some(Sel::new(100, 100, 300, 200)), 1920, 1080);
        app.on_ldown(200, 200, 1920, 1080);
        app.on_lup(200, 200);
        app.recompute_hud(1920, 1080);
        let ok = app.btn_ok;
        let act = app.on_ldown(
            (ok.left + ok.right) / 2,
            (ok.top + ok.bottom) / 2,
            1920,
            1080,
        );
        assert_eq!(act, Action::ConfirmSave);
        assert_eq!(app.phase, Phase::Exiting);
    }

    #[test]
    fn click_x_returns_cancel() {
        let mut app = App::new(1920, 1080);
        app.set_hover_target(Some(Sel::new(100, 100, 300, 200)), 1920, 1080);
        app.on_ldown(200, 200, 1920, 1080);
        app.on_lup(200, 200);
        app.recompute_hud(1920, 1080);
        let x = app.btn_x;
        let act = app.on_ldown((x.left + x.right) / 2, (x.top + x.bottom) / 2, 1920, 1080);
        assert_eq!(act, Action::Cancel);
    }

    #[test]
    fn drag_redraws_selection() {
        let mut app = App::new(1920, 1080);
        app.set_hover_target(Some(Sel::new(100, 100, 300, 200)), 1920, 1080);
        app.on_ldown(500, 500, 1920, 1080); // 洞外按下
        app.on_move(600, 650, 1920, 1080); // 拖过阈值
        assert_eq!(app.phase, Phase::Dragging);
        app.on_lup(600, 650);
        assert_eq!(app.phase, Phase::Confirm);
        assert!(app.sel.w > 0 && app.sel.h > 0);
    }
}
