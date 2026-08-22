import React, { Component, type ReactNode } from 'react'
import { ErrorXIcon } from './Icons'
import { Button } from './Button'
import { invoke } from '../core/bridge'
import '../styles/error.css'
import zh from '../locales/zh'
import en from '../locales/en'

function eb(key: string): string {
  const lang =
    (typeof window !== 'undefined' ? localStorage.getItem('nuphus_language') : null) || 'zh'
  const pack = lang === 'en' ? en : zh
  return (pack as Record<string, string>)[key] || (zh as Record<string, string>)[key] || key
}

interface Props {
  children: ReactNode
  onExit?: () => void
}

interface State {
  hasError: boolean
  error: Error | null
  showDetail: boolean
}

export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props)
    this.state = { hasError: false, error: null, showDetail: false }
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error, showDetail: false }
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('[ErrorBoundary] Caught error:', error)
    console.error('[ErrorBoundary] Component stack:', info.componentStack)
  }

  handleReload = () => {
    window.location.reload()
  }

  handleExportLog = async () => {
    const { error } = this.state
    const log = [
      `${eb('error.time')}: ${new Date().toISOString()}`,
      `${eb('error.userAgent')}: ${navigator.userAgent}`,
      `${eb('error.error')}: ${error?.name}: ${error?.message}`,
      `${eb('error.stack')}:\n${error?.stack || eb('error.na')}`,
    ].join('\n\n')

    try {
      const result = await invoke<string>('export_error_log', { content: log })
      if (result && typeof result === 'string' && result.length > 10) {
        alert(`${eb('error.exportSuccess')}\n${result}`)
      } else {
        alert(eb('error.exportFail'))
      }
    } catch {
      alert(eb('error.exportFail'))
    }
  }

  toggleDetail = () => {
    this.setState(prev => ({ showDetail: !prev.showDetail }))
  }

  render() {
    if (!this.state.hasError) {
      return this.props.children
    }

    return (
      <div className="error-screen error-screen--boundary">
        <div className="error-content">
          <div className="error-icon-wrap">
            <ErrorXIcon />
          </div>

          <h1 className="error-title">{eb('error.title')}</h1>
          <p className="error-desc">{eb('error.desc')}</p>

          <div className="error-actions">
            <Button variant="error-primary" onClick={this.handleReload}>
              {eb('error.reload')}
            </Button>
            <Button variant="error-secondary" onClick={this.handleExportLog}>
              {eb('error.exportLog')}
            </Button>
            {this.props.onExit && (
              <Button variant="error-ghost" onClick={this.props.onExit}>
                {eb('error.exit')}
              </Button>
            )}
          </div>

          <div className="error-detail">
            <button className="error-detail-toggle" onClick={this.toggleDetail}>
              {this.state.showDetail ? eb('error.hideDetail') : eb('error.showDetail')}
            </button>
            {this.state.showDetail && (
              <pre className="error-detail-code">
                {this.state.error?.name}: {this.state.error?.message}
                {'\n'}
                {this.state.error?.stack}
              </pre>
            )}
          </div>
        </div>
      </div>
    )
  }
}
