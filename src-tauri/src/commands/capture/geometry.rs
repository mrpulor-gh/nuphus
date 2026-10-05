//! 截图遮罩的几何与选区计算（**平台无关**）。
//!
//! 移植自 shot 项目（`shot/src/main.rs`）的 `Sel` / `Point` / `Rect` /
//! `normalize` / `apply_node` / 节点布局逻辑。
//!
//! 与 shot 的差异：**不依赖全局屏幕尺寸**，改由调用方传入 —— 便于单测、
//! 便于将来扩展多显示器。
//!
//! 硬约束（沿用 shot 的设计原则）：
//!   · 负值/越界是**合法输入**，不是错误（normalize 只做左右/上下对调，
//!     绝不把选区按回屏幕；越界部分由渲染与裁剪沿屏幕边缘自然切掉）。
//!   · 夹紧放在**输出侧**（渲染/裁剪），不放在输入侧。

/// 屏幕坐标点（物理像素）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// 矩形（left/top 含，right/bottom 不含）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    /// 全零矩形。派生出的 `Default::default()` 不是 `const fn`，
    /// 无法在 `const fn App::new` 里使用，故单列一个 const 常量。
    pub const ZERO: Rect = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
}

/// 选区（屏幕物理像素坐标，w/h 为正）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Sel {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Sel {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
    /// 屏内可见部分（超出屏幕的边被屏幕边缘切掉）。选区完全在屏外时返回 None。
    /// 渲染（洞/边框/HUD）与裁剪都以它为依据。
    pub fn clamped(&self, screen_w: i32, screen_h: i32) -> Option<Self> {
        let x0 = self.x.max(0).min(screen_w);
        let y0 = self.y.max(0).min(screen_h);
        let x1 = self.right().max(0).min(screen_w);
        let y1 = self.bottom().max(0).min(screen_h);
        if x1 > x0 && y1 > y0 {
            Some(Self::new(x0, y0, x1 - x0, y1 - y0))
        } else {
            None
        }
    }
}

/// 8 节点各自的缩放语义。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    NW,
    NE,
    SW,
    SE,
    W,
    E,
    N,
    S,
}

pub const NODE_COUNT: usize = 8;
pub const NODE_KINDS: [Kind; NODE_COUNT] = [
    Kind::NW,
    Kind::NE,
    Kind::SW,
    Kind::SE,
    Kind::W,
    Kind::E,
    Kind::N,
    Kind::S,
];

// ── 交互参数（与 shot 对齐）──
pub const MIN_SEL_W: i32 = 8;
pub const MIN_SEL_H: i32 = 8;
pub const DRAG_START_PX: i32 = 4; // 超过才算拖拽
pub const NODE_CORNER: i32 = 13; // 角节点方块边长
pub const NODE_EDGE: i32 = 9; // 边节点方块边长
pub const NODE_OUT: i32 = 8; // 节点中心相对洞边界外移量
pub const NODE_HIT_PAD: i32 = 4; // 节点命中外扩（指尖友好）
/// OK/X 按钮尺寸与间距：对齐宿主 UI 的按钮观感（小、扁、6px 圆角）。
/// 旧值 116x38 + 间距 10 在遮罩上像两块大砖头（2026-10-05 大王要求改小、改精细）。
pub const BTN_W: i32 = 58;
pub const BTN_H: i32 = 26;
pub const BTN_GAP: i32 = 8;

/// 归一化选区：左右对调、上下对调，保证 w/h 为正且不小于最小尺寸。
///
/// **故意不把选区按回屏幕内**。理由：选区允许超出屏幕 —— 用户把右边拖到
/// 左边左边，语义就是「让右边变成新的左边」（横向负值 → 左右对调）；
/// 把选区拖出屏幕边缘，语义就是「只要屏幕内那一块」。超出的部分由
/// **渲染**（洞只画屏内可见部分）与**裁剪**（按屏幕边界 clamp）自然切掉。
pub fn normalize(s: Sel) -> Sel {
    let x0 = s.x.min(s.x + s.w);
    let y0 = s.y.min(s.y + s.h);
    let w = s.w.abs().max(MIN_SEL_W);
    let h = s.h.abs().max(MIN_SEL_H);
    Sel::new(x0, y0, w, h)
}

