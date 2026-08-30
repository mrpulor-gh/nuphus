/**
 * 消息气泡（轻风格）：
 * - header：名称 + 时间
 * - assistant 消息：执行状态行（一行纯文字，工具调用 DOM 样式，动态更新；
 *   可在「动作 / Agent 文本」两种模式间切换，点击打开执行弹窗）
 *   + 最终回复区（椭圆角背景 + Markdown 渲染）
 * - user 消息：向 agent 气泡靠拢（椭圆角背景 + Markdown），名称右侧
 *
 * 轻量原则：执行过程不再内嵌列表，收敛为一行状态行 + 弹窗详情。
 */

import { useEffect, useRef, useState } from 'react'
import {
  Check,
  ChevronRight,
  Copy,
  Loader2,
  MessageSquareText,
  Sparkles,
  Star,
  Wrench,
  X,
} from 'lucide-react'
import type { ChatMessage, TraceItem } from '../store'
import MobileMarkdown from './MobileMarkdown'
import TraceModal from './TraceModal'
import { t } from '../i18n'

interface Props {
  message: ChatMessage
  /** 当前生效 agent 显示名（后端 identity 下发；缺省 Nuphus） */
  assistantName?: string
  /** 会话累计上下文用量（token_usage 事件，传给执行弹窗统计展示） */
  tokenUsage?: { inputTokens: number; outputTokens?: number; cacheHitTokens?: number }
  /** 点评回调（assistant 消息「点评」按钮触发，App 层提交记忆评分） */
  onRateMessage?: (message: ChatMessage) => void
}

function formatTime(ts?: number): string {
  if (!ts) return ''
  const d = new Date(ts)
  const hh = String(d.getHours()).padStart(2, '0')
  const mm = String(d.getMinutes()).padStart(2, '0')
  return `${hh}:${mm}`
}

/**
 * 跨环境复制文本：
 * 1. 优先 Clipboard API（仅 secure context = HTTPS/localhost 可用）
 * 2. HTTP 环境（局域网 / 中继）fallback：隐藏 textarea + document.execCommand('copy')
 * 必须在用户手势同步调用栈内执行（React onClick 内调用满足）。
 */
function copyText(text: string): Promise<boolean> {
  if (navigator.clipboard && window.isSecureContext) {
    return navigator.clipboard
      .writeText(text)
      .then(() => true)
      .catch(() => execCommandCopy(text))
  }
  return Promise.resolve(execCommandCopy(text))
}

function execCommandCopy(text: string): boolean {
  try {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.style.position = 'fixed'
    ta.style.top = '0'
    ta.style.left = '0'
    ta.style.opacity = '0'
    ta.style.pointerEvents = 'none'
    document.body.appendChild(ta)
    ta.focus()
    ta.select()
    ta.setSelectionRange(0, text.length)
    const ok = document.execCommand('copy')
    document.body.removeChild(ta)
    return ok
  } catch {
    return false
  }
}

function senderName(message: ChatMessage, assistantName?: string): string {
  if (message.role === 'user')
    return message.source === 'desktop' ? t('mobile.desktop') : t('mobile.me')
  if (message.role === 'assistant') return assistantName || 'Nuphus'
  return t('mobile.system')
}

/** 从执行过程中提取状态行摘要（工具计数 / 最后工具 / 最后 agent 文本） */
function execSummary(items: TraceItem[]) {
  const tools = items.filter(i => i.kind === 'tool')
  const lastTool = [...items].reverse().find(i => i.kind === 'tool')
  const lastText = [...items].reverse().find(i => i.kind === 'text')
  return { tools, lastTool, lastText }
}

