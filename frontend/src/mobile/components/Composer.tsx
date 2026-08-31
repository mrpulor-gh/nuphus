/**
 * 底部悬浮输入栏：圆角 + 毛玻璃（覆盖滚动内容时保持可读，iOS 底栏范式）
 * - font-size ≥16px（防 iOS focus 自动放大）
 * - 触摸目标 ≥44pt；Enter 换行、独立发送按钮（iOS 范式）
 * - 离线禁用发送
 *
 * 单一按钮设计（零认知负担）：
 * - 空闲   → 发送（纸飞机）＝ 新执行
 * - 执行中 → 发送（纸飞机）＝ 追加指令打断（下一轮生效），底部提示说明
 * 无暂停/继续按钮——执行控制仅「终止」：执行中 + 空输入框 → 终止按钮（与桌面端三态一致）。
 *
 * 「+」扩展菜单（Action Sheet，贴输入栏上缘滑出）：
 * - 拍摄 / 相册：压缩后 data URL 入缩略图胶囊（≤9 张），随消息发送（/message images）
 * - 模式设置：Leader / Workflow 内联展开切换，发送时显式带 mode
 * - 模型设置：只读展示（session_info 下发），引导桌面端 /models 配置
 * 边界：执行中后端 busy 分支丢弃 images/mode → 图片项禁用，模式标注「下次生效」。
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import { Send, Plus, Camera, Image as ImageIcon, Mic, Square, X, Loader2 } from 'lucide-react'
import { t } from '../i18n'

/**
 * 发送防连点窗口：消息已乐观受理（空闲=乐观气泡 / 执行中=追加 toast），
 * 不等待后端整轮执行返回——后端 submit_user_message 同步等待整轮 agent
 * 执行完成（可数分钟），fetch 最长 15s 超时；若 sending 绑定 fetch 生命周期，
 * 追加消息会被上一次发送的锁卡住（用户实测：追加时发送按钮不可点击）。
 * 800ms 窗口足够防双击/连点，且追加场景按钮立即可用。
 */
const SEND_DEBOUNCE_MS = 800

/** 图片压缩上限：最长边 / JPEG 质量 / 单图字节（data URL 经 /message JSON 上传，原图必撑爆请求体） */
const MAX_IMAGE_EDGE = 1920
const JPEG_QUALITY = 0.8
const MAX_IMAGE_BYTES = 500 * 1024
/** 一次最多携带图片数（微信范式） */
const MAX_IMAGES = 9

export interface SendPayload {
  images?: string[]
  mode?: string
}

interface Props {
  disabled: boolean
  /** 鉴权 token（拉取桌面端模型配置） */
  token: string
  /** agent 显示名（GET /identity 下发，桌面端 soul 配置；无则 Nuphus） */
  assistantName?: string
  /** 当前模式（store.activity.mode，WS 事件同步） */
  mode: string
  /** 当前执行模型（store.model，session_info 事件下发，只读展示） */
  model?: string
  /** 会话累计上下文用量（store.tokenUsage，token_usage 事件） */
  tokenUsage?: { inputTokens: number; cacheHitTokens?: number }
  /** 执行中（store.activity.running）：图片禁用、模式标注下次生效 */
  isProcessing: boolean
  /** 终止执行（执行中 + 输入框空时按钮变为终止，POST /stop 直接终止；与桌面端三态一致） */
  onStopExecution?: () => void
  onSend: (content: string, opts?: SendPayload) => Promise<void>
  /** 模型切换成功回调（更新 store.model，下次发送即用新模型） */
  onModelChanged?: (model: string) => void
  /** 轻量反馈（图片超限 / 压缩失败 / 模式切换等） */
  onToast?: (text: string) => void
  /** 手动重新拉取历史（网络/应用切换后历史不显示时一键刷新，无需重启应用） */
  onReloadHistory?: () => void
}

/**
 * 图片压缩 Worker 封装：FileReader / createImageBitmap / canvas 全部在 Worker 内执行，
 * 主线程零解码负担——UI 始终响应（菜单关闭、进度指示、错误提示必然渲染）。
 * iOS 18 standalone 实测：主线程解码 48MP 原图会冻结 UI（React 不渲染、定时器不触发）。
 */