/// 按锚点与当前点构造选区（拖拽重划用），并夹到屏幕内。
pub fn sel_from_anchor(a: Point, bx: i32, by: i32, screen_w: i32, screen_h: i32) -> Sel {
    let bx = bx.max(0).min(screen_w);
    let by = by.max(0).min(screen_h);
    let x0 = a.x.min(bx).max(0);
    let x1 = a.x.max(bx).min(screen_w);
    let y0 = a.y.min(by).max(0);
    let y1 = a.y.max(by).min(screen_h);
    let w = (x1 - x0).max(MIN_SEL_W).min(screen_w - x0);
    let h = (y1 - y0).max(MIN_SEL_H).min(screen_h - y0);
    normalize(Sel::new(x0, y0, w, h))
}

/// 8 节点调整选区。
///
/// 负值/越界是**合法输入**：把右边拖到左边的左边，用户意图就是「让右边变成
/// 新的左边」（缩小并翻转对角）。所以这里**禁止用"钳住不让过"来处理** ——
/// 那会让缩放直接失效，也会让选区被顶在屏幕边上出不去。
pub fn apply_node(s: Sel, k: Kind, px: i32, py: i32, screen_w: i32, screen_h: i32) -> Sel {
    let cx = px.max(0).min(screen_w);
    let cy = py.max(0).min(screen_h);
    let (l, t, r, b) = (s.x, s.y, s.right(), s.bottom());
    let (mut nl, mut nr, mut nt, mut nb) = (l, r, t, b);
    match k {
        Kind::NW => {
            nl = cx;
            nt = cy;
        }
        Kind::NE => {
            nr = cx;
            nt = cy;
        }
        Kind::SW => {
            nl = cx;
            nb = cy;
        }
        Kind::SE => {
            nr = cx;
            nb = cy;
        }
        Kind::W => nl = cx,
        Kind::E => nr = cx,
        Kind::N => nt = cy,
        Kind::S => nb = cy,
    }
    let x0 = nl.min(nr);
    let x1 = nl.max(nr);
    let y0 = nt.min(nb);
    let y1 = nt.max(nb);
    if x1 - x0 < MIN_SEL_W || y1 - y0 < MIN_SEL_H {
        return s;
    }
    Sel::new(x0, y0, x1 - x0, y1 - y0)
}

pub fn hit_rect(r: &Rect, px: i32, py: i32) -> bool {
    r.right > r.left && px >= r.left && px < r.right && py >= r.top && py < r.bottom
}

pub fn node_is_corner(i: usize) -> bool {
    matches!(NODE_KINDS[i], Kind::NW | Kind::NE | Kind::SW | Kind::SE)
}

/// 8 节点位置：中心一律**外移到洞边界之外**（画在洞里=不渲染=看不见）。
pub fn layout_nodes(sel: Sel, out: &mut [Point; NODE_COUNT]) {
    let (l, t, r, b) = (sel.x, sel.y, sel.right(), sel.bottom());
    let (cx, cy) = ((l + r) / 2, (t + b) / 2);
    let o = NODE_OUT;
    out[0] = Point { x: l - o, y: t - o }; // NW
    out[1] = Point { x: r + o, y: t - o }; // NE
    out[2] = Point { x: l - o, y: b + o }; // SW
    out[3] = Point { x: r + o, y: b + o }; // SE
    out[4] = Point { x: l - o, y: cy }; // W
    out[5] = Point { x: r + o, y: cy }; // E
    out[6] = Point { x: cx, y: t - o }; // N
    out[7] = Point { x: cx, y: b + o }; // S
}

/// 命中某个节点（半径按角/边分别算 + 外扩）。
pub fn hit_node_at(pts: &[Point; NODE_COUNT], px: i32, py: i32) -> Option<usize> {
    (0..NODE_COUNT).find(|&i| {
        let p = pts[i];
        let half = (if node_is_corner(i) {
            NODE_CORNER
        } else {
            NODE_EDGE
        }) / 2
            + NODE_HIT_PAD;
        (px - p.x).abs() <= half && (py - p.y).abs() <= half
    })
}

