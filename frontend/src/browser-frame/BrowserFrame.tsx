// BrowserFrame.tsx — 独立浏览器窗口的**壳页面**（44px 控制器，一个 Window 的子 webview）。
//
// 架构（docs/browser-shell-arch.md；Rust 侧 src-tauri/src/commands/browser.rs）：
//   宿主 window label = `browser-<n>`（Rust `next_label()` 分配）
//   ├─ {label}-frame   ← 本页面（App URL，固定 44px）
//   └─ {label}-content ← 远程页（External URL，注入标注/录制脚本）
// 本组件只驱动 **content**：所有命令都以窗口 label 为参数，Rust 内部转 -content webview。
//
// ── 自身 label 怎么拿（已查证 tauri 2.11.5 源码，非假设）──
// `@tauri-apps/api/window` 的 `getCurrentWindow()` 读
// `window.__TAURI_INTERNALS__.metadata.currentWindow.label`；而该 metadata 由 tauri 的
// `manager/webview.rs:183-194` 对**每个** webview 注入（含 Window::add_child 的子
// webview），其中 currentWindow.label 填的是**宿主窗口** label。故壳页面直接
// `getCurrentWindow().label` 即得 `browser-<n>` —— 无需 Rust 新增命令、无需按
// `-frame` 后缀做字符串手术。非 Tauri 环境（浏览器直开 / vitest）没有 internals，
// 回落 `?window=<label>` 查询参数（dev 调试用）。
//
// ── 为什么纯轮询、不订阅事件（已查证）──
// Rust 只 `emit_to("main", …)`；即便壳自己 listen，`@tauri-apps/api/event` 的 listen 走
// `plugin:event|listen`（tauri event/plugin.rs:15）是**插件命令**，受 ACL 管辖：
// capabilities/default.json 的 windows 列表是
// ["main","splash","execution","hud","capture_overlay","workflow-monitor"]，
// `browser-*` 不在表内 → 壳页面的 listen 会被拒（ipc/authority.rs:459 匹配不到）。
// 而 browser_* 是**应用命令**且本应用无 app ACL manifest（tauri.conf.json 无 acl 段），
// 本地源直接放行（webview/mod.rs:1823）——所以壳能 invoke，不能 listen。
// 结论：状态同步用轮询（get_state 2s / list_windows 3s），动作后立即补一次。
//
// ── 聚焦其它窗口：本期做不到（已查证，降级）──
// `Window.setFocus()` → `plugin:window|set_focus`（window/plugin.rs:13 按 label 参数
// 解析目标窗口，主窗口调用它确实能聚焦 browser-*），但壳自己的 label 是 browser-*，
// 同样过不了 capability 的 windows 列表 → ACL 拒绝。故 tab 点击非本窗口时降级为
// 「刷新列表 + 明确日志」，聚焦能力等下个 Rust 任务补 `browser_focus_window(label)`。

import { useCallback, useEffect, useRef, useState } from 'react'
import { ArrowLeft, ArrowRight, Circle, Loader2, PenTool, RefreshCw, X } from 'lucide-react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  browserClearRecording,
  browserClose,
  browserGetRecording,
  browserGetState,
  browserGoBack,
  browserGoForward,
  browserListWindows,
  browserNavigate,
  browserRecordAction,
  browserReload,
  type BrowserState,
  type BrowserWindowInfo,
} from '../main-window/lib/api'
import { useLanguage } from '../locales'
import { getNuphusBrowser } from './nuphusBrowser'
import './browser-frame.css'

/** 自身状态轮询间隔：远程页重定向 / 点链接跳转壳不参与，只能靠轮询跟上地址栏 */
const STATE_POLL_MS = 2000
/** tab 列表轮询间隔（架构文档 §4.2 定 3s）：多窗口增删的次要信息，不必更勤 */
const TAB_POLL_MS = 3000
/** 错误 chip 自动消失：44px 栏内放不下常驻错误条，4s 足够读完一句 */
const ERROR_AUTO_DISMISS_MS = 4000

/** 窗口尚未拿到 label / get_state 未回来时的占位态（全 false，按钮全部置灰） */
const INITIAL_STATE: BrowserState = {
  url: '',
  title: '',
  loading: false,
  canGoBack: false,
  canGoForward: false,
}

/** 从后端错误（string 或 Error）取出可展示文案 */
function errorText(e: unknown): string {
  if (typeof e === 'string') return e
  if (e instanceof Error) return e.message
  return String(e)
}