export default function MessageBubble({
  message,
  assistantName,
  tokenUsage,
  onRateMessage,
}: Props) {
  const [traceOpen, setTraceOpen] = useState(false)
  // 复制反馈（短暂显示「已复制」）
  const [copied, setCopied] = useState(false)
  // 图片放大预览（点击缩略图打开全屏 Lightbox）
  const [previewImg, setPreviewImg] = useState<string | null>(null)

  // 执行弹窗自动关闭：点开时消息正在流式（执行中）→ 流式结束（执行完成，
  // streaming 置 false）自动关闭弹窗回到对话；点开时已完成的（历史消息）不自动关。
  const wasStreamingOnOpenRef = useRef(false)
  useEffect(() => {
    if (traceOpen) wasStreamingOnOpenRef.current = !!message.streaming
  }, [traceOpen])
  useEffect(() => {
    if (traceOpen && wasStreamingOnOpenRef.current && !message.streaming) {
      setTraceOpen(false)
    }
  }, [message.streaming, traceOpen])

  if (message.role === 'system') {
    return <div className="mobile-msg-system">{message.content}</div>
  }

  // refine（提炼摘要）：独立分隔条样式——内容完整展示（Leader 自我提炼的决策记忆），
  // 与普通 assistant 回复区分，避免被当作「最后一轮回复」。
  if (message.role === 'refine') {
    return (
      <div className="mobile-msg-refine">
        <div className="mobile-msg-refine-head">
          <Sparkles size={13} aria-hidden="true" />
          <span>{t('sessionDivider.refined')}</span>
        </div>
        {message.content && (
          <div className="mobile-msg-refine-body">
            <MobileMarkdown content={message.content} />
          </div>
        )}
      </div>
    )
  }

  const isUser = message.role === 'user'
  const traceItems = message.traceItems || []
  const hasTrace = traceItems.length > 0
  const hasFinal = !!message.content && message.content.length > 0
  // 共享媒体渲染：图片（点击放大）+ 音频（可播放）——user 与 assistant 消息通用
  const mediaBlock =
    (message.images && message.images.length > 0) || (message.audio && message.audio.length > 0) ? (
      <>
        {message.images && message.images.length > 0 && (
          <div className="mobile-msg-images">
            {message.images.map((img, i) => (
              <img
                key={i}
                src={img}
                alt={`图片 ${i + 1}`}
                className="mobile-msg-image"
                loading="lazy"
                onClick={() => setPreviewImg(img)}
              />
            ))}
          </div>
        )}
        {message.audio && message.audio.length > 0 && (
          <div className="mobile-msg-audios">
            {message.audio.map((src, i) => (
              <audio key={i} src={src} controls preload="none" />
            ))}
          </div>
        )}
      </>
    ) : null
  const { tools, lastTool, lastText } = execSummary(traceItems)
  const streaming = !!message.streaming
  // 执行中（流式输出 / 工具调用尚未完成）：执行栏呼吸动画，视觉提示「还在执行」
  const isRunning = streaming || lastTool?.status === 'running'

  // 状态行动态文字（一行，纯文字）：以显示 agent 文本为主，
  // 没有文本才显示工具调用（工具计数 / 完成态）——自动降级，无手动切换。
  let lineText = ''
  if (lastText) {
    lineText = lastText.text
  } else if (lastTool) {
    lineText = `${lastTool.name} · ${tools.length} ${t('mobile.toolsCount')}`
  } else if (streaming) {
    lineText = t('mobile.thinking')
  } else if (hasTrace) {
    lineText = `${t('mobile.execDone')} · ${tools.length} ${t('mobile.toolsCount')}`
  } else {
    lineText = ''
  }

  return (
    <div
      className={[
        'mobile-msg',
        isUser ? 'mobile-msg-user' : 'mobile-msg-ai',
        message.pending ? 'is-pending' : '',
      ]
        .filter(Boolean)
        .join(' ')}
    >
      {/* header：名称 + 时间（user 名称在右侧，靠拢 agent 结构） */}
      <div className="mobile-msg-header">
        <span className="mobile-msg-name">{senderName(message, assistantName)}</span>
        {message.timestamp && (
          <span className="mobile-msg-time">{formatTime(message.timestamp)}</span>
        )}
      </div>

      {/* user：椭圆角气泡 + Markdown（向 agent 最终回复靠拢） */}
      {isUser && (
        <div className="mobile-msg-bubble">
          {mediaBlock}
          {message.content && <MobileMarkdown content={message.content} />}
          {message.streaming && <span className="mobile-msg-cursor" aria-hidden="true" />}
        </div>
      )}

      {/* assistant：执行状态胶囊（一行，文本优先）+ 最终回复区 */}
      {!isUser && (
        <>
          {hasTrace && lineText && (
            <div className={`mobile-exec-line${isRunning ? ' is-running' : ''}`}>
              <button
                type="button"
                className="mobile-exec-line-main"
                onClick={() => setTraceOpen(true)}
                aria-label="查看完整执行过程"
              >
                <span className="mobile-exec-line-icon">
                  {lastText ? (
                    <MessageSquareText size={12} aria-hidden="true" />
                  ) : lastTool ? (
                    streaming ? (
                      <Loader2 size={12} className="mobile-spin" aria-hidden="true" />
                    ) : lastTool.status === 'ok' ? (
                      <Check size={12} className="is-ok" aria-hidden="true" />
                    ) : lastTool.status === 'fail' ? (
                      <X size={12} className="is-fail" aria-hidden="true" />
                    ) : (
                      <Wrench size={12} aria-hidden="true" />
                    )
                  ) : (
                    <Wrench size={12} aria-hidden="true" />
                  )}
                </span>
                <span className="mobile-exec-line-text">{lineText}</span>
                {tools.length > 0 && <span className="mobile-exec-line-count">{tools.length}</span>}
                <ChevronRight size={13} className="mobile-exec-line-chevron" aria-hidden="true" />
              </button>
            </div>
          )}
          {/* 媒体（agent 产图 / 语音）：独立于最终回复区，无 content 也可显示 */}
          {mediaBlock}
          {hasFinal && (
            <div className="mobile-msg-final">
              <MobileMarkdown content={message.content} />
            </div>
          )}
          {!isUser && hasFinal && !streaming && (
            <div className="mobile-msg-actions">
              <button
                type="button"
                className="mobile-msg-action"
                onClick={() => {
                  void copyText(message.content).then(ok => {
                    if (ok) {
                      setCopied(true)
                      setTimeout(() => setCopied(false), 1500)
                    }
                  })
                }}
                title={t('mobile.copyContent')}
              >
                {copied ? (
                  <Check size={13} aria-hidden="true" />
                ) : (
                  <Copy size={13} aria-hidden="true" />
                )}
                <span>{copied ? t('mobile.copied') : t('mobile.copy')}</span>
              </button>
              {onRateMessage && (
                <button
                  type="button"
                  className="mobile-msg-action"
                  onClick={() => onRateMessage(message)}
                  title={t('mobile.rateExecution')}
                >
                  <Star size={13} aria-hidden="true" />
                  <span>{t('mobile.rate')}</span>
                </button>
              )}
            </div>
          )}
          {streaming && !hasTrace && (
            <div className="mobile-msg-bubble mobile-msg-bubble-ai">
              <span className="mobile-msg-cursor" aria-hidden="true" />
            </div>
          )}
        </>
      )}

      {traceOpen && (
        <TraceModal
          traceItems={traceItems}
          tokenUsage={tokenUsage}
          onClose={() => setTraceOpen(false)}
        />
      )}

      {/* 图片放大预览：全屏 overlay，点击关闭 */}
      {previewImg && (
        <div
          className="mobile-lightbox"
          role="dialog"
          aria-label="图片预览"
          onClick={() => setPreviewImg(null)}
        >
          <img src={previewImg} alt="图片预览" />
          <button
            type="button"
            className="mobile-lightbox-close"
            onClick={() => setPreviewImg(null)}
            aria-label="关闭预览"
          >
            <X size={20} aria-hidden="true" />
          </button>
        </div>
      )}
    </div>
  )
}