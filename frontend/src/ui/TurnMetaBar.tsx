/**
 * TurnMetaBar —— 一轮执行的元数据条（耗时 / 上下文增量 / 步数），chip 视觉。
 *
 * ## 为什么是「唯一组件」
 *
 * 三处接入点（assistant 消息底部 / 执行面板 ctx 弹窗 / 移动端模型信息卡）必须
 * **共用同一数据源同一个渲染**。若各自实现，必然各自计时、各自格式化，最终像
 * 原缺陷那样三处显示三个不同的耗时。这里收口：
 *   - 数据：上游传 `meta: TurnMeta`（后端权威）
 *   - 判空：`isTurnMetaEmpty` → 不渲染空壳条（与 `trace_items` 同策略）
 *   - 标签：`turn.*` 走 i18n，zh/en 双侧齐备
 *
 * ## 令牌项 = 上下文增量（不是累计消耗）
 *
 * `tokens` slot 显示 `meta.contextDeltaTokens`（轮末占用 − 轮初占用，后端权威）。
 * 曾经这里显示「本轮所有 LLM 调用的 token 合计」——同一段上下文在同轮多次调用里
 * 被反复计入，累加和随调用次数线性膨胀（130K × 40 次 = 5.2M，实测出现过 11.2M），
 * 对用户是没有信息量的数字。增量回答「这一轮让上下文长了多少」。
 *
 * ## 耗时权威性（缺陷根因）
 *
 * 执行中前端**不持有起点**：起点由后端 `started_at_ms` 下发，调用方以
 * `Date.now() - started_at_ms` 实时推算后经 `liveDurationMs` 传入。刷新页面后
 * 起点仍是后端绝对值，计时从真实起点继续走，不归零。完成后以 `meta.durationMs`
 * 为准（后端权威值）。
 *
 * ## slot 化
 *
 * `slots` 决定显示哪几项，顺序即传入顺序；缺数据的 slot 自动跳过。
 * 消息底部给全三 slot；紧凑场景（如移动端）可只给 `['duration']`。
 */

import { useEffect, useState } from 'react'
import { useLanguage } from '../locales'
import type { TurnMeta } from '../core/types'
import { isTurnMetaEmpty, resolveTurnCalls, resolveTurnDuration } from '../core/types'
import { formatTokens } from './tokenFormat'

/** 元数据条可展示槽位 */
export type TurnMetaSlot = 'duration' | 'tokens' | 'steps'

export interface TurnMetaBarProps {
  /**
   * 后端权威元数据。耗时规则由 `resolveTurnDuration` 统一：
   *   · 有 `durationMs`（turn 已结束）→ 直接显示
   *   · 否则有 `startedAtMs`（进行中）→ `Date.now() - startedAtMs` 实时推算
   * 故本组件**不接受**调用方传入的实时值——那会让同一指标出现第二个来源。
   *
   * 走秒（2026-10-19 局部化改造，节奏按大王定调 200ms）：进行中的轮次**由本组件
   * 内部自tick**——只重渲染这一条元数据条，不再由调用方在顶层 forceTick 整树。
   * 历史原因：顶层 tick 曾在巨大会话（1056 消息）执行期把主线程压到心跳饿死。
   * 已结束的轮次（有 durationMs）不 tick，历史消息零定时器成本。
   */
  meta: TurnMeta | null | undefined
  /** 展示哪些槽位（默认全三 slot：耗时 / 令牌 / 步数） */
  slots?: TurnMetaSlot[]
  /** 附加 class（消息内 / 弹窗内视觉微调） */
  className?: string
  /**
   * 当前正在执行的工具（实时 timeline 最后一个 running 的 tool_call，大王
   * 2026-10-22 定）：执行态提示只跟随**这一条工具**——命令结束（成功或失败）
   * 立即消失，不等整轮结束。
   */
  activeTool?: TurnMetaBarActiveTool | null
}

/** 当前执行中的工具（只需 name / params 用于判断命令类型） */
export interface TurnMetaBarActiveTool {
  id: string
  toolName?: string
  params?: unknown
}