/**
 * 归一化用户输入：裸域名补 https://，其余原样交给后端裁决。
 *
 * 刻意**不在前端**判定「允许的 scheme」——安全边界只有后端一处实现
 * （browser.rs::parse_remote_url），前端再写一份必然规则漂移。
 * 这里只做便利性补全，不做安全判定。
 */
export function normalizeUrl(raw: string): string {
  const trimmed = raw.trim()
  if (!trimmed) return ''
  // 已有 scheme（http/https/其它）原样透传，由后端裁决放行与否
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(trimmed)) return trimmed
  return `https://${trimmed}`
}

/**
 * 解析壳页面所在的**窗口** label。
 *
 * 主路径 `getCurrentWindow().label`（内部读 tauri 注入的 currentWindow metadata）；
 * 非 Tauri 环境抛异常 / 空 label 时回落 `?window=` 查询参数（dev 直开与单测用）。
 */
export function resolveWindowLabel(): string {
  try {
    const label = getCurrentWindow().label
    if (label) return label
  } catch {
    // 没有 __TAURI_INTERNALS__（浏览器直开 / vitest jsdom）：走下面的回落
  }
  return new URLSearchParams(window.location.search).get('window') ?? ''
}

/** 带 nonce 的错误态：重复错误也要重置自动消失计时（同字符串不会触发 effect 重跑） */
interface FrameError {
  text: string
  nonce: number
}

