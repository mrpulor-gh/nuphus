import { useState, useCallback, useEffect, useRef } from 'react'
import { IconSparkles, IconChevronDown } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import MarkdownContent from './MarkdownContent'
import '../../styles/session-divider.css'

export interface SessionDividerProps {
  summary: string
  messageCount: number
  sessionId: string
  streamingContent?: string | null
}

function computeRatio(original: number): string | null {
  if (original <= 1) return null
  return (((original - 1) / original) * 100).toFixed(1)
}

export function SessionDivider({
  summary,
  messageCount,
  sessionId,
  streamingContent,
}: SessionDividerProps) {
  const { t } = useLanguage()
  const isStreaming = streamingContent != null
  const displayContent = isStreaming ? streamingContent || '' : summary
  const hasBody = !!displayContent
  const [expanded, setExpanded] = useState(hasBody)
  const bodyRef = useRef<HTMLDivElement>(null)
  const ratio = computeRatio(messageCount)

  useEffect(() => {
    if (!isStreaming) return
    setExpanded(true)
  }, [isStreaming])

  // 注：流式滚动跟随由外层消息列表承担（ChatPanel 监听 messages 滚底），
  // 本容器 expanded/streaming 均为 max-height:none，不设内部滚动。旧 scrollTop 跟随逻辑已移除。

  const handleToggle = useCallback(() => {
    if (isStreaming) return
    setExpanded(prev => !prev)
  }, [isStreaming])

  if (!isStreaming && !summary) return null

  const labelText = isStreaming ? t('refine.processing') : t('sessionDivider.refined')

  return (
    <div className="message-row">
      <div className={`message-bubble refine ${isStreaming ? 'refine-streaming' : ''}`}>
        {/* ── Header：仿 agent 气泡的名称+时间 ── */}
        <div
          className="message-header"
          onClick={handleToggle}
          role="button"
          tabIndex={0}
          aria-expanded={expanded}
        >
          <span className="message-label assistant">
            <IconSparkles size={12} />
            &nbsp;{labelText}
          </span>

          <span className="message-time">
            {messageCount > 0 && !isStreaming && (
              <>
                {messageCount} 条 → 1 条{ratio && <> · {ratio}%</>}
              </>
            )}
            {sessionId && !isStreaming && <> · #{sessionId.slice(0, 8)}</>}
            {isStreaming && <span className="refine-streaming-dot" />}
          </span>

          <span className={`refine-header-toggle ${expanded ? 'expanded' : ''}`}>
            <IconChevronDown size={12} />
          </span>
        </div>

        {/* ── Content ── */}
        {hasBody && (
          <div
            ref={bodyRef}
            className={`message-content refine ${expanded ? 'expanded' : 'collapsed'} ${isStreaming ? 'streaming' : ''}`}
          >
            <MarkdownContent content={displayContent} />
            {isStreaming && <span className="message-thinking-cursor" />}
          </div>
        )}
      </div>
    </div>
  )
}