let imageWorker: Worker | null = null
let workerSeq = 0
const workerPending = new Map<
  number,
  { resolve: (v: string) => void; reject: (e: Error) => void }
>()

function getImageWorker(): Worker {
  if (!imageWorker) {
    imageWorker = new Worker(new URL('../image-worker.ts', import.meta.url), { type: 'module' })
    imageWorker.onmessage = (
      e: MessageEvent<{ id: number; ok: boolean; dataUrl?: string; error?: string }>,
    ) => {
      const { id, ok, dataUrl, error } = e.data
      const pending = workerPending.get(id)
      if (!pending) return
      workerPending.delete(id)
      if (ok && dataUrl) pending.resolve(dataUrl)
      else pending.reject(new Error(error || t('mobile.imageProcessFailed')))
    }
    imageWorker.onerror = e => {
      const err = new Error(`${t('mobile.imageWorkerError')}: ${e.message}`)
      workerPending.forEach(p => p.reject(err))
      workerPending.clear()
    }
  }
  return imageWorker
}

/** 提交一张图片到 Worker 压缩，返回 data URL（Promise 一定 settle：成功/失败皆回传） */
function compressImageInWorker(file: File): Promise<string> {
  const id = ++workerSeq
  return new Promise((resolve, reject) => {
    workerPending.set(id, { resolve, reject })
    getImageWorker().postMessage({ id, file })
  })
}

/** token 数量缩写：1.0M / 320K / 860 */
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1000) return `${Math.round(n / 1000)}K`
  return String(n)
}