/** 毫秒 → 紧凑可读时长（与执行面板 / 输入栏同款格式，避免三处各写一套） */
export function formatTurnDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return '0ms'
  if (ms < 1000) return `${Math.round(ms)}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  const m = Math.floor(ms / 60000)
  const s = Math.floor((ms % 60000) / 1000)
  return `${m}m ${s}s`
}

/** 命令动作分类（2026-10-26 重做，大王实测驱动）。
 *
 * ## 旧实现的三个问题
 *  ① **类别优先级抢判定**（测试 > 构建 > 检查）：我（模型）常把一条命令写成
 *     `cd …; cargo fmt …; cargo check …; cargo test …` 串联，只要句子里出现 test，
 *     整条就显示 Testing —— 用户在前两个阶段看到的文案是错的；
 *  ② 宽正则误伤：`lint` / `--check` 会命中路径、参数等非动作文本；
 *  ③ 覆盖窄：install / download / sleep 一律退化 Running，用户看不出在干什么。
 *
 * ## 新规则（可解释、可测）
 *  1. **剥壳**：`pwsh|powershell [-开关] -Command "…"` 取内层命令；
 *  2. **切段**：按 `;` / `&&` / `||` / 换行 切成命令段；段内取**管道最左侧**命令
 *     （`… | Select-String` 的右侧是消费端，不是动作）；
 *  3. **跳过噪声段**：cd / set / echo / mkdir / $env: / exit 之类没有动作语义；
 *  4. **取首个命中类别的段**：命令序列通常"先做什么，动作就从那句开始"，
 *     不再按类别优先级抢（那正是 check 被 test 吞掉的根因）；
 *  5. 逐类别**动词级**匹配；全都判不出 → Running...（不猜）。
 *
 * 无中英对照需求，直接用英文（2026-10-22 大王定）。
 */

/** 噪声段：没有动作语义的前置/配置命令 */
const SHELL_NOISE_SEG =
  /^\s*(cd|set|setx|mkdir|md|echo|write-output|write-host|exit|\$env:|if|else|try|fi|then|do|done)\b/i

/** 命令段拆分：剥壳 → 切段 → 段内取管道左侧 → 去噪声 */
function shellSegments(command: string): string[] {
  let cmd = command
  // 剥壳：pwsh / powershell 的 -Command "…" 内层才是真命令
  const wrapped = cmd.match(
    /(?:pwsh|powershell)(?:\.exe)?\b[^"'`]*?-c(?:ommand)?\s+(["'`])([\s\S]*)\1/i,
  )
  if (wrapped) cmd = wrapped[2]
  return cmd
    .split(/;|&&|\|\||\r?\n/)
    .map(seg => seg.split('|')[0].trim())
    .filter(seg => seg.length > 0 && !SHELL_NOISE_SEG.test(seg))
}

/** 类别判定表：**顺序即类别顺序**，逐段试探（动词级匹配，避免子串误伤） */
const SHELL_ACTION_PATTERNS: ReadonlyArray<readonly [RegExp, string]> = [
  [
    /\b(cargo\s+(test|nextest)|nextest|vitest|pytest|jest|mocha|npm\s+(run\s+)?test|pnpm\s+(run\s+)?test|yarn\s+(run\s+)?test|go\s+test|dotnet\s+test|ctest)\b/i,
    'Testing...',
  ],
  [
    /\b(cargo\s+install|npm\s+(i|install|ci)|pnpm\s+(i|install|add)|yarn\s+(add|install)|bun\s+(add|install)|pip3?\s+install|winget\s+install|choco\s+install|scoop\s+install|apt(-get)?\s+install|brew\s+install|dotnet\s+(add|tool))\b/i,
    'Installing...',
  ],
  [
    /\b(curl|wget|invoke-webrequest|start-bitstransfer|git\s+(clone|pull|fetch)|gh\s+release\s+download)\b/i,
    'Downloading...',
  ],
  [
    /\b(cargo\s+(build|rustc)|tauri\s+build|npm\s+run\s+build|pnpm\s+(run\s+)?build|yarn\s+(run\s+)?build|vite\s+build|webpack|rollup|esbuild|go\s+build|dotnet\s+build|msbuild|cmake|make)\b/i,
    'Building...',
  ],
  [
    /\b(cargo\s+(check|clippy|fmt)|rustfmt|tsc|eslint|prettier|stylelint|ruff|mypy|pyright|golangci-lint|go\s+vet|lint|--check|--noemit|-d\s+warnings)\b/i,
    'Checking...',
  ],
  [/\b(start-sleep|sleep|timeout|wait)\b/i, 'Waiting...'],
  [/\b(stop-process|taskkill|process_kill|kill)\b/i, 'Stopping...'],
]

/** 执行态提示文案（大王 2026-10-22 定）：按 system_shell 命令类型**单项显示**；
 *  判断不出类型或非 shell 工具回落 Running。细则见上方分类说明。 */
export function runningToolLabel(tool: TurnMetaBarActiveTool): string {
  const isShell = tool.toolName === 'system_shell' || tool.toolName === 'shell'
  if (!isShell) return 'Running...'
  const p = tool.params as { command?: unknown } | null | undefined
  const cmd = typeof p?.command === 'string' ? p.command : ''
  if (!cmd.trim()) return 'Running...'
  for (const seg of shellSegments(cmd)) {
    for (const [re, label] of SHELL_ACTION_PATTERNS) {
      if (re.test(seg)) return label
    }
  }
  return 'Running...'
}

/** 耐心阈值（毫秒）：单条工具执行超过 60s 才提示（HUD 单次运行同款节奏） */
const PATIENCE_DELAY_MS = 60_000

/** 执行中状态指示（2026-10-31 大王定 v4「去边框均衡器」）：两根竖条高低起伏、
 *  相位错开半拍——与虚拟形象 working 态同源（avatar.css `nvEq`：1.1s /
 *  scaleY .55→1 / 右条 -0.55s），去掉外壳只留双竖，读「正在动手」。
 *  契约延续：**不做旋转**（旋转语义是加载等待，长任务会被读成卡住）。
 *  SVG 10×10 无边框无背景，chip 左间距仍由图标自身宽度承担。 */
