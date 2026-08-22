import { useState, useEffect, useCallback, useRef, useMemo } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { createPortal } from 'react-dom'
import { convertFileSrc } from '@tauri-apps/api/core'
import { submitUserInput, rejectUserInput } from '../lib/api'
import MarkdownContent from '../chat/MarkdownContent'
import { IconX, IconShield } from '../../ui/Icons'
import { IconButton, Button } from '../../ui/Button'
import '../../styles/user-input-prompt.css'
import {
  useToolCapture,
  type CaptureResult,
  type CaptureMode,
  inputTypeToCaptureMode,
} from '../tools/useToolCapture'
import { playPopupSound } from '../../ui/sound'

/// 将文件系统路径转为浏览器可访问的 URL（Tauri asset protocol）
function toAssetUrl(path: string | null | undefined): string | null {
  if (!path) return null
  // 已经是 http/https/data/asset URL 则直接返回
  if (/^(https?:\/\/|data:|asset:\/\/|tauri:\/\/)/i.test(path)) return path
  try {
    return convertFileSrc(path)
  } catch {
    return null
  }
}

interface UserInputPromptProps {
  title: string
  prompt: string
  sensitive: boolean
  actionId: string
  inputType: string
  onSubmit: (id: string) => void
  onReject: (id: string) => void
  // ── icon_confirm ──
  iconPath?: string | null
  defaultName?: string | null
  defaultShortcut?: string | null
  relX?: number | null
  relY?: number | null
  defaultNote?: string | null
}

const CAPTURE_LABELS: Record<string, string> = {
  screenshot: '截取屏幕区域',
  region: '框选坐标区域',
  mouse_pos: '点击目标位置',
  color: '框选取色区域',
}

