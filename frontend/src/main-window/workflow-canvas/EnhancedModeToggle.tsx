import { useCallback, useEffect, useRef, useState } from 'react'
import { IconSparkles } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { CompactModal } from '../layout/CompactModal'
import { useLanguage } from '../../locales'
import './EnhancedModeToggle.css'
import {
  getWorkflowEnhancedMode,
  setWorkflowEnhancedMode,
  type WorkflowEnhancedMode,
} from '../lib/api'
import {
  publishWorkflowEnhancedMode,
  WORKFLOW_ENHANCED_MODE_CHANGED_EVENT,
  WORKFLOW_ENHANCED_MODE_REFRESH_EVENT,
} from './enhancedModeEvents'

type EnhancedModeToggleProps = {
  disabled?: boolean
  onNotice?: (message: string) => void
  compact?: boolean
}

const INITIAL_STATE: WorkflowEnhancedMode = {
  enabled: false,
  configured: false,
}

function statusLabel(
  state: WorkflowEnhancedMode,
  loadFailed: boolean,
  ui: (zh: string, en: string) => string,
): string {
  const status = state.status?.trim().toLowerCase().replace(/-/g, '_') ?? ''
  if (loadFailed || ['unavailable', 'service_unavailable', 'offline', 'error'].includes(status)) {
    return ui('服务不可用', 'Unavailable')
  }
  if (status === 'unsupported_platform') return ui('当前平台未支持', 'Unsupported platform')
  if (status === 'needs_accessibility') return ui('需辅助功能权限', 'Accessibility required')
  if (
    state.enabled &&
    (!state.configured || ['primary_fallback', 'compatibility'].includes(status))
  ) {
    return ui('主模型', 'Primary model')
  }
  if (!state.configured || status === 'unconfigured') return ui('未配置', 'Not configured')
  if (['preview', 'previewing'].includes(status)) return ui('预览', 'Preview')
  if (['needs_approval', 'needs_incremental_authorization'].includes(status)) {
    return ui('需要增量授权', 'Additional approval needed')
  }
  if (['running', 'executing'].includes(status)) return ui('执行中', 'Running')
  if (['stopped', 'cancelled'].includes(status)) return ui('已停止', 'Stopped')
  return state.enabled ? ui('可用', 'Ready') : ui('已关闭', 'Off')
}

export function EnhancedModeToggle({
  disabled = false,
  onNotice,
  compact = false,
}: EnhancedModeToggleProps) {
  const { lang } = useLanguage()
  const ui = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const [state, setState] = useState<WorkflowEnhancedMode>(INITIAL_STATE)
  const [loading, setLoading] = useState(true)
  const [updating, setUpdating] = useState(false)
  const [loadFailed, setLoadFailed] = useState(false)
  const [showConfigurationWarning, setShowConfigurationWarning] = useState(false)
  const requestId = useRef(0)

  const load = useCallback(async () => {
    const id = ++requestId.current
    setLoading(true)
    try {
      const current = await getWorkflowEnhancedMode()
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

  const updateMode = async (enabled: boolean) => {
    setUpdating(true)
    try {
      const next = await setWorkflowEnhancedMode(enabled)
      if (!next) throw new Error(ui('后端未返回增强模式状态', 'No enhanced mode status returned'))
      setState(next)
      setLoadFailed(false)
      publishWorkflowEnhancedMode(next)
    } catch (error) {
      setLoadFailed(true)
      onNotice?.(`${ui('切换增强模式失败：', 'Could not change enhanced mode: ')}${String(error)}`)
    } finally {
      setUpdating(false)
    }
  }

  const toggle = async () => {
    if (disabled || loading || updating) return
    if (!state.enabled && !state.configured) {
      setShowConfigurationWarning(true)
      return
    }
    await updateMode(!state.enabled)
  }

  const openConfiguration = () => {
    setShowConfigurationWarning(false)
    window.dispatchEvent(
      new CustomEvent('nuphus-nav-models', {
        detail: { view: 'jev' },
      }),
    )
  }

  const enableWithoutConfiguration = () => {
    setShowConfigurationWarning(false)
    void updateMode(true)
  }

  const label = loading ? ui('读取中', 'Loading') : statusLabel(state, loadFailed, ui)
  const locked = disabled || loading || updating

  return (
    <>
      <button
        type="button"
        className={`wfc-enhanced-toggle${state.enabled ? ' is-on' : ''}${compact ? ' is-compact' : ''}`}
        aria-pressed={state.enabled}
        aria-label={`${ui('增强模式，', 'Enhanced mode, ')}${label}`}
        disabled={locked}
        onClick={() => void toggle()}
        title={
          disabled
            ? ui(
                '运行中或只读画布不可切换增强模式',
                'Enhanced mode cannot be changed while running or on a read-only canvas',
              )
            : state.status === 'needs_accessibility'
              ? ui(
                  '请在 macOS 系统设置中允许 Nuphus 使用辅助功能，返回后自动刷新',
                  'Allow Nuphus in macOS Accessibility settings. Status refreshes when you return.',
                )
              : state.status === 'unsupported_platform'
                ? ui(
                    '当前平台尚未支持桌面语义动作，增强模式偏好已保留',
                    'Desktop semantic actions are not supported on this platform. Your enhanced mode preference is retained.',
                  )
                : ui(
                    '从本地候选动作中进行结构化判断',
                    'Choose from structured local action candidates',
                  )
        }
      >
        <IconSparkles size={13} />
        <span className="wfc-enhanced-label">
          {compact ? ui('增强', 'Enhanced') : ui('增强模式', 'Enhanced mode')}
        </span>
        <span className="wfc-enhanced-switch" aria-hidden="true">
          <span />
        </span>
        <span className={`wfc-enhanced-status is-${loadFailed ? 'error' : state.status || 'idle'}`}>
          {label}
        </span>
      </button>

      <CompactModal
        open={showConfigurationWarning}
        onClose={() => setShowConfigurationWarning(false)}
        title={ui('未配置增强判断模型', 'Enhanced decision model not configured')}
        icon={<IconSparkles size={14} />}
        size="sm"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setShowConfigurationWarning(false)}>
              {ui('取消', 'Cancel')}
            </Button>
            <div className="wfc-enhanced-confirm-actions">
              <Button size="sm" onClick={openConfiguration}>
                {ui('前往配置', 'Configure model')}
              </Button>
              <Button variant="primary" size="sm" onClick={enableWithoutConfiguration}>
                {ui('仍然开启', 'Enable anyway')}
              </Button>
            </div>
          </>
        }
      >
        <p className="wfc-enhanced-confirm-copy">
          {ui(
            '增强模式需要配置增强判断模型才能获得完整效果。当前尚未配置；仍然开启时将由主模型完成候选判断，效果可能较差，并会消耗更多 Token。',
            'Configure an enhanced decision model for the full experience. If you enable this mode without one, the primary model will choose action candidates. Results may be less reliable and token usage may be higher.',
          )}
        </p>
      </CompactModal>
    </>
  )
}
