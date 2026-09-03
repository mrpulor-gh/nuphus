/**
 * RecIntentPanel.tsx — 意图面板（每步捕获/框选后弹出的确认层）
 *
 * 内容（设计意图 §二步骤4）：参数预览（窗口/坐标/截图缩略）→ 操作意图
 * （自由输入或模板快捷填充）→ 可变参数标记 → 异常分支备注（可选）→ 确认真实入画布。
 */

import { useCallback, useEffect, useMemo, useState } from 'react'
import { Check, X } from 'lucide-react'
import type { IntentPayload, RecPending } from './recorderTypes'
import { ACTION_LABEL } from './recorderTypes'
import {
  INTENT_SUGGESTIONS,
  REGION_ANCHOR_SEMANTIC,
  wheelAmount,
  wheelDirection,
} from './recorderMap'
import './recorder.css'

interface RecIntentPanelProps {
  pending: RecPending
  onConfirm: (payload: IntentPayload) => Promise<boolean>
  onCancel: () => void
  /** browser_click 已捕获元素的「重新捕获」入口（丢弃当前捕获回 capturing 重走 CDP 捕获） */
  onRecapture?: () => void
  /** browser_extract「选择目标元素」入口：复用 rec_browser 捕获（点击元素 = 选定元素），
   *  不生成点击步骤；捕获完成回到本面板展示已选元素（C2） */
  onSelectElement?: () => void
  /** 本步确认后将插入到画布的位置文案（「X」之后 / 层末尾） */
  insertTargetLabel?: string | null
}

function mapButton(btn: string | null | undefined): string {
  switch (btn) {
    case 'right':
      return '右键'
    case 'middle':
      return '中键'
    default:
      return '左键'
  }
}

const KEY_DISPLAY: Record<string, string> = {
  ctrl: 'Ctrl',
  shift: 'Shift',
  alt: 'Alt',
  win: 'Win',
  enter: 'Enter',
  esc: 'Esc',
  tab: 'Tab',
  space: 'Space',
  backspace: 'Backspace',
  delete: 'Delete',
  home: 'Home',
  end: 'End',
  pageup: 'PageUp',
  pagedown: 'PageDown',
  '⏎': 'Enter',
  '⎋': 'Esc',
  '⇥': 'Tab',
  '␣': 'Space',
  '⌫': 'Backspace',
  '⌦': 'Delete',
  '↖': 'Home',
  '↘': 'End',
  '⇞': 'PageUp',
  '⇟': 'PageDown',
}

function displayKeys(keys: string[]): string {
  if (!keys || keys.length === 0) return '（无）'
  return keys.map(k => KEY_DISPLAY[k.toLowerCase()] ?? k.toUpperCase()).join(' + ')
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="rec-preview-row">
      <span className="rec-preview-label">{label}</span>
      <span className="rec-preview-value">{children}</span>
    </div>
  )
}

