/**
 * 消息流：气泡列表 + 空状态
 * 自动滚动到底（新消息/流式输出时），用户上翻时暂停跟随
 * 执行状态由消息内状态行承载（轻量一行），不再有独立活动卡
 */

import { useEffect, useRef } from 'react'
import { Loader2 } from 'lucide-react'
import type { ChatMessage, ActivityState } from '../store'
import MessageBubble from './MessageBubble'
import { t } from '../i18n'

interface Props {
  messages: ChatMessage[]
  activity: ActivityState
  assistantName?: string
  /** 会话累计上下文用量（token_usage 事件，下传执行弹窗统计展示） */
  tokenUsage?: { inputTokens: number; outputTokens?: number; cacheHitTokens?: number }
  /** 点评回调（assistant 消息「点评」按钮触发，App 层提交记忆评分） */
  onRateMessage?: (message: ChatMessage) => void
}

export default function MessageList({
  messages,
  activity,
  assistantName,
  tokenUsage,
  onRateMessage,
}: Props) {
  const listRef = useRef<HTMLDivElement>(null)
  const followRef = useRef(true)

  // 用户上翻则暂停自动跟随；回滚到底部附近恢复跟随
  const handleScroll = () => {
    const el = listRef.current
    if (!el) return
    followRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80
  }

  useEffect(() => {
    const el = listRef.current
    if (el && followRef.current) {
      el.scrollTop = el.scrollHeight
    }
  }, [messages, activity])

  if (messages.length === 0 && !activity.running) {
    return (
      <div className="mobile-empty" ref={listRef}>
        <svg
          className="mobile-empty-logo"
          viewBox="0 0 256 256"
          fill="none"
          role="img"
          aria-label="Nuphus"
        >
          <g stroke="currentColor" strokeWidth={24} strokeLinecap="round" fill="none">
            <path d="M64 20 H192 A44 44 0 0 1 236 64 V156" />
            <path d="M200 236 H64 A44 44 0 0 1 20 192 V64 A44 44 0 0 1 64 20" />
            <path d="M80 180 L80 76 M176 180 L176 76" />
          </g>
        </svg>
        <p className="mobile-empty-title">与桌面端同一对话</p>
        <p className="mobile-empty-sub">手机发的消息会在桌面同步执行</p>
      </div>
    )
  }

  return (
    <div className="mobile-messages" ref={listRef} onScroll={handleScroll}>
      {messages.map(m => (
        <MessageBubble
          key={m.id}
          message={m}
          assistantName={assistantName}
          tokenUsage={tokenUsage}
          onRateMessage={onRateMessage}
        />
      ))}
      {/* 执行刚启动、尚无 assistant 消息时的轻量状态行（短暂过渡） */}
      {activity.running && messages.length === 0 && (
        <div className="mobile-exec-line mobile-exec-line--global">
          <span className="mobile-exec-line-icon">
            <Loader2 size={12} className="mobile-spin" aria-hidden="true" />
          </span>
          <span className="mobile-exec-line-text">{activity.goal || t('mobile.executing')}</span>
        </div>
      )}
    </div>
  )
}
