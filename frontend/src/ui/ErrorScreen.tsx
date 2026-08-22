import { useState } from 'react'
import { Button } from './Button'
import '../styles/error.css'

export type ErrorKind =
  'backend_unavailable' | 'config_corrupted' | 'api_key_invalid' | 'port_in_use' | 'unknown'

interface ErrorInfo {
  kind: ErrorKind
  message: string
  detail?: string
}

interface ErrorScreenProps {
  error: ErrorInfo
  onRetry?: () => void
  onOpenSettings?: () => void
  onExit?: () => void
}

const ERROR_META: Record<
  ErrorKind,
  {
    title: string
    desc: string
    icon: 'plug' | 'broken' | 'key' | 'port' | 'alert'
    actions: Array<'retry' | 'settings' | 'exit'>
  }
> = {
  backend_unavailable: {
    title: '后端服务未响应',
    desc: '无法连接到 Nuphus 核心服务，我的「大脑」暂时离线了。请检查进程是否正常运行。',
    icon: 'plug',
    actions: ['retry', 'exit'],
  },
  config_corrupted: {
    title: '配置文件损坏',
    desc: '我的记忆文件似乎有些混乱，无法正常解析。可以尝试重置为默认配置让我重新整理。',
    icon: 'broken',
    actions: ['retry', 'settings', 'exit'],
  },
  api_key_invalid: {
    title: '模型 API 密钥无效',
    desc: '当前配置的 API 密钥无法通过验证，我无法与模型建立连接。请在设置中更新密钥。',
    icon: 'key',
    actions: ['settings', 'exit'],
  },
  port_in_use: {
    title: '端口被占用',
    desc: 'Nuphus 所需的通信端口已被其他程序占用，我找不到我的「座位」了。',
    icon: 'port',
    actions: ['retry', 'settings', 'exit'],
  },
  unknown: {
    title: '启动异常',
    desc: '初始化过程中遇到了意料之外的问题，连我也不知道发生了什么。',
    icon: 'alert',
    actions: ['retry', 'exit'],
  },
}

function ErrorIcon({ kind, size = 40 }: { kind: ErrorKind; size?: number }) {
  const s = size

  switch (kind) {
    case 'api_key_invalid':
      return (
        <svg
          width={s}
          height={s}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path
            d="M12 2a7 7 0 0 0-7 7c0 2.38 1.19 4.47 3 5.74V17a1 1 0 0 0 1 1h6a1 1 0 0 0 1-1v-2.26c1.81-1.27 3-3.36 3-5.74a7 7 0 0 0-7-7z"
            opacity=".4"
          />
          <circle cx="12" cy="9" r="2" />
          <line x1="3" y1="3" x2="21" y2="21" />
        </svg>
      )

    case 'backend_unavailable':
      return (
        <svg
          width={s}
          height={s}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M12 22a7 7 0 0 0 7-7c0-2-1-3.9-3-5.5" opacity=".3" />
          <path d="M5 15a7 7 0 0 0 7 7" opacity=".5" />
          <path d="M5 15a7 7 0 0 1 7-7" opacity=".5" />
          <line x1="3" y1="3" x2="21" y2="21" />
          <circle cx="12" cy="12" r="2" opacity=".4" />
        </svg>
      )

    case 'config_corrupted':
      return (
        <svg
          width={s}
          height={s}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" opacity=".3" />
          <polyline points="14 2 14 8 20 8" opacity=".3" />
          <line x1="9" y1="13" x2="15" y2="13" />
          <line x1="9" y1="17" x2="13" y2="17" />
          <line x1="3" y1="3" x2="21" y2="21" />
        </svg>
      )

    case 'port_in_use':
      return (
        <svg
          width={s}
          height={s}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <rect x="2" y="2" width="20" height="8" rx="2" ry="2" opacity=".3" />
          <rect x="2" y="14" width="20" height="8" rx="2" ry="2" opacity=".3" />
          <line x1="6" y1="6" x2="6.01" y2="6" />
          <line x1="6" y1="18" x2="6.01" y2="18" />
          <line x1="3" y1="10" x2="3" y2="14" />
          <line x1="21" y1="10" x2="21" y2="14" />
          <line x1="3" y1="3" x2="21" y2="21" />
        </svg>
      )

    default:
      return (
        <svg
          width={s}
          height={s}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <circle cx="12" cy="12" r="10" opacity=".4" />
          <line x1="15" y1="9" x2="9" y2="15" />
          <line x1="9" y1="9" x2="15" y2="15" />
        </svg>
      )
  }
}

export function ErrorScreen({ error, onRetry, onOpenSettings, onExit }: ErrorScreenProps) {
  const [showDetail, setShowDetail] = useState(false)
  const meta = ERROR_META[error.kind] || ERROR_META.unknown

  return (
    <div className="error-screen" data-error-kind={error.kind}>
      <div className="error-content">
        <div className="error-icon-wrap" data-error-kind={error.kind}>
          <ErrorIcon kind={error.kind} size={40} />
        </div>

        <h1 className="error-title">{meta.title}</h1>
        <p className="error-desc">{meta.desc}</p>

        <div className="error-actions">
          {meta.actions.includes('retry') && onRetry && (
            <Button variant="error-primary" onClick={onRetry}>
              重试
            </Button>
          )}
          {meta.actions.includes('settings') && onOpenSettings && (
            <Button variant="error-secondary" onClick={onOpenSettings}>
              打开设置
            </Button>
          )}
          {meta.actions.includes('exit') && onExit && (
            <Button variant="error-ghost" onClick={onExit}>
              退出
            </Button>
          )}
        </div>

        {error.detail && (
          <div className="error-detail">
            <button className="error-detail-toggle" onClick={() => setShowDetail(!showDetail)}>
              {showDetail ? '收起详情' : '查看详情'}
            </button>
            {showDetail && <pre className="error-detail-code">{error.detail}</pre>}
          </div>
        )}
      </div>
    </div>
  )
}
