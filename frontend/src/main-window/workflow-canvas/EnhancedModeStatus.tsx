/**
 * EnhancedModeStatus.tsx — 增强模式**只读状态行**（输入栏模型 hover 弹窗内）
 *
 * 大王 2026-10-09 定稿：workflow 模式下，把原输入框 chip 上的增强开关
 * ①移进模型 hover 弹窗（推理强度）的下方，②取消开关控制，只显示当前状态，
 * ③点击跳「设置 → 模型 → 增强判断模型」。
 *
 * 为什么这里**没有**开关：
 * 后端 `set_workflow_enhanced_mode` 的唯一前端入口仍是 EnhancedModeToggle
 * （含未配置确认弹窗 + 权限回退），本组件刻意不 import setWorkflowEnhancedMode。
 * 一旦在这里也放开关，同一能力会出现两处入口、两套确认流程，且两者都常驻
 * 会互相打架 —— 故本组件只负责「如实播报状态 + 给出配置入口」。
 *
 * 状态文案复用 EnhancedModeToggle 导出的 statusLabel / INITIAL_STATE（唯一实现点），
 * 事件订阅沿用同一套 requestId 防竞态模式。
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import { IconSparkles } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import { getWorkflowEnhancedMode, type WorkflowEnhancedMode } from '../lib/api'
import {
  WORKFLOW_ENHANCED_MODE_CHANGED_EVENT,
  WORKFLOW_ENHANCED_MODE_REFRESH_EVENT,
} from './enhancedModeEvents'
import { INITIAL_STATE, statusLabel } from './EnhancedModeToggle'
import './EnhancedModeStatus.css'

export function EnhancedModeStatus() {
  const { lang } = useLanguage()
  const ui = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const [state, setState] = useState<WorkflowEnhancedMode>(INITIAL_STATE)
  const [loading, setLoading] = useState(true)
  const [loadFailed, setLoadFailed] = useState(false)
  const requestId = useRef(0)

  const load = useCallback(async () => {
    const id = ++requestId.current
    setLoading(true)
    try {
      const current = await getWorkflowEnhancedMode()
      // 竞态闸门：只有最后一次请求的结果可以落地（focus 刷新与挂载首查常交错）
      if (id !== requestId.current) return
      if (!current) throw new Error('后端未返回增强模式状态')
      setState(current)
      setLoadFailed(false)
    } catch {
      if (id !== requestId.current) return
      setState(INITIAL_STATE)
      setLoadFailed(true)
    } finally {
      if (id === requestId.current) setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    const sync = (event: Event) => {
      const next = (event as CustomEvent<WorkflowEnhancedMode>).detail
      if (!next) return
      ++requestId.current
      setState(next)
      setLoadFailed(false)
      setLoading(false)
    }
    const refresh = () => void load()
    const refreshWhenVisible = () => {
      if (document.visibilityState === 'visible') refresh()
    }
    window.addEventListener(WORKFLOW_ENHANCED_MODE_CHANGED_EVENT, sync)
    window.addEventListener(WORKFLOW_ENHANCED_MODE_REFRESH_EVENT, refresh)
    window.addEventListener('focus', refresh)
    document.addEventListener('visibilitychange', refreshWhenVisible)
    return () => {
      ++requestId.current
      window.removeEventListener(WORKFLOW_ENHANCED_MODE_CHANGED_EVENT, sync)
      window.removeEventListener(WORKFLOW_ENHANCED_MODE_REFRESH_EVENT, refresh)
      window.removeEventListener('focus', refresh)
      document.removeEventListener('visibilitychange', refreshWhenVisible)
    }
  }, [load])

  const openConfiguration = () => {
    // 复用既有跳转通道：App.tsx 监听 nuphus-nav-models → 设置 → 模型 → 增强判断模型
    window.dispatchEvent(
      new CustomEvent('nuphus-nav-models', {
        detail: { view: 'jev' },
      }),
    )
  }

  const label = loading ? ui('读取中', 'Loading') : statusLabel(state, loadFailed, ui)

  return (
    <button
      type="button"
      role="menuitem"
      className={`wfc-enhanced-status-item${state.enabled ? ' is-on' : ''}`}
      aria-label={`${ui('增强模式，', 'Enhanced mode, ')}${label}`}
      title={ui('点击前往增强判断模型设置', 'Open enhanced decision model settings')}
      onClick={openConfiguration}
    >
      <IconSparkles size={12} aria-hidden />
      <span className="wfc-enhanced-status-item-label">{ui('增强', 'Enhanced')}</span>
      <span className="wfc-enhanced-status-item-sep" aria-hidden>
        ·
      </span>
      <span className="wfc-enhanced-status-item-state">{label}</span>
    </button>
  )
}
