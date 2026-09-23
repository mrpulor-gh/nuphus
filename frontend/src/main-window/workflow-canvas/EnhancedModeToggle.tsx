import { useCallback, useEffect, useState } from 'react'
import { IconSparkles } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { CompactModal } from '../layout/CompactModal'
import './EnhancedModeToggle.css'
import {
  getWorkflowEnhancedMode,
  setWorkflowEnhancedMode,
  type WorkflowEnhancedMode,
} from '../lib/api'

type EnhancedModeToggleProps = {
  disabled?: boolean
  onNotice?: (message: string) => void
  compact?: boolean
}

const ENHANCED_MODE_CHANGED_EVENT = 'nuphus:workflow-enhanced-mode-changed'

const INITIAL_STATE: WorkflowEnhancedMode = {
  enabled: false,
  configured: false,
}

function statusLabel(state: WorkflowEnhancedMode, loadFailed: boolean): string {
  const status = state.status?.trim().toLowerCase().replace(/-/g, '_') ?? ''
  if (loadFailed || ['unavailable', 'service_unavailable', 'offline', 'error'].includes(status)) {
    return '服务不可用'
  }
  if (
    state.enabled &&
    (!state.configured || ['primary_fallback', 'compatibility'].includes(status))
  ) {
    return '主模型'
  }
  if (!state.configured || status === 'unconfigured') return '未配置'
  if (['preview', 'previewing'].includes(status)) return '预览'
  if (['needs_approval', 'needs_incremental_authorization'].includes(status)) {
    return '需要增量授权'
  }
  if (['running', 'executing'].includes(status)) return '执行中'
  if (['stopped', 'cancelled'].includes(status)) return '已停止'
  return state.enabled ? '可用' : '已关闭'
}

export function EnhancedModeToggle({
  disabled = false,
  onNotice,
  compact = false,
}: EnhancedModeToggleProps) {
  const [state, setState] = useState<WorkflowEnhancedMode>(INITIAL_STATE)
  const [loading, setLoading] = useState(true)
  const [updating, setUpdating] = useState(false)
  const [loadFailed, setLoadFailed] = useState(false)
  const [showConfigurationWarning, setShowConfigurationWarning] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const current = await getWorkflowEnhancedMode()
      if (!current) throw new Error('后端未返回增强模式状态')
      setState(current)
      setLoadFailed(false)
    } catch {
      setState(INITIAL_STATE)
      setLoadFailed(true)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    const sync = (event: Event) => {
      const next = (event as CustomEvent<WorkflowEnhancedMode>).detail
      if (!next) return
      setState(next)
      setLoadFailed(false)
      setLoading(false)
    }
    window.addEventListener(ENHANCED_MODE_CHANGED_EVENT, sync)
    return () => window.removeEventListener(ENHANCED_MODE_CHANGED_EVENT, sync)
  }, [])

  const updateMode = async (enabled: boolean) => {
    setUpdating(true)
    try {
      const next = await setWorkflowEnhancedMode(enabled)
      if (!next) throw new Error('后端未返回增强模式状态')
      setState(next)
      setLoadFailed(false)
      window.dispatchEvent(
        new CustomEvent<WorkflowEnhancedMode>(ENHANCED_MODE_CHANGED_EVENT, { detail: next }),
      )
    } catch (error) {
      setLoadFailed(true)
      onNotice?.(`切换增强模式失败：${String(error)}`)
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

  const label = loading ? '读取中' : statusLabel(state, loadFailed)
  const locked = disabled || loading || updating

  return (
    <>
      <button
        type="button"
        className={`wfc-enhanced-toggle${state.enabled ? ' is-on' : ''}${compact ? ' is-compact' : ''}`}
        aria-pressed={state.enabled}
        aria-label={`增强模式，${label}`}
        disabled={locked}
        onClick={() => void toggle()}
        title={disabled ? '运行中或只读画布不可切换增强模式' : '从本地候选动作中进行结构化判断'}
      >
        <IconSparkles size={13} />
        <span className="wfc-enhanced-label">{compact ? '增强' : '增强模式'}</span>
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
        title="未配置增强判断模型"
        icon={<IconSparkles size={14} />}
        size="sm"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setShowConfigurationWarning(false)}>
              取消
            </Button>
            <div className="wfc-enhanced-confirm-actions">
              <Button size="sm" onClick={openConfiguration}>
                前往配置
              </Button>
              <Button variant="primary" size="sm" onClick={enableWithoutConfiguration}>
                仍然开启
              </Button>
            </div>
          </>
        }
      >
        <p className="wfc-enhanced-confirm-copy">
          增强模式需要配置增强判断模型才能获得完整效果。当前尚未配置；仍然开启时将由主模型完成候选判断，效果可能较差，并会消耗更多
          Token。
        </p>
      </CompactModal>
    </>
  )
}
