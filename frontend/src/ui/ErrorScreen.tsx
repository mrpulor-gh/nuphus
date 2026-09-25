import { useState } from 'react'
import { Button } from './Button'
import { IconKeyRound, IconServerOff, IconFileX, IconServer, IconCircleX } from './Icons'
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
      return <IconKeyRound size={s} strokeWidth={1.5} />

    case 'backend_unavailable':
      return <IconServerOff size={s} strokeWidth={1.5} />

    case 'config_corrupted':
      return <IconFileX size={s} strokeWidth={1.5} />

    case 'port_in_use':
      return <IconServer size={s} strokeWidth={1.5} />

    default:
      return <IconCircleX size={s} strokeWidth={1.5} />
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
