//! hud_arbiter.rs — HUD 并行工具计时仲裁
//!
//! 面向问题的契约（用户口令）：一轮模型往返里多个工具并行时，HUD 计时显示按
//! 优先级仲裁：system shell / system 族工具 > desktop / browser 工具 > 普通工具
//! （普通工具不上 HUD）。且全部工具结束后 elapsed 计时必须退表，不允许出现
//! 「running 永不收尾」。
//!
//! 机制（为什么可靠）：
//! - 旧路径上 running HudUpdate 由四处独立发出（agent/mod.rs 主轮、exec_tool.rs
//!   execute_tool_only 直发、sub_task_loop.rs 预检提示、各 shell 结局），last-write-wins：
//!   并行时低优先级的 running 覆盖高优先级，且部分结局（shell 前台超时被杀、
//!   system 族 hint 工具完成、safety 拦截）没有任何终态 HudUpdate，最后一发
//!   phase=running 永远挂着 → 计时不退表。
//! - ToolCallStart / ToolCallEnd 事件在各执行体（react_loop / sub_task_loop /
//!   workflow_agent）保证成对发出，且含 call_id 与 success —— 把它当作 HUD 工具
//!   生命周期的唯一真相源：Start 注册进活跃集，End 注销并裁决。
//!
//! 状态机（纯派生 + 两片薄记忆，避免「这条终态属于谁」的歧义）：
//!   · active: call_id → {优先级, 工具名}（HUD-eligible 工具的调用登记）
//!   · displayed: 当前上屏的 running 文本（Some = 屏上有 running 计时帧）
//!   · pending_ends: 已见「终态相位」数 − 已被 End 抵消数。终态相位总是属于
//!     「马上要 End 的那个工具」，所以终态(+1)与 End(−1) 一一抵消。
//!   派生： live() = active.len() > pending_ends（集内还有真正在跑的工具）。
//!
//! 同级并行规则：同优先级不互抢（FIFO 稳定，先到者保持显示）；切换由先行者
//! 的 ToolCallEnd 驱动——结束一个，显示切到留任者，计时连续；全部结束合成终
//! 态退表。两个 system_shell 文本相同 → 显示文本去重，视觉上不闪动。
//!
//! 裁决对外三件事：
//!   ① ToolCallStart：注册；更高优先级进场 → 立即切 running
//!   ② running HudUpdate：有活跃集 → 改写为 top 文本（去重）；无 → 原样放行；
//!      终态相位经过：pending+1，若 live → 切回留任 top 的 running（计时续表）
//!   ③ ToolCallEnd：注销 + 抵消一笔 pending；集内留任 → 切到留任 top；
//!      集空且这条 End 未抵消任何终态（该工具从没发过终态 = sub_task 提示
//!      工具 / shell 前台超时等路径）且屏上仍处 running 显示 → 依据 success
//!      合成 done/error 终态，计时器必然退表
//!   flush_all()：任务级事件（ExecutionError / LeaderDone / ExecutionCompleted）
//!   到达时清场 —— 中断路径 ToolCallEnd 缺席，靠它兜底防泄漏。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

// ─────────────────────────────────────────────────────────────
//  优先级
// ─────────────────────────────────────────────────────────────

/// 优先级数值越小越高。0 = system 族，1 = 桌面/网页。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Priority(pub u8);

pub const PRIORITY_SYSTEM: Priority = Priority(0);
pub const PRIORITY_DESKTOP_WEB: Priority = Priority(1);

/// system 族工具与其 HUD 提示文案（与 sub_task_loop 的 system_hint 同源）。
pub fn system_hint(tool: &str) -> Option<&'static str> {
    match tool {
        "system_shell" => Some("执行系统命令"),
        "system_sleep" => Some("等待中"),
        "system_info" => Some("获取系统信息"),
        "system_env_get" => Some("读取环境变量"),
        "process_list" => Some("查看进程"),
        "process_kill" => Some("终止进程"),
        _ => None,
    }
}

/// HUD 参与判定：返回 (优先级, running 显示文本)。None = 普通工具，不参与。
pub fn classify_tool(tool: &str) -> Option<(Priority, String)> {
    if let Some(hint) = system_hint(tool) {
        return Some((PRIORITY_SYSTEM, hint.to_string()));
    }
    if tool.starts_with("desktop_")
        || tool.starts_with("browser_")
        || tool.starts_with("system_")
        || tool.starts_with("process_")
    {
        return Some((PRIORITY_DESKTOP_WEB, format!("{} — 执行中...", tool)));
    }
    None
}