/// OK/X 按钮位置：**整组水平排列、居中贴选区下边（或上边）外侧**，永远在屏内且不重叠。
///
/// 旧实现贴「右下斜角」：选区贴屏幕右/下边缘时，OK 被回退进选区内部、X 再被屏幕边界
/// `min` 回来 → **两个按钮叠压到一起**（2026-10-05 大王实测截图确认）。
///
/// 现在的规则：
/// 1. 组内间距固定（OK 在左、X 在右，间距 `BTN_GAP`）→ 任何情况下都不重叠；
/// 2. 垂直优先「下边外侧」（多留 `NODE_OUT` 避开 S 节点），放不下翻到「上边外侧」
///    （同样避开 N 节点），都放不下才退回屏内最下方（保证可见可点）；
/// 3. 水平按选区中心居中后整体 clamp 到屏内 → 贴边也不会被压扁或叠在一起。
pub fn layout_buttons(sel: Sel, screen_w: i32, screen_h: i32) -> (Rect, Rect) {
    let group_w = BTN_W * 2 + BTN_GAP;
    let cx = sel.x + sel.w / 2;
    let gx = (cx - group_w / 2).clamp(0, (screen_w - group_w).max(0));

    let below = sel.bottom() + BTN_GAP + NODE_OUT;
    let above = sel.y - BTN_GAP - NODE_OUT - BTN_H;
    let gy = if below + BTN_H <= screen_h {
        below
    } else if above >= 0 {
        above
    } else {
        (screen_h - BTN_H).max(0)
    };

    let ok = Rect {
        left: gx,
        top: gy,
        right: gx + BTN_W,
        bottom: gy + BTN_H,
    };
    let xl = gx + BTN_W + BTN_GAP;
    let x = Rect {
        left: xl,
        top: gy,
        right: xl + BTN_W,
        bottom: gy + BTN_H,
    };
    (ok, x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_flips_negative_w() {
        // 右边拖到左边左边 → 左右对调，宽为正
        let s = normalize(Sel::new(100, 50, -40, 30));
        assert_eq!(s, Sel::new(60, 50, 40, 30));
    }

    #[test]
    fn normalize_keeps_out_of_screen() {
        // 越界不按回屏幕内（夹紧在输出侧）
        let s = normalize(Sel::new(-50, -20, 30, 30));
        assert_eq!(s, Sel::new(-50, -20, 30, 30));
    }

    #[test]
    fn clamped_cuts_to_screen() {
        let s = Sel::new(-10, -10, 50, 50).clamped(1920, 1080).unwrap();
        assert_eq!(s, Sel::new(0, 0, 40, 40));
    }

    #[test]
    fn clamped_offscreen_is_none() {
        assert!(Sel::new(2000, 2000, 10, 10).clamped(1920, 1080).is_none());
    }

    #[test]
    fn apply_node_se_moves_corner() {
        let s = Sel::new(100, 100, 200, 200);
        let n = apply_node(s, Kind::SE, 400, 400, 1920, 1080);
        assert_eq!(n, Sel::new(100, 100, 300, 300));
    }

    #[test]
    fn apply_node_too_small_keeps_original() {
        let s = Sel::new(100, 100, 200, 200);
        let n = apply_node(s, Kind::SE, 101, 101, 1920, 1080); // 缩到 < MIN
        assert_eq!(n, s);
    }

    #[test]
    fn hit_node_finds_corner() {
        let s = Sel::new(100, 100, 200, 200);
        let mut pts = [Point::default(); NODE_COUNT];
        layout_nodes(s, &mut pts);
        // NW 节点中心 = (100-8, 100-8) = (92,92)
        assert_eq!(hit_node_at(&pts, 92, 92), Some(0));
        assert_eq!(hit_node_at(&pts, 1000, 1000), None);
    }

    #[test]
    fn sel_from_anchor_clamps_to_screen() {
        let s = sel_from_anchor(Point { x: 10, y: 10 }, 5000, 5000, 1920, 1080);
        assert_eq!(s.right(), 1920);
        assert_eq!(s.bottom(), 1080);
    }

    #[test]
    fn layout_buttons_never_overlap_at_screen_corner() {
        // 选区贴屏幕右下角 —— 旧实现在这里把 OK / X 挤到一起
        let sel = Sel::new(1920 - 320, 1080 - 220, 320, 220);
        let (ok, x) = layout_buttons(sel, 1920, 1080);
        assert!(ok.right() + BTN_GAP <= x.left, "OK/X 之间必须留出间距");
        assert!(ok.left >= 0 && x.right() <= 1920, "整组必须在屏内");
        assert!(ok.bottom() <= sel.y, "下方放不下时应翻到选区上边外侧");
    }

    #[test]
    fn layout_buttons_fit_for_full_screen_selection() {
        // 选区占满整屏：仍必须在屏内且互不重叠（退到屏内最下方）
        let sel = Sel::new(0, 0, 1920, 1080);
        let (ok, x) = layout_buttons(sel, 1920, 1080);
        assert!(ok.right() + BTN_GAP <= x.left, "OK/X 之间必须留出间距");
        assert!(
            ok.left >= 0 && x.right() <= 1920 && ok.top >= 0 && x.bottom() <= 1080,
            "整组必须在屏内"
        );
    }
}