function LiveThinking() {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      fill="none"
      className="turn-meta-thinking"
      aria-hidden="true"
    >
      {/* 双竖：底端对齐、圆头收口；缩放由 CSS 驱动（transform-origin: bottom） */}
      <path
        d="M3 8.6 L3 1.4"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        className="turn-meta-thinking-bar-l"
      />
      <path
        d="M7 8.6 L7 1.4"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        className="turn-meta-thinking-bar-r"
      />
    </svg>
  )
}

export function TurnMetaBar({
  meta,
  slots = ['duration', 'tokens', 'steps'],
  className,
  activeTool,
}: TurnMetaBarProps) {
  const { t } = useLanguage()

  // 进行中走秒：仅当「有起点、无终值」（轮次进行中）时本组件内部 200ms 自tick。
  // 只重渲染这一条 bar（useState 在本组件），历史轮次（有 durationMs）不挂定时器。
  const isLive = meta?.durationMs == null && (meta?.startedAtMs ?? 0) > 0
  const [, forceTick] = useState(0)
  useEffect(() => {
    if (!isLive) return
    const timer = window.setInterval(() => forceTick(n => n + 1), 200)
    return () => window.clearInterval(timer)
  }, [isLive])

  // 执行态提示（大王 2026-10-22 定调，两次修正）：
  //   · 只跟随**当前这一条工具**（activeTool），工具结束（成功或失败）立即消失，
  //     不等整轮结束——失败 / 中断若没收到 tool_call_end，轮次收尾时 activeTool
  //     也会变 null，同样立即消失；
  //   · 单条工具执行**超过 60s** 才出现（HUD 耐心提示同款节奏）；
  //   · 文案按 system_shell 命令类型单项显示（Testing / Building / Checking），
  //     判不出类型回落 Running...（无中英对照需求，直接英文）。
  // 只依赖 activeTool.id 起定时器：timeline 每来一行输出都会产生新数组/新对象，
  // 若依赖对象本身会把计时器反复重置，60s 永远到不了。
  const activeToolId = activeTool?.id ?? null
  const [patience, setPatience] = useState(false)
  useEffect(() => {
    setPatience(false)
    if (!isLive || !activeToolId) return
    const timer = window.setTimeout(() => setPatience(true), PATIENCE_DELAY_MS)
    return () => window.clearTimeout(timer)
  }, [isLive, activeToolId])

  // 判空：duration / token / steps 全空**且无起点**才不渲染（无起点连耗时都推不出）。
  // startedAtMs 必须单独计入判据——「仅带起点」的轮次有耗时可展示，不能当空壳条。
  const hasStart = meta?.startedAtMs != null && meta.startedAtMs > 0
  if (isTurnMetaEmpty(meta) && !hasStart) return null

  const m = meta ?? {}

  // 耗时 / 步数唯一出口，无兜底、无 live 分支：
  //   执行中 → Date.now() - startedAtMs（turn 开始即起跑）
  //   已结束 → durationMs（后端权威）
  // 走秒由调用方（ChatPanel/ChatInputBar）的 tick 触发重渲染，此处不持有时间值。
  const durationMs = resolveTurnDuration(meta)
  // 上下文增量（后端权威）：本轮让上下文长了多少，**不是**多次调用的累计消耗
  const ctxDelta = m.contextDeltaTokens ?? 0
  const steps = resolveTurnCalls(meta)

  // 图标策略（2026-10-22 大王定）：**仅 duration slot 且执行中（isLive）**配 HUD
  // 同款旋转 spinner——它是状态指示（「在跑」vs「已完成」的区分），不是装饰。
  // 其余 slot（令牌/步数）保持纯文字：只读信息、不是按钮，图标只会拉宽 chip
  // 挤同行其它项（曾给令牌项加 CPU 图标的教训）。
  const chips: { key: TurnMetaSlot; label: string; value: string }[] = []
  for (const slot of slots) {
    if (slot === 'duration' && durationMs > 0) {
      chips.push({ key: slot, label: t('turn.duration'), value: formatTurnDuration(durationMs) })
    } else if (slot === 'tokens' && ctxDelta > 0) {
      chips.push({
        key: slot,
        label: t('turn.ctxDelta'),
        value: `+${formatTokens(ctxDelta)}`,
      })
    } else if (slot === 'steps' && steps > 0) {
      chips.push({ key: slot, label: t('turn.steps'), value: String(steps) })
    }
  }

  // slot 全被数据缺失跳过（如只有起点、无 token 无步数）→ 同样不渲染
  if (chips.length === 0) return null

  return (
    <div className={`turn-meta-bar${className ? ` ${className}` : ''}`}>
      {chips.map(c => (
        <span key={c.key} className={`turn-meta-chip turn-meta-chip--${c.key}`}>
          {c.key === 'duration' && isLive && <LiveThinking />}
          <span className="turn-meta-chip-label">{c.label}</span>
          <span className="turn-meta-chip-value">{c.value}</span>
        </span>
      ))}
      {isLive && patience && activeTool && (
        <span className="turn-meta-chip turn-meta-chip--patience">
          {runningToolLabel(activeTool)}
        </span>
      )}
    </div>
  )
}