// ─────────────────────────────────────────────────────────────
//  仲裁状态
// ─────────────────────────────────────────────────────────────

struct Entry {
    priority: Priority,
    tool: String,
}

struct ArbiterState {
    /// call_id → 條目。只放 HUD-eligible 工具。
    active: HashMap<String, Entry>,
    /// 当前上屏的 running 文本（Some = 屏上有 running 计时帧）。
    displayed: Option<(String, Priority)>,
    /// 已见未配对的终态相位数（终态 +1 / End −1）。
    pending_ends: usize,
}

static ARBITER: LazyLock<Mutex<ArbiterState>> = LazyLock::new(|| {
    Mutex::new(ArbiterState {
        active: HashMap::new(),
        displayed: None,
        pending_ends: 0,
    })
});

fn with_state<T>(f: impl FnOnce(&mut ArbiterState) -> T) -> T {
    let mut g = match ARBITER.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    f(&mut g)
}

impl ArbiterState {
    fn running_text(e: &Entry) -> String {
        match system_hint(&e.tool) {
            Some(h) => h.to_string(),
            None => format!("{} — 执行中...", e.tool),
        }
    }

    /// 活跃集中优先级最高条目（数值最小）→ (显示文本, 优先级)。
    fn top(&self) -> Option<(String, Priority)> {
        self.active
            .values()
            .min_by_key(|e| e.priority.0)
            .map(|e| (Self::running_text(e), e.priority))
    }

    /// 集内还有真正在跑的工具（条目数 > 未配对终态数）。
    fn live(&self) -> bool {
        self.active.len() > self.pending_ends
    }
}

// ─────────────────────────────────────────────────────────────
//  对外裁决接口（src-tauri emitter 在对应事件处调用）
// ─────────────────────────────────────────────────────────────

/// ToolCallStart：eligible 工具注册进活跃集。
/// 返回 Some(text) = 需要显示/切换 running 文本（调用方用 fresh seq 发 show）；
/// None = 当前显示维持不变。
pub fn on_tool_call_start(call_id: &str, tool: &str) -> Option<String> {
    let (priority, _) = classify_tool(tool)?;
    with_state(|s| {
        s.active.insert(
            call_id.to_string(),
            Entry {
                priority,
                tool: tool.to_string(),
            },
        );
        let Some((top_text, top_prio)) = s.top() else {
            return None;
        };
        let switch = match &s.displayed {
            None => true,
            // 更高优先级进场 → 切换；同级 FIFO 稳定不互抢
            Some((d, dp)) => top_prio < *dp && top_text != *d,
        };
        if switch {
            s.displayed = Some((top_text.clone(), top_prio));
            Some(top_text)
        } else {
            None
        }
    })
}

/// running HudUpdate 事件裁决：有活跃集 → 改写为 top 文本；无活跃集 → 原样放行。
/// 返回 Some(rewrite_text) = 改写显示；None = 保持事件原文。
pub fn on_running_hint() -> Option<String> {
    with_state(|s| {
        let Some((top_text, top_prio)) = s.top() else {
            return None;
        };
        let needs_rewrite = match &s.displayed {
            Some((d, _)) => *d != top_text,
            None => true,
        };
        if needs_rewrite {
            s.displayed = Some((top_text.clone(), top_prio));
            Some(top_text)
        } else {
            None
        }
    })
}

/// HUD 终态相位（done/error/warning/success/info/security）经过时调用：
/// 记一笔未配对终态；若仍有留任在跑，切回留任 top 的 running（计时续表）；
/// 集空则放行终态自然落定（清 running 帧显示）。
/// 返回 Some(text) = 需要补发 running 显示。
pub fn on_hud_terminal() -> Option<String> {
    with_state(|s| {
        s.pending_ends += 1;
        if !s.live() {
            s.displayed = None;
            return None;
        }
        let Some((top_text, top_prio)) = s.top() else {
            return None;
        };
        let needs_switch = match &s.displayed {
            Some((d, _)) => *d != top_text,
            None => true,
        };
        s.displayed = Some((top_text.clone(), top_prio));
        if needs_switch {
            Some(top_text)
        } else {
            None
        }
    })
}

