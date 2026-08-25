/**
 * MobileErrorBoundary — 移动端渲染错误兜底。
 *
 * 背景（2026-08-26 实测）：中继公网路径加载大量历史消息（180+ 条）时，
 * 低端 WebView 一次性渲染可能崩溃（无 ErrorBoundary → React 整树卸载 → 白屏）。
 * 本边界兜住渲染期异常：显示可见错误卡片 + 重试按钮，白屏变可恢复提示。
 */
import { Component, type ReactNode } from 'react'
import { t } from '../i18n'

interface Props {
  children: ReactNode
  /** 重试动作（默认整页刷新） */
  onRetry?: () => void
}

interface State {
  error: string | null
}

export default class MobileErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(err: unknown): State {
    return { error: err instanceof Error ? err.message : String(err) }
  }

  componentDidCatch(err: unknown) {
    // 尽力留痕：localStorage 最近一条渲染错误（真机无 console 可看，重启后排查用）
    try {
      localStorage.setItem('nuphus_mobile_last_error', String(err))
    } catch {
      /* ignore */
    }
  }

  private handleRetry = () => {
    if (this.props.onRetry) {
      this.props.onRetry()
    } else {
      window.location.reload()
    }
  }

  render() {
    if (!this.state.error) return this.props.children
    return (
      <div className="mobile-error-boundary" role="alert">
        <div className="mobile-error-boundary-card">
          <div className="mobile-error-boundary-title">{t('mobile.renderErrorTitle')}</div>
          <div className="mobile-error-boundary-desc">{t('mobile.renderErrorDesc')}</div>
          <button type="button" className="mobile-error-boundary-btn" onClick={this.handleRetry}>
            {t('mobile.retry')}
          </button>
        </div>
      </div>
    )
  }
}