export function BrowserFrame() {
  const { t } = useLanguage()
  /** 宿主窗口 label（browser-<n>）；空串 = 还没解析出来，命令一律不发的 */
  const [label, setLabel] = useState('')
  /** 本窗口的权威状态（url/title/loading/前进后退可用性）—— 全部来自 get_state */
  const [state, setState] = useState<BrowserState>(INITIAL_STATE)
  /** 地址栏草稿，与 state.url 解耦：编辑中不被轮询回写（理由见文件头旧面板同款注释） */
  const [draft, setDraft] = useState('')
  /**
   * 「用户正在编辑地址栏」标记的 ref。
   * 不用 useState：它只被轮询回写路径**读取**，从不参与渲染；做成 state 会让每次
   * 聚焦都重挂轮询 effect（ref 读最新值又不重挂，与旧 BrowserPanel 同一思路）。
   */
  const editingRef = useRef(false)
  const [error, setError] = useState<FrameError | null>(null)
  const [recording, setRecording] = useState(false)
  const [annotating, setAnnotating] = useState(false)
  /** 全窗口快照（tab 列表）：含本窗口，框用于「有哪些窗口 / 关谁」 */
  const [tabs, setTabs] = useState<BrowserWindowInfo[]>([])
  /** 最近一轮录到的动作条数（停录时取回， UI 只展示计数，回放是后续 CDP 的事） */
  const [recordCount, setRecordCount] = useState(0)
  const addressRef = useRef<HTMLInputElement>(null)
  /**
   * 「下一次 focus 是挂载时的程序聚焦，不算用户编辑」哨兵。
   *
   * 为什么需要：起始页形态要自动聚焦地址栏等输入，但 onFocus 会把 editingRef
   * 置真 → 首帧权威 URL 被当成「用户正在编辑」而永不回写（实测回归：地址栏
   * 停在空串）。故程序聚焦前举牌，onFocus 见到就放行并不计编辑。
   * activeElement 前置检查：StrictMode 双挂载时第二次 focus() 不再派发事件，
   * 哨兵不会被提前消耗掉（否则用户第一次真点击会被误放行）。
   */
  const skipAutoFocusRef = useRef(false)

  const showError = useCallback((text: string | null) => {
    setError(prev => (text ? { text, nonce: (prev?.nonce ?? 0) + 1 } : null))
  }, [])

  /** 错误 chip 自动消失 */
  useEffect(() => {
    if (!error) return
    const timer = window.setTimeout(() => setError(null), ERROR_AUTO_DISMISS_MS)
    return () => window.clearTimeout(timer)
  }, [error])

  /** 挂载即解析自身 label + 聚焦地址栏（起始页形态：等用户输入） */
  useEffect(() => {
    setLabel(resolveWindowLabel())
    if (document.activeElement !== addressRef.current) {
      skipAutoFocusRef.current = true
      addressRef.current?.focus()
    }
  }, [])

  const refreshState = useCallback(async () => {
    if (!label) return
    try {
      const snapshot = await browserGetState(label)
      // 窗口正在关闭时 get_state 会 Err，被下面 catch 静默掉：下一轮轮询自然没有了
      if (snapshot) setState(snapshot)
    } catch {
      /* 静默：轮询是自愈的，弹错误反而打断用户 */
    }
  }, [label])

  const refreshTabs = useCallback(async () => {
    try {
      const list = await browserListWindows()
      if (list) setTabs(list)
    } catch {
      /* tab 列表是辅助信息：失败不弹错误，下一轮轮询再试 */
    }
  }, [])

  /** 轮询：状态 2s（地址栏/按钮态）、tab 3s；label 变化立即补一次首帧 */
  useEffect(() => {
    if (!label) return
    void refreshState()
    void refreshTabs()
    const stateTimer = window.setInterval(() => void refreshState(), STATE_POLL_MS)
    const tabTimer = window.setInterval(() => void refreshTabs(), TAB_POLL_MS)
    return () => {
      window.clearInterval(stateTimer)
      window.clearInterval(tabTimer)
    }
  }, [label, refreshState, refreshTabs])

  /** 地址栏回写：编辑中不回写，否则每轮询一次就把用户输入覆盖掉 */
  useEffect(() => {
    if (!editingRef.current) setDraft(state.url)
  }, [state.url])

  /** 导航到地址栏目标（回车 / Go 共用） */
  const submitAddress = useCallback(async () => {
    if (!label) return
    const target = normalizeUrl(draft)
    if (!target) {
      showError(t('browser.emptyUrl'))
      return
    }
    editingRef.current = false
    showError(null)
    try {
      await browserNavigate(label, target)
      // 乐观回填：失败时下面 refreshState 会把权威 URL 拉回来
      setDraft(target)
    } catch (e) {
      showError(errorText(e) || t('browser.loadFailed'))
    }
    await refreshState()
  }, [draft, label, refreshState, showError, t])

  /** 前进/后退/刷新的公共尾部：发命令 + 立即拉一次权威态（不等下一轮轮询） */
  const runHistoryAction = useCallback(
    async (action: () => Promise<unknown>, fallbackKey: string) => {
      if (!label) return
      showError(null)
      try {
        await action()
      } catch (e) {
        showError(errorText(e) || t(fallbackKey))
      }
      await refreshState()
    },
    [label, refreshState, showError, t],
  )

  /** 录制开关：开录 = 新会话（先清上一轮流），停录 = 取回条数 */
  const toggleRecording = useCallback(async () => {
    if (!label) return
    const next = !recording
    setRecording(next)
    showError(null)
    try {
      if (next) await browserClearRecording(label)
      // 开关状态本身也进录制流：回放时需要知道「从哪一刻起算录到了」
      await browserRecordAction(label, { type: 'record-toggle', on: next, at: Date.now() })
      getNuphusBrowser().setRecording(next)
      if (next) {
        setRecordCount(0)
      } else {
        const list = await browserGetRecording(label)
        setRecordCount(list?.length ?? 0)
      }
    } catch (e) {
      setRecording(!next) // 回滚开关态：命令失败不能留一个「假装在录」的 UI
      showError(errorText(e) || t('browser.recordFailed'))
    }
  }, [label, recording, showError, t])

  /** 标注开关：本期只翻 UI 态 + 过通信缝（缝内是日志，见 nuphusBrowser.ts） */
  const toggleAnnotating = useCallback(() => {
    const next = !annotating
    setAnnotating(next)
    getNuphusBrowser().setAnnotate(next)
  }, [annotating])

  /**
   * tab 点击（非本窗口）：降级路径——见文件头「聚焦其它窗口」段。
   * 不做视觉假切换：点不动就是点不动，刷新列表 + 日志比骗用户的选中态诚实。
   */
  const handleTabClick = useCallback(
    (info: BrowserWindowInfo) => {
      if (info.label === label) return
      console.info(
        `[browser-frame] 聚焦其它浏览窗口需 Rust browser_focus_window（未接入），已刷新列表: ${info.label}`,
      )
      void refreshTabs()
    },
    [label, refreshTabs],
  )

  /** tab ×：关对应窗口（本窗口也会被关——Destroyed 时壳随窗口一起消亡） */
  const handleTabClose = useCallback(
    async (target: string) => {
      showError(null)
      try {
        await browserClose(target)
      } catch (e) {
        showError(errorText(e) || t('browser.closeTab'))
        return
      }
      await refreshTabs()
    },
    [refreshTabs, showError, t],
  )

  return (
    <div className="bf-root">
      <div className="bf-nav">
        <button
          type="button"
          className="bf-icon-btn"
          onClick={() => void runHistoryAction(() => browserGoBack(label), 'browser.backTitle')}
          disabled={!state.canGoBack}
          title={t('browser.backTitle')}
          aria-label={t('browser.back')}
        >
          <ArrowLeft size={14} />
        </button>
        <button
          type="button"
          className="bf-icon-btn"
          onClick={() =>
            void runHistoryAction(() => browserGoForward(label), 'browser.forwardTitle')
          }
          disabled={!state.canGoForward}
          title={t('browser.forwardTitle')}
          aria-label={t('browser.forward')}
        >
          <ArrowRight size={14} />
        </button>
        <button
          type="button"
          className="bf-icon-btn"
          onClick={() => void runHistoryAction(() => browserReload(label), 'browser.reloadTitle')}
          disabled={state.loading}
          title={t('browser.reloadTitle')}
          aria-label={t('browser.reload')}
        >
          <RefreshCw size={14} />
        </button>
      </div>

      <div className="bf-address-wrap">
        {state.loading && <Loader2 className="bf-spinner" size={13} aria-hidden />}
        <input
          ref={addressRef}
          className="bf-address"
          type="text"
          value={draft}
          placeholder={t('browser.addressPlaceholder')}
          spellCheck={false}
          autoComplete="off"
          aria-label={t('browser.addressPlaceholder')}
          onChange={e => {
            setDraft(e.target.value)
            editingRef.current = true
          }}
          onFocus={() => {
            // 程序自动聚焦不计编辑（见 skipAutoFocusRef 注释）
            if (skipAutoFocusRef.current) {
              skipAutoFocusRef.current = false
              return
            }
            editingRef.current = true
          }}
          onBlur={() => {
            editingRef.current = false
            // 空输入回落到权威 URL（用户没改就直接对齐）
            if (!draft.trim()) setDraft(state.url)
          }}
          onKeyDown={e => {
            if (e.key === 'Enter') {
              e.preventDefault()
              void submitAddress()
            }
          }}
        />
      </div>

      <button
        type="button"
        className="bf-btn"
        onClick={() => void submitAddress()}
        title={t('browser.goTitle')}
      >
        {t('browser.go')}
      </button>

      <div className="bf-toggles">
        <button
          type="button"
          className={recording ? 'bf-toggle is-on' : 'bf-toggle'}
          onClick={() => void toggleRecording()}
          aria-pressed={recording}
          title={
            recording
              ? `${t('browser.recording')}（${t('browser.recordStop')}）`
              : recordCount > 0
                ? `${t('browser.recordTitle')} · ${t('browser.recorded', String(recordCount))}`
                : t('browser.recordTitle')
          }
        >
          <Circle size={12} />
          <span className="bf-toggle-text">{t('browser.record')}</span>
        </button>
        <button
          type="button"
          className={annotating ? 'bf-toggle is-on' : 'bf-toggle'}
          onClick={toggleAnnotating}
          aria-pressed={annotating}
          title={t('browser.annotateTitle')}
        >
          <PenTool size={12} />
          <span className="bf-toggle-text">{t('browser.annotate')}</span>
        </button>
      </div>

      {/* 错误 chip：44px 栏内不做常驻错误条（会撑高壳、挤掉导航），
          截断展示 + title 挂全文，4s 自退 */}
      {error && (
        <div className="bf-error" role="alert" title={error.text}>
          {error.text}
        </div>
      )}

      <div className="bf-tabs" role="group" aria-label={t('browser.tabs')}>
        {tabs.map(info => {
          const own = info.label === label
          return (
            <div key={info.label} className={own ? 'bf-tab is-own' : 'bf-tab'}>
              <button
                type="button"
                className="bf-tab-main"
                onClick={() => handleTabClick(info)}
                title={own ? info.url || info.title : t('browser.focusPending')}
              >
                {info.label === label && state.loading && (
                  <Loader2 className="bf-tab-spinner" size={11} aria-hidden />
                )}
                <span className="bf-tab-label">{info.title || t('browser.untitled')}</span>
              </button>
              <button
                type="button"
                className="bf-tab-close"
                onClick={() => void handleTabClose(info.label)}
                title={t('browser.closeTab')}
                aria-label={`${t('browser.closeTab')}：${info.title || info.label}`}
              >
                <X size={11} />
              </button>
            </div>
          )
        })}
      </div>
    </div>
  )
}

export default BrowserFrame