export function UserInputPrompt({
  title,
  prompt,
  sensitive,
  actionId,
  inputType = 'text',
  onSubmit,
  onReject,
  iconPath,
  defaultName,
  defaultShortcut,
  relX,
  relY,
  defaultNote,
}: UserInputPromptProps) {
  // ── text state ──
  const [value, setValue] = useState('')
  const inputRef = useRef<HTMLInputElement | HTMLTextAreaElement>(null)

  // 弹窗出现即播放请求音效
  useEffect(() => {
    playPopupSound('request')
  }, [])

  // ── capture state ──
  const { capture } = useToolCapture()
  const [captureResult, setCaptureResult] = useState<CaptureResult | null>(null)
  const [capturing, setCapturing] = useState(false)

  // ── icon_confirm state ──
  const [iconName, setIconName] = useState(defaultName || '')
  const [iconShortcut, setIconShortcut] = useState(defaultShortcut || '')
  const [iconX, setIconX] = useState(relX != null ? String(relX) : '')
  const [iconY, setIconY] = useState(relY != null ? String(relY) : '')
  const [iconNote, setIconNote] = useState(defaultNote || '')
  const [iconScreenshot, setIconScreenshot] = useState<string | null>(() => toAssetUrl(iconPath))

  // ── shared state ──
  const [busy, setBusy] = useState(false)
  const isText = inputType === 'text'
  const isIconConfirm = inputType === 'icon_confirm'

  // ══════════════════════════════════════════
  // Text submit
  // ══════════════════════════════════════════
  const handleTextSubmit = useCallback(async () => {
    if (busy || !value.trim()) return
    setBusy(true)
    try {
      await submitUserInput(actionId, value)
      onSubmit(actionId)
    } catch (e) {
      console.warn('submission failed:', e)
      setBusy(false)
    }
  }, [busy, value, actionId, onSubmit])

  // ══════════════════════════════════════════
  // Capture submit (screenshot/region/...)
  // ══════════════════════════════════════════
  const handleCaptureSubmit = useCallback(async () => {
    if (busy || !captureResult) return
    setBusy(true)
    try {
      const payload: any = {}
      if (captureResult.mode === 'screenshot') {
        payload.path = captureResult.path
        if (captureResult.region) payload.region = captureResult.region
      } else if (captureResult.mode === 'picker') {
        payload.region = captureResult.region
      } else if (captureResult.mode === 'mouse_pos') {
        payload.pos = captureResult.region
          ? { x: captureResult.region.x, y: captureResult.region.y }
          : undefined
      }
      await submitUserInput(actionId, JSON.stringify(payload))
      onSubmit(actionId)
    } catch (e) {
      console.warn('submission failed:', e)
      setBusy(false)
    }
  }, [busy, captureResult, actionId, onSubmit])

  // ══════════════════════════════════════════
  // icon_confirm submit
  // ══════════════════════════════════════════
  const handleIconConfirmSubmit = useCallback(async () => {
    if (busy || !iconName.trim()) return
    setBusy(true)
    try {
      const payload: Record<string, any> = { name: iconName.trim() }
      if (iconShortcut.trim()) payload.shortcut = iconShortcut.trim()
      const nx = parseInt(iconX, 10)
      const ny = parseInt(iconY, 10)
      if (!isNaN(nx)) payload.rel_x = nx
      if (!isNaN(ny)) payload.rel_y = ny
      if (iconNote.trim()) payload.note = iconNote.trim()
      await submitUserInput(actionId, JSON.stringify(payload))
      onSubmit(actionId)
    } catch (e) {
      console.warn('submission failed:', e)
      setBusy(false)
    }
  }, [busy, iconName, iconShortcut, iconX, iconY, iconNote, actionId, onSubmit])

  // ══════════════════════════════════════════
  // Reject
  // ══════════════════════════════════════════
  const handleReject = useCallback(async () => {
    if (busy) return
    setBusy(true)
    try {
      await rejectUserInput(actionId)
    } catch (_) {
      /* ignore */
    }
    onReject(actionId)
  }, [busy, actionId, onReject])

  // ══════════════════════════════════════════
  // Capture
  // ══════════════════════════════════════════
  const handleCapture = useCallback(
    async (mode: CaptureMode) => {
      if (busy) return
      setCapturing(true)
      try {
        const result = await capture(mode)
        if (result) setCaptureResult(result)
      } finally {
        setCapturing(false)
      }
    },
    [busy, capture],
  )

  // ══════════════════════════════════════════
  // icon_confirm: re-capture icon screenshot
  // ══════════════════════════════════════════
  const handleIconCapture = useCallback(async () => {
    if (busy) return
    setCapturing(true)
    try {
      const result = await capture('screenshot')
      if (result?.path) setIconScreenshot(toAssetUrl(result.path))
    } finally {
      setCapturing(false)
    }
  }, [busy, capture])

  // ══════════════════════════════════════════
  // Keyboard
  // ══════════════════════════════════════════
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !busy) handleReject()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [handleReject, busy])

  // text auto-focus
  useEffect(() => {
    if (isText && inputRef.current) inputRef.current.focus()
  }, [isText])

  const promptContent = (
    <div className="uip-overlay">
      <div className={`uip-panel ${isIconConfirm ? 'uip-panel--lg' : 'uip-panel--md'}`}>
        {/* Header */}
        <div className="uip-header">
          <div className="uip-title-group">
            {sensitive && <IconShield size={18} />}
            <span className="uip-title">{title}</span>
          </div>
          {!busy && (
            <IconButton label="关闭" onClick={handleReject}>
              <IconX size={18} />
            </IconButton>
          )}
        </div>

        <div className="uip-prompt">
          <MarkdownContent content={prompt} />
        </div>

        {/* ── TEXT ── */}
        {isText && (
          <>
            <input
              ref={inputRef as any}
              type={sensitive ? 'password' : 'text'}
              value={value}
              onChange={e => setValue(e.target.value)}
              onKeyDown={e => {
                if (e.key === 'Enter') handleTextSubmit()
              }}
              placeholder={sensitive ? '········' : '输入内容...'}
              className="uip-input"
            />
            <div className="uip-actions uip-actions--spaced">
              <Button variant="ghost" onClick={handleReject} disabled={busy}>
                取消
              </Button>
              <Button variant="primary" onClick={handleTextSubmit} disabled={busy || !value.trim()}>
                {busy ? '处理中...' : '提交'}
              </Button>
            </div>
          </>
        )}

        {/* ── ICON_CONFIRM ── */}
        {isIconConfirm && (
          <>
            {/* icon preview */}
            <div className="uip-field--spaced">
              <div className="uip-label">图标预览</div>
              <div className="uip-icon-preview">
                {iconScreenshot ? (
                  <img src={iconScreenshot} alt="图标预览" />
                ) : (
                  <span className="uip-hint">点击下方按钮截取图标</span>
                )}
              </div>
              <button
                className="uip-capture-btn"
                onClick={handleIconCapture}
                disabled={busy || capturing}
              >
                {capturing ? '截取中...' : '重新截取图标'}
              </button>
            </div>

            {/* name field */}
            <div className="uip-field">
              <div className="uip-label">功能名称 *</div>
              <input
                value={iconName}
                onChange={e => setIconName(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter') handleIconConfirmSubmit()
                }}
                placeholder="如：搜索、资源管理器..."
                className="uip-input"
                autoFocus
              />
            </div>

            {/* shortcut field */}
            <div className="uip-field">
              <div className="uip-label">快捷键（可选）</div>
              <input
                value={iconShortcut}
                onChange={e => setIconShortcut(e.target.value)}
                placeholder="如：Ctrl+Shift+F"
                className="uip-input"
              />
            </div>

            {/* coordinates row */}
            <div className="uip-row">
              <div>
                <div className="uip-label">相对 X</div>
                <input
                  value={iconX}
                  onChange={e => setIconX(e.target.value)}
                  placeholder="24"
                  className="uip-input uip-input--center"
                />
              </div>
              <div>
                <div className="uip-label">相对 Y</div>
                <input
                  value={iconY}
                  onChange={e => setIconY(e.target.value)}
                  placeholder="120"
                  className="uip-input uip-input--center"
                />
              </div>
            </div>

            {/* note field */}
            <div className="uip-field--spaced">
              <div className="uip-label">备注（可选）</div>
              <input
                value={iconNote}
                onChange={e => setIconNote(e.target.value)}
                placeholder="如：自定义图标名称"
                className="uip-input"
              />
            </div>

            {/* actions */}
            <div className="uip-actions">
              <Button variant="ghost" onClick={handleReject} disabled={busy}>
                取消
              </Button>
              <Button
                variant="primary"
                onClick={handleIconConfirmSubmit}
                disabled={busy || !iconName.trim()}
              >
                {busy ? '处理中...' : '确认提交'}
              </Button>
            </div>
          </>
        )}

        {/* ── CAPTURE TYPES (screenshot / region / mouse_pos / color) ── */}
        {!isText && !isIconConfirm && (
          <>
            {!captureResult ? (
              <div>
                <button
                  className="uip-capture-main-btn"
                  onClick={() => handleCapture(inputTypeToCaptureMode(inputType))}
                  disabled={busy || capturing}
                >
                  {capturing ? '操作中...' : CAPTURE_LABELS[inputType] || '开始捕获'}
                </button>
              </div>
            ) : (
              <>
                <div className="uip-capture-result">
                  <div className="uip-capture-result-label">
                    已捕获
                    {inputType === 'screenshot'
                      ? '截图'
                      : inputType === 'region'
                        ? '区域'
                        : inputType === 'color'
                          ? '取色区域'
                          : inputType === 'mouse_pos'
                            ? '坐标'
                            : '截图'}
                  </div>
                  {captureResult.mode === 'screenshot' && captureResult.path && (
                    <div>
                      <img className="uip-capture-img" src={captureResult.path} alt="截图" />
                      {captureResult.region && (
                        <div className="uip-hint uip-hint--spaced">
                          区域: x={captureResult.region.x} y={captureResult.region.y} w=
                          {captureResult.region.width} h={captureResult.region.height}
                        </div>
                      )}
                    </div>
                  )}
                  {captureResult.mode === 'picker' && captureResult.region && (
                    <div className="uip-capture-value">
                      x={captureResult.region.x} y={captureResult.region.y} w=
                      {captureResult.region.width} h={captureResult.region.height}
                    </div>
                  )}
                  {captureResult.mode === 'mouse_pos' && captureResult.region && (
                    <div className="uip-capture-value">
                      x={captureResult.region.x} y={captureResult.region.y}
                    </div>
                  )}
                </div>

                <div className="uip-btn-row">
                  <Button
                    variant="ghost"
                    onClick={() => {
                      setCaptureResult(null)
                    }}
                    disabled={busy}
                  >
                    重新捕获
                  </Button>
                  <Button variant="primary" onClick={handleCaptureSubmit} disabled={busy}>
                    {busy ? '处理中...' : '提交结果'}
                  </Button>
                </div>
              </>
            )}

            <div className="uip-footer-hint">
              <span>
                <kbd className="uip-kbd">Esc</kbd> 取消
              </span>
            </div>
          </>
        )}
      </div>
    </div>
  )

  return createPortal(promptContent, document.body)
}