export function RecIntentPanel({
  pending,
  onConfirm,
  onCancel,
  onRecapture,
  onSelectElement,
  insertTargetLabel,
}: RecIntentPanelProps) {
  const [intent, setIntent] = useState('')
  const [variable, setVariable] = useState(false)
  const [exception, setException] = useState('')
  const [text, setText] = useState('{{input}}')
  const [send, setSend] = useState('none')
  const [seconds, setSeconds] = useState(1)
  const [target, setTarget] = useState<'window' | 'chatagent'>('window')
  const [clickAfter, setClickAfter] = useState<'none' | 'click' | 'double_click'>('none')
  const [url, setUrl] = useState('')
  const [contentNote, setContentNote] = useState('')
  const [busy, setBusy] = useState(false)
  const [localError, setLocalError] = useState<string | null>(null)

  /** browser_click 默认意图：点击「元素文本」/ selector（让用户少一步输入，且意图可执行） */
  function defaultClickIntent(): string {
    const bc = pending.browserCapture
    if (!bc) return ''
    const textPart = (bc.text ?? '').trim().slice(0, 24)
    const what = textPart || bc.selector
    return `点击“${what}”`
  }

  // pending 变化（每步新捕获）→ 表单重置
  useEffect(() => {
    setIntent('')
    setVariable(false)
    setException('')
    setText('{{input}}')
    setSend('none')
    setSeconds(1)
    setTarget('window')
    setClickAfter('none')
    setUrl('')
    setContentNote('')
    setLocalError(null)
    if (pending.action === 'browser_click' && pending.browserCapture) {
      setIntent(defaultClickIntent())
    }
    if (pending.action === 'browser_extract') {
      setContentNote(pending.browserCapture?.selector ? '获取该元素内容' : '获取当前页面主要内容')
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending])

  // Esc 关闭面板
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onCancel()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [onCancel])

  const preview = useMemo(() => {
    const ev = pending.event
    const ov = pending.overlay
    const rows: { label: string; value: React.ReactNode }[] = []
    if (pending.action === 'click') {
      if (ev) {
        rows.push({ label: '目标窗口', value: ev.window_title || '（未知）' })
        rows.push({ label: '坐标', value: `(${ev.x}, ${ev.y})` })
        rows.push({
          label: '动作',
          value: ev.kind === 'double_click' ? '双击' : `单击（${mapButton(ev.button)}）`,
        })
      }
    } else if (pending.action === 'scroll') {
      if (ev) {
        rows.push({ label: '目标窗口', value: ev.window_title || '（未知）' })
        rows.push({ label: '坐标', value: `(${ev.x}, ${ev.y})` })
        rows.push({
          label: '滚轮',
          value: `${wheelDirection(ev.wheel_delta) === 'up' ? '上滚' : '下滚'} ${wheelAmount(ev.wheel_delta)} 格`,
        })
      }
    } else if (pending.action === 'text') {
      if (ev) {
        rows.push({ label: '定位窗口', value: ev.window_title || '（未知）' })
        rows.push({ label: '定位坐标', value: `(${ev.x}, ${ev.y})（用于在目标输入框聚焦）` })
      }
    } else if (pending.action === 'hotkey') {
      if (ev) {
        rows.push({ label: '目标窗口', value: ev.window_title || '（未知）' })
        rows.push({ label: '按键组合', value: displayKeys(ev.keys ?? []) })
      }
    } else if (pending.action === 'region' || pending.action === 'find_image') {
      if (ov) {
        rows.push({
          label: '区域',
          value: `${ov.rect.width}×${ov.rect.height} @ (${ov.rect.x}, ${ov.rect.y})`,
        })
        rows.push({
          label: '截图',
          value: ov.base64 ? (
            <img
              src={ov.base64}
              alt="ROI 预览"
              className="rec-preview-thumb"
              style={{ display: 'block', marginTop: 4 }}
            />
          ) : (
            <span className="rec-mono">{ov.path}</span>
          ),
        })
      }
    } else if (pending.action === 'browser_click') {
      const bc = pending.browserCapture
      if (bc) {
        const textPart = (bc.text ?? '').trim()
        rows.push({
          label: '元素',
          value: textPart ? `「${textPart.slice(0, 40)}」` : `<${bc.tag || 'element'}>（无文本）`,
        })
        rows.push({
          label: 'CSS selector',
          value: <span className="rec-mono rec-mono--wrap">{bc.selector}</span>,
        })
        if (bc.url) {
          rows.push({
            label: '所在页面',
            value: <span className="rec-mono rec-mono--wrap">{bc.url}</span>,
          })
        }
      }
    } else if (pending.action === 'browser_extract') {
      const bc = pending.browserCapture
      if (bc) {
        const textPart = (bc.text ?? '').trim()
        rows.push({
          label: '已选元素',
          value: textPart ? `「${textPart.slice(0, 40)}」` : `<${bc.tag || 'element'}>（无文本）`,
        })
        rows.push({
          label: 'CSS selector',
          value: <span className="rec-mono rec-mono--wrap">{bc.selector}</span>,
        })
        if (bc.url) {
          rows.push({
            label: '所在页面',
            value: <span className="rec-mono rec-mono--wrap">{bc.url}</span>,
          })
        }
      }
    }
    return rows
  }, [pending])

  const confirm = async () => {
    if (!intent.trim()) {
      setLocalError('请填写操作意图（这步在做什么），便于 WorkflowAgent 泛化')
      return
    }
    if (pending.action === 'browser_navigate' && !url.trim()) {
      setLocalError('请填写要打开的网址（browser_navigate 必填）')
      return
    }
    if (pending.action === 'browser_click' && !pending.browserCapture?.selector) {
      setLocalError('尚未捕获到网页元素：请点「重新捕获」后在浏览器中完成点击')
      return
    }
    setBusy(true)
    setLocalError(null)
    const ok = await onConfirm({
      intent: intent.trim(),
      exceptionNote: exception,
      variable,
      text,
      send,
      seconds,
      target,
      clickAfter,
      url: url.trim(),
      contentNote: contentNote.trim(),
    })
    if (!ok) {
      setBusy(false)
      setLocalError('加入画布失败（只读或插入被拒绝），请检查画布状态后重试')
    }
  }

  const suggestions = INTENT_SUGGESTIONS[pending.action] ?? []

  return (
    <div className="rec-intent-backdrop">
      <div className="rec-intent">
        <div className="rec-intent-head">
          <h3 className="rec-intent-title">
            确认录制步骤 · 第 {pending.stepNo} 步
            <span className="rec-intent-action">{ACTION_LABEL[pending.action]}</span>
          </h3>
          <button type="button" className="rec-intent-close" onClick={onCancel} title="取消本步">
            <X size={15} />
          </button>
        </div>

        <div className="rec-intent-body">
          <div className="rec-section">
            <div className="rec-section-title">参数预览</div>
            {preview.length > 0 ? (
              <div className="rec-preview">
                {preview.map((r, i) => (
                  <Row key={i} label={r.label}>
                    {r.value}
                  </Row>
                ))}
              </div>
            ) : (
              <div className="rec-preview-empty">
                {pending.action === 'sleep'
                  ? '等待延时步骤：下方填写秒数'
                  : pending.action === 'browser_navigate'
                    ? '打开网址步骤：下方填写目标 URL'
                    : pending.action === 'browser_extract'
                      ? '网页内容步骤：下方描述要获取的页面内容'
                      : '（无捕获参数）'}
              </div>
            )}
            {pending.action === 'region' && (
              <div className="rec-semantic-note">{REGION_ANCHOR_SEMANTIC}</div>
            )}
            {pending.action === 'find_image' && (
              <>
                <div className="rec-semantic-note">
                  将生成 desktop_find_image 步骤（template = 上方 ROI 截图，threshold 0.9， 搜索区域
                  = 框选 rect），执行时在当前屏幕查找该模板。
                </div>
                <div className="rec-field rec-field--inline rec-intent-extra">
                  <label className="rec-label">找图后</label>
                  <select
                    className="rec-input"
                    value={clickAfter}
                    onChange={e => setClickAfter(e.target.value as typeof clickAfter)}
                  >
                    <option value="none">无动作（仅定位/断言）</option>
                    <option value="click">单击匹配位置</option>
                    <option value="double_click">双击匹配位置</option>
                  </select>
                </div>
                {clickAfter !== 'none' && (
                  <div className="rec-semantic-note rec-semantic-note--warn">
                    将额外生成 desktop_mouse 步骤：find_image 运行时把匹配坐标写入 capture
                    变量，点击步骤以模板引用驱动（点击落在匹配框左上角坐标；如需中心请让
                    WorkflowAgent 补 w/2 偏移）。
                  </div>
                )}
              </>
            )}
          </div>

          {pending.action === 'text' && (
            <>
              <div className="rec-section">
                <div className="rec-section-title">输入目标</div>
                <div className="rec-field rec-field--inline">
                  <label className="rec-label">方式</label>
                  <select
                    className="rec-input"
                    value={target}
                    onChange={e => setTarget(e.target.value as typeof target)}
                  >
                    <option value="window">窗口输入（定位点击处输入文本）</option>
                    <option value="chatagent">chatagent 处理</option>
                  </select>
                </div>
                {target === 'chatagent' ? (
                  <div className="rec-semantic-note rec-semantic-note--warn">
                    将生成 chat 步骤：本步交给 chatagent 处理（窗口/文本基础参数可为空，只填下方意图
                    + 备注）。运行时 ChatAgent 会收到「此处由 chatagent 处理：&lt;意图&gt;」并执行；
                    WorkflowAgent 阅读 record-draft 后可按需补全 chat 内容。
                  </div>
                ) : (
                  <div className="rec-semantic-note">
                    将生成 desktop_input 步骤（mode=type）：在刚捕获的定位坐标对应窗口输入文本。
                  </div>
                )}
              </div>

              {target === 'window' && (
                <div className="rec-section">
                  <div className="rec-section-title">输入内容（IME 无法自动采集文本，需手填）</div>
                  <div className="rec-field">
                    <textarea
                      className="rec-input rec-input--area"
                      value={text}
                      onChange={e => setText(e.target.value)}
                      placeholder="默认 {{input}}：运行时由工作流参数提供（脱敏原则）"
                      rows={2}
                    />
                  </div>
                  <div className="rec-field rec-field--inline">
                    <label className="rec-label">输入后按键</label>
                    <select
                      className="rec-input"
                      value={send}
                      onChange={e => setSend(e.target.value)}
                    >
                      <option value="none">不发送（仅输入）</option>
                      <option value="enter">回车</option>
                      <option value="tab">Tab</option>
                      <option value="ctrl+enter">Ctrl+Enter</option>
                    </select>
                  </div>
                </div>
              )}
            </>
          )}

          {pending.action === 'sleep' && (
            <div className="rec-section">
              <div className="rec-section-title">等待时长</div>
              <div className="rec-field rec-field--inline">
                <label className="rec-label">秒数</label>
                <input
                  className="rec-input rec-input--num"
                  type="number"
                  min={0.1}
                  step={0.1}
                  value={seconds}
                  onChange={e => setSeconds(Number(e.target.value) || 1)}
                />
              </div>
            </div>
          )}

          {pending.action === 'browser_navigate' && (
            <div className="rec-section">
              <div className="rec-section-title">网址</div>
              <div className="rec-field">
                <input
                  className="rec-input"
                  value={url}
                  onChange={e => setUrl(e.target.value)}
                  placeholder="https://example.com（必填）"
                />
              </div>
              <div className="rec-semantic-note">
                将生成 browser_navigate 步骤：在浏览器中打开该网址（地址栏直达；WorkflowAgent
                可据本步意图把 URL 参数化）。
              </div>
            </div>
          )}

          {pending.action === 'browser_click' && (
            <div className="rec-section">
              <div className="rec-section-title">已捕获网页元素</div>
              <div className="rec-semantic-note">
                将生成 browser_click 步骤（selector ={' '}
                <span className="rec-mono">{pending.browserCapture?.selector ?? '（未捕获）'}</span>
                ）：运行时在浏览器当前页用该 selector 定位并点击。页面结构变化可能导致失效，
                WorkflowAgent 可据意图修复。
              </div>
              {onRecapture && (
                <div className="rec-field rec-intent-extra">
                  <button
                    type="button"
                    className="rec-btn"
                    disabled={busy}
                    onClick={() => void onRecapture()}
                  >
                    重新捕获（再点一次浏览器中的目标元素）
                  </button>
                </div>
              )}
            </div>
          )}

          {pending.action === 'browser_extract' && (
            <div className="rec-section">
              <div className="rec-section-title">
                {pending.browserCapture?.selector ? '已选择目标元素（元素级提取）' : '获取内容说明'}
              </div>
              {pending.browserCapture?.selector ? (
                <>
                  <div className="rec-semantic-note">
                    将生成 browser_exec 步骤（h.extract selector），运行时只提取该元素文本：
                    selector = <span className="rec-mono">{pending.browserCapture.selector}</span>
                    。页面结构变化可能导致失效，WorkflowAgent 可据意图修复。
                  </div>
                  {onSelectElement && (
                    <div className="rec-field rec-intent-extra">
                      <button
                        type="button"
                        className="rec-btn"
                        disabled={busy}
                        onClick={() => void onSelectElement()}
                      >
                        重新选择元素（回到浏览器中点选另一个元素）
                      </button>
                    </div>
                  )}
                </>
              ) : (
                <>
                  <div className="rec-semantic-note">
                    默认整页提取：将生成 browser_extract 步骤，运行时获取浏览器当前页文本 （语义 =
                    在「打开网址」步骤之后取当前页）。若只需某个元素的内容，可点下方
                    「选择目标元素」用鼠标在浏览器中点选——生成元素级提取步骤，无需整页文本。
                  </div>
                  {onSelectElement && (
                    <div className="rec-field rec-intent-extra">
                      <button
                        type="button"
                        className="rec-btn"
                        disabled={busy}
                        onClick={() => void onSelectElement()}
                      >
                        选择目标元素（点选页面元素精确获取其内容）
                      </button>
                    </div>
                  )}
                </>
              )}
              <div className="rec-field" style={{ marginTop: 8 }}>
                <textarea
                  className="rec-input rec-input--area"
                  value={contentNote}
                  onChange={e => setContentNote(e.target.value)}
                  placeholder="描述要获取页面什么（默认：获取当前页面主要内容）"
                  rows={2}
                />
              </div>
            </div>
          )}

          <div className="rec-section">
            <div className="rec-section-title">操作意图（这步在做什么）</div>
            <div className="rec-suggestions">
              {suggestions.map(s => (
                <button
                  key={s}
                  type="button"
                  className={`rec-chip${intent === s ? ' is-active' : ''}`}
                  onClick={() => setIntent(s)}
                >
                  {s}
                </button>
              ))}
            </div>
            <input
              className="rec-input"
              value={intent}
              onChange={e => setIntent(e.target.value)}
              placeholder="必填：用一句话描述这一步的目的，如「点击搜索框并输入关键词」"
            />
          </div>

          <div className="rec-section">
            <label className="rec-check">
              <input
                type="checkbox"
                checked={variable}
                onChange={e => setVariable(e.target.checked)}
              />
              <span>
                可变参数标记
                <span className="rec-check-hint">（此值每次运行可能变化，需参数化）</span>
              </span>
            </label>
            <textarea
              className="rec-input rec-input--area"
              value={exception}
              onChange={e => setException(e.target.value)}
              placeholder="异常分支备注（可选）：如「若弹出登录框则中止 / 找不到元素时重试一次」"
              rows={2}
            />
          </div>

          {localError && <div className="rec-intent-error">{localError}</div>}
        </div>

        {/* 大王反馈：确认前展示将插入到画布的哪个间隙 */}
        <div className="rec-intent-insert">
          <span className="rec-intent-insert-label">插入位置</span>
          <span className="rec-intent-insert-value">{insertTargetLabel ?? '层末尾'}</span>
        </div>

        <div className="rec-intent-foot">
          <button type="button" className="rec-btn" onClick={onCancel} disabled={busy}>
            取消本步
          </button>
          <button
            type="button"
            className="rec-btn rec-btn--primary"
            onClick={() => void confirm()}
            disabled={busy}
          >
            <Check size={13} /> {busy ? '加入中…' : '确认并加入画布'}
          </button>
        </div>
      </div>
    </div>
  )
}
