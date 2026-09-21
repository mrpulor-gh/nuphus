import { useCallback, useEffect, useState } from 'react'
import { IconSparkles } from '../../ui/Icons'
import {
  getWorkflowEnhancedMode,
  setWorkflowEnhancedMode,
  type WorkflowEnhancedMode,
} from '../lib/api'

type EnhancedModeToggleProps = {
  disabled?: boolean
  onNotice?: (message: string) => void
}

const INITIAL_STATE: WorkflowEnhancedMode = {
  enabled: false,
  configured: false,
}

function statusLabel(state: WorkflowEnhancedMode, loadFailed: boolean): string {
  const status = state.status?.trim().toLowerCase().replace(/-/g, '_') ?? ''
  if (loadFailed || ['unavailable', 'service_unavailable', 'offline', 'error'].includes(status)) {
    return '服务不可用'
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

export function EnhancedModeToggle({ disabled = false, onNotice }: EnhancedModeToggleProps) {
  const [state, setState] = useState<WorkflowEnhancedMode>(INITIAL_STATE)
  const [loading, setLoading] = useState(true)
  const [updating, setUpdating] = useState(false)
  const [loadFailed, setLoadFailed] = useState(false)

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

  const toggle = async () => {
    if (disabled || loading || updating) return
    if (!state.enabled && !state.configured) {
      onNotice?.('Jev 尚未配置，请先前往「模型设置 → Jev 增强判断」填写并测试连接。')
      return
    }
    setUpdating(true)
    try {
      const next = await setWorkflowEnhancedMode(!state.enabled)
      if (!next) throw new Error('后端未返回增强模式状态')
      setState(next)
      setLoadFailed(false)
    } catch (error) {
      setLoadFailed(true)
      onNotice?.(`切换增强模式失败：${String(error)}`)
    } finally {
      setUpdating(false)
    }
  }

  const label = loading ? '读取中' : statusLabel(state, loadFailed)
  const locked = disabled || loading || updating

  return (
    <button
      type="button"
      className={`wfc-enhanced-toggle${state.enabled ? ' is-on' : ''}`}
      aria-pressed={state.enabled}
      aria-label={`增强模式，${label}`}
      disabled={locked}
      onClick={() => void toggle()}
      title={
        disabled
          ? '运行中或只读画布不可切换增强模式'
          : '使用 Jev 从本地候选动作中做结构化选择；鼠标与视觉回退仍可用'
      }
    >
      <IconSparkles size={13} />
      <span>增强模式</span>
      <span className="wfc-enhanced-switch" aria-hidden="true">
        <span />
      </span>
      <span className={`wfc-enhanced-status is-${loadFailed ? 'error' : state.status || 'idle'}`}>
        {label}
      </span>
    </button>
  )
}