/// ToolCallEnd 裁决。
pub enum ToolEndAction {
    /// 无需任何 HUD 动作。
    None,
    /// 集内留任工具成为显示对象 → 切换 running 显示（计时续表）。
    SwitchRunning(String),
    /// 集空 + 这条 End 未抵消任何终态（该工具从未发过终态 HudUpdate）+
    /// 屏上仍处 running 显示 → 合成终态（done/error），计时退表。
    SynthTerminal { phase: &'static str, text: String },
}

pub fn on_tool_call_end(call_id: &str, tool_name: &str, success: bool) -> ToolEndAction {
    let tool_name = tool_name.to_string();
    with_state(|s| {
        if s.active.remove(call_id).is_none() {
            // 无登记的 End（普通工具/双发）不消耗 pending，也裁决不出文本
            return ToolEndAction::None;
        }
        // 配对抵消：这条 End 对应「已见过的某条终态」（1:1 消耗）
        let had_terminal = s.pending_ends > 0;
        if had_terminal {
            s.pending_ends -= 1;
        }
        match s.top() {
            Some((top_text, top_prio)) => {
                let needs_switch = match &s.displayed {
                    Some((d, _)) => *d != top_text,
                    None => true,
                };
                if needs_switch {
                    s.displayed = Some((top_text.clone(), top_prio));
                    ToolEndAction::SwitchRunning(top_text)
                } else {
                    ToolEndAction::None
                }
            }
            None => {
                s.displayed = None;
                if had_terminal {
                    // 有真实终态垫背：终态已上屏，End 无需动作
                    return ToolEndAction::None;
                }
                // 无终态配对的收尾 = 该工具从没发过终态 HudUpdate
                // —— sub_task 提示工具 / shell 前台超时等路径；此时若屏幕
                // 还停在 running 显示，必须合成终态收尾，否则「running 永不
                // 收尾」复发。（displayed=None 说明终态已被别的路径收过，
                // 无需重复合成。）
                let (phase, desc) = (
                    if success { "done" } else { "error" },
                    if success { "完成" } else { "失败" },
                );
                ToolEndAction::SynthTerminal {
                    phase,
                    text: format!("{} — {}", tool_name, desc),
                }
            }
        }
    })
}

/// 任务级清场（ExecutionError / LeaderDone / ExecutionCompleted 等）：中断/异常
/// 路径 ToolCallEnd 缺席，靠这里兜底防泄漏。
pub fn flush_all() {
    with_state(|s| {
        s.active.clear();
        s.displayed = None;
        s.pending_ends = 0;
    })
}

// ─────────────────────────────────────────────────────────────
//  测试
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        let mut g = match ARBITER.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.active.clear();
        g.displayed = None;
        g.pending_ends = 0;
    }

    fn name(a: &ToolEndAction) -> &'static str {
        match a {
            ToolEndAction::None => "None",
            ToolEndAction::SwitchRunning(_) => "SwitchRunning",
            ToolEndAction::SynthTerminal { .. } => "SynthTerminal",
        }
    }

    #[test]
    fn priority_order_classifies() {
        assert_eq!(classify_tool("system_shell").unwrap().0, PRIORITY_SYSTEM);
        assert_eq!(classify_tool("process_list").unwrap().0, PRIORITY_SYSTEM);
        assert_eq!(
            classify_tool("desktop_screenshot").unwrap().0,
            PRIORITY_DESKTOP_WEB
        );
        assert_eq!(
            classify_tool("browser_navigate").unwrap().0,
            PRIORITY_DESKTOP_WEB
        );
        assert!(classify_tool("Read").is_none());
        assert!(classify_tool("think").is_none());
    }

    #[test]
    fn parallel_switches_to_system_priority() {
        reset();
        // 截图先注册，shell 后注册 → 仲裁切到 system 优先级文本
        let a = on_tool_call_start("c1", "desktop_screenshot");
        assert_eq!(a.as_deref(), Some("desktop_screenshot — 执行中..."));
        let b = on_tool_call_start("c2", "system_shell");
        assert_eq!(b.as_deref(), Some("执行系统命令"));

        // running hint：显示已等于 top → 不重复改写
        assert!(on_running_hint().is_none());

        // shell 结束（真实 done 先经过，配对抵消）→ 留任桌面工具切回显示
        assert!(on_hud_terminal().is_none());
        match on_tool_call_end("c2", "system_shell", true) {
            ToolEndAction::SwitchRunning(t) => assert_eq!(t, "desktop_screenshot — 执行中..."),
            other => panic!("expected SwitchRunning, got {}", name(&other)),
        }
        // 桌面工具收尾（真实 done 经过，count==pending → 集空）→ 无需合成
        assert!(on_hud_terminal().is_none());
        match on_tool_call_end("c1", "desktop_screenshot", true) {
            ToolEndAction::None => {}
            other => panic!("expected None, got {}", name(&other)),
        }
    }

    #[test]
    fn same_level_desktop_fifo_then_switch() {
        reset();
        // 同级 desktop 并行：先到者显示；先结束者结束 → 切到留任者
        let a = on_tool_call_start("c1", "desktop_screenshot");
        assert_eq!(a.as_deref(), Some("desktop_screenshot — 执行中..."));
        let b = on_tool_call_start("c2", "desktop_click");
        assert!(b.is_none(), "同级不抢占");
        match on_tool_call_end("c1", "desktop_screenshot", true) {
            ToolEndAction::SwitchRunning(t) => assert_eq!(t, "desktop_click — 执行中..."),
            other => panic!("expected SwitchRunning, got {}", name(&other)),
        }
        // desktop_screenshot 的 done 终态在 End 前经过 → 不合成，自动隐藏语义保留
        // （顺序为 done 先于 ToolCallEnd，见 sub_task_loop L588→L620）
    }

    #[test]
    fn same_level_system_two_shells_no_flap() {
        reset();
        // 两个 system_shell 并行：文本相同 → 去重不闪烁
        assert_eq!(
            on_tool_call_start("c1", "system_shell").as_deref(),
            Some("执行系统命令")
        );
        assert!(
            on_tool_call_start("c2", "system_shell").is_none(),
            "同级+同文本不切换"
        );
        // shell1 结束（无终态路径=前台超时被杀）→ 留任同文本 → 显示不变
        match on_tool_call_end("c1", "system_shell", false) {
            ToolEndAction::None => {}
            other => panic!("expected None (同文本不变), got {}", name(&other)),
        }
        // shell2 结束（正常路径有 done）→ End 配对抵消 → 无合成
        on_hud_terminal();
        match on_tool_call_end("c2", "system_shell", true) {
            ToolEndAction::None => {}
            other => panic!("expected None, got {}", name(&other)),
        }
    }

    #[test]
    fn missing_terminal_for_hint_tool_gets_synth() {
        reset();
        // system_info 在 sub_task 路径没有终态 HudUpdate —— End 时必须合成收尾
        on_tool_call_start("c1", "system_info");
        match on_tool_call_end("c1", "system_info", true) {
            ToolEndAction::SynthTerminal { phase, text } => {
                assert_eq!(phase, "done");
                assert_eq!(text, "system_info — 完成");
            }
            other => panic!("expected SynthTerminal, got {}", name(&other)),
        }
    }

    #[test]
    fn synth_failure_on_unsuccess() {
        reset();
        on_tool_call_start("c1", "system_shell");
        // 前台超时被杀：无终态 HudUpdate、success=false → 合成 error 终态
        match on_tool_call_end("c1", "system_shell", false) {
            ToolEndAction::SynthTerminal { phase, text } => {
                assert_eq!(phase, "error");
                assert_eq!(text, "system_shell — 失败");
            }
            other => panic!("expected SynthTerminal, got {}", name(&other)),
        }
    }

    #[test]
    fn single_leader_tool_no_double_terminal() {
        reset();
        // Leader 单工具标准链：running → done(真实终态) → ToolCallEnd
        on_tool_call_start("c1", "desktop_click");
        assert!(on_running_hint().is_none());
        // done 终态经过（笔数 1 = 集内 1 → live=false）
        assert!(on_hud_terminal().is_none());
        match on_tool_call_end("c1", "desktop_click", true) {
            ToolEndAction::None => {}
            other => panic!("expected None, got {}", name(&other)),
        }
    }

    #[test]
    fn flush_clears_everything() {
        reset();
        on_tool_call_start("c1", "system_shell");
        on_hud_terminal();
        flush_all();
        assert!(on_hud_terminal().is_none());
        match on_tool_call_end("c1", "system_shell", true) {
            ToolEndAction::None => {}
            other => panic!("flushed entry: expected None, got {}", name(&other)),
        }
    }
}