export default function Composer({
  disabled,
  token,
  assistantName,
  mode,
  model,
  tokenUsage,
  isProcessing,
  onStopExecution,
  onSend,
  onModelChanged,
  onToast,
  onReloadHistory,
}: Props) {
  const [value, setValue] = useState('')
  const [sending, setSending] = useState(false)
  const inputRef = useRef<HTMLTextAreaElement>(null)

  // ── 「+」扩展菜单状态（图片入口专用：拍摄 / 相册；模式/模型/重拉已整合进设置抽屉） ──
  const [menuOpen, setMenuOpen] = useState(false)
  const [pendingImages, setPendingImages] = useState<string[]>([])
  /** 图片压缩处理中（选图后立即反馈，避免「无反应」感） */
  const [imageProcessing, setImageProcessing] = useState(false)
  /** 处理中进度：当前第几张 / 共几张 */
  const [imageTotal, setImageTotal] = useState(0)
  const [imageDone, setImageDone] = useState(0)

  const plusBtnRef = useRef<HTMLButtonElement>(null)
  const sheetRef = useRef<HTMLDivElement>(null)
  // 轻面板：无全屏遮罩，点击面板外任意处关闭
  useEffect(() => {
    if (!menuOpen) return
    const onDown = (e: PointerEvent) => {
      const target = e.target as Node
      if (plusBtnRef.current?.contains(target)) return
      if (sheetRef.current?.contains(target)) return
      setMenuOpen(false)
    }
    document.addEventListener('pointerdown', onDown)
    return () => document.removeEventListener('pointerdown', onDown)
  }, [menuOpen])

  const hasText = value.trim().length > 0
  const hasImages = pendingImages.length > 0
  // 发送按钮可用：离线禁用（与桌面端一致，图片是附加项，仍需文字可发）
  const canSend = hasText && !disabled && !sending

  const handleSend = () => {
    const content = value.trim()
    if (!content || !canSend) return
    // 执行中后端 busy 分支只入队 message，images 会被静默丢弃——
    // 发送时显式剥离图片并告知，避免用户以为图已发出
    const images = isProcessing ? undefined : hasImages ? pendingImages : undefined
    if (isProcessing && hasImages) onToast?.(t('mobile.imagesRemovedWhileBusy'))
    setSending(true)
    setValue('')
    setPendingImages([])
    // 复位高度
    if (inputRef.current) inputRef.current.style.height = 'auto'
    // 防连点短窗口后立即释放按钮：消息已乐观受理（空闲=乐观气泡 /
    // 执行中=追加 toast），发送按钮不等待后端整轮执行返回——
    // 后端同步等待执行完成（可数分钟），fetch 15s 超时前若锁着 sending，
    // 追加消息会被上一次发送的锁卡住（用户实测：追加时发送按钮不可点击）。
    const guard = setTimeout(() => setSending(false), SEND_DEBOUNCE_MS)
    // onSend 内部（App.handleSend）已 try/catch 处理失败（toast / 撤销气泡），
    // 此处仅需吞掉意外 reject，避免 unhandled rejection 污染控制台。
    // ⚠️ finally 必须同时复位 sending：若 promise 在 guard 触发前 settle，
    // clearTimeout 会取消 800ms 复位定时器，导致 sending 永久卡 true、按钮锁死
    // （用户实测：发送后按钮直接锁住，无法再发消息）。
    void onSend(content, {
      images,
      mode,
    })
      .catch(() => {})
      .finally(() => {
        clearTimeout(guard)
        setSending(false)
      })
  }

  // ── 主按钮：恒为「发送」──
  // 空闲 = 新执行；执行中 = 追加指令打断（下一轮生效）。单一按钮，零认知负担。
  const handleInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setValue(e.target.value)
    const el = e.target
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 120)}px`
  }

  // ── 「+」菜单 ──
  const toggleMenu = () => {
    setMenuOpen(prev => {
      const next = !prev
      // 打开时收起键盘，避免弹层被键盘挤压
      if (next) inputRef.current?.blur()
      return next
    })
  }

  /** 图片文件 → Worker 压缩 → 入胶囊区（最多 9 张）。
   *  主线程零解码：立即关面板 + 显示「处理中 i/n」进度，逐张回传插入——
   *  任何一步失败都 toast 具体原因，杜绝静默 */
  const handleFiles = async (e: React.ChangeEvent<HTMLInputElement>) => {
    // ⚠️ FileList 是 live 对象：必须先拷贝成数组，再清空 input.value——
    // 否则 e.target.value='' 会把之前捕获的 FileList 引用同步清空（length 变 0），
    // 导致「已选图却静默 return」——这是此前所有版本无反应的真正根因。
    const pickedAll = Array.from(e.target.files ?? [])
    const count = pickedAll.length
    console.log('[mobile] files selected:', count)
    e.target.value = '' // 允许重复选择同一文件（清空不再影响已拷贝的 File 数组）
    if (count === 0) {
      onToast?.(t('mobile.noImagePicked'))
      return
    }
    const room = MAX_IMAGES - pendingImages.length
    if (room <= 0) {
      onToast?.(`${t('mobile.maxImages')} ${MAX_IMAGES}`)
      return
    }
    const picked = pickedAll.slice(0, room)
    // 立即关闭面板 + 显示处理中进度——主线程不阻塞，必然渲染
    setMenuOpen(false)
    setImageTotal(picked.length)
    setImageDone(0)
    setImageProcessing(true)
    try {
      for (const [i, f] of picked.entries()) {
        try {
          const dataUrl = await compressImageInWorker(f)
          setPendingImages(prev => [...prev, dataUrl])
        } catch (err) {
          const reason = err instanceof Error ? err.message : String(err)
          onToast?.(`${t('mobile.imageProcessFailed')}: ${reason}`)
          console.warn('[mobile] image compress failed:', reason)
        } finally {
          setImageDone(i + 1)
        }
      }
    } finally {
      setImageProcessing(false)
    }
  }

  /** 语音输入（A 方案：系统键盘听写，零权限零后端）：
   *  关闭面板 → 聚焦输入框弹出系统键盘 → 引导点击键盘麦克风图标说话。
   *  ⚠️ iOS 要求 focus() 在用户手势同步栈内执行——setTimeout 延迟会丢失
   *  用户激活（user activation），键盘不弹出（实测「点击语音没弹键盘」根因） */
  const handleVoiceInput = (e?: React.MouseEvent) => {
    e?.preventDefault()
    setMenuOpen(false)
    onToast?.(t('mobile.voiceKeyboardHint'))
    inputRef.current?.focus()
  }

  const removeImage = (idx: number) => {
    setPendingImages(prev => prev.filter((_, i) => i !== idx))
  }

  return (
    <footer className="mobile-composer">
      {(pendingImages.length > 0 || imageProcessing) && (
        <div className="mobile-composer-images">
          {pendingImages.map((src, i) => (
            <div className="mobile-composer-image" key={i}>
              <img src={src} alt={`待发送图片 ${i + 1}`} />
              <button
                type="button"
                className="mobile-composer-image-remove"
                onClick={() => removeImage(i)}
                aria-label="移除图片"
              >
                <X size={12} aria-hidden="true" />
              </button>
            </div>
          ))}
          {imageProcessing && (
            <div className="mobile-composer-image-processing" role="status">
              <Loader2 size={14} className="mobile-spin" aria-hidden="true" />
              <span className="mobile-composer-image-processing-count">
                {imageDone + 1}/{imageTotal}
              </span>
              <span>处理图片中…</span>
            </div>
          )}
        </div>
      )}
      <div className="mobile-composer-inner">
        <div className="mobile-composer-pill">
          <button
            type="button"
            ref={plusBtnRef}
            className={`mobile-composer-plus ${menuOpen ? 'open' : ''}`}
            onClick={toggleMenu}
            aria-label={menuOpen ? t('mobile.closeExtMenu') : t('mobile.openExtMenu')}
            aria-expanded={menuOpen}
          >
            <Plus size={22} aria-hidden="true" />
          </button>
          <textarea
            ref={inputRef}
            className="mobile-composer-input"
            rows={1}
            enterKeyHint="send"
            aria-label={t('mobile.messageInput')}
            placeholder={
              disabled
                ? t('mobile.placeholderOffline')
                : `${t('mobile.placeholderSendTo')} ${assistantName || 'Nuphus'}…`
            }
            value={value}
            onChange={handleInput}
            onKeyDown={e => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                void handleSend()
              }
            }}
          />
          <button
            type="button"
            className="mobile-composer-mic"
            onClick={e => handleVoiceInput(e)}
            aria-label={t('mobile.voiceInput')}
          >
            <Mic size={20} aria-hidden="true" />
          </button>
          {/* 发送 / 终止三态（与桌面端一致）：
              执行中 + 输入框空 → 终止按钮（实色红底白方块，点击终止）；
              否则 → 发送按钮（执行中+有内容 = 追加指令；空闲+空 = 待命灰显） */}
          {isProcessing && !hasText && !sending ? (
            <button
              type="button"
              className="mobile-composer-stop"
              onClick={onStopExecution}
              aria-label="终止"
            >
              <Square size={14} fill="currentColor" aria-hidden="true" />
            </button>
          ) : (
            <button
              type="button"
              className="mobile-composer-send"
              disabled={!canSend}
              onClick={handleSend}
              aria-label="发送"
            >
              <Send size={20} aria-hidden="true" />
            </button>
          )}
        </div>

        {/* ── 「+」扩展面板：iOS 原生 Action Sheet（固定底部弹出 + 毛玻璃 + 取消按钮） ── */}
        {menuOpen && (
          <>
            <div className="mobile-plus-sheet" role="menu" ref={sheetRef} aria-label="扩展菜单">
              <label className="mobile-plus-capsule" role="menuitem">
                {/* file input 内嵌 label：点击 label 由浏览器原生触发选择器——
                  standalone PWA（主屏幕启动）拦截 JS input.click()，label 触发不受限 */}
                <input
                  type="file"
                  accept="image/*"
                  capture="environment"
                  className="mobile-file-input-sr"
                  onChange={handleFiles}
                />
                <span className="mobile-plus-capsule-text">
                  <span className="mobile-plus-capsule-line">
                    <span className="mobile-plus-capsule-icon">
                      <Camera size={18} aria-hidden="true" />
                    </span>
                    <span className="mobile-plus-capsule-title">拍摄</span>
                  </span>
                  <span className="mobile-plus-capsule-sub">拍照后发送</span>
                </span>
              </label>
              <label className="mobile-plus-capsule" role="menuitem">
                <input
                  type="file"
                  accept="image/*"
                  multiple
                  className="mobile-file-input-sr"
                  onChange={handleFiles}
                />
                <span className="mobile-plus-capsule-text">
                  <span className="mobile-plus-capsule-line">
                    <span className="mobile-plus-capsule-icon">
                      <ImageIcon size={18} aria-hidden="true" />
                    </span>
                    <span className="mobile-plus-capsule-title">从相册选择</span>
                  </span>
                  <span className="mobile-plus-capsule-sub">从相册选择图片</span>
                </span>
              </label>
            </div>
          </>
        )}
      </div>
    </footer>
  )
}
