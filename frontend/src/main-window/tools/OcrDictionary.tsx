// OcrDictionary.tsx — Dictionary OCR
import { useState, useEffect, useRef, useCallback } from 'react'
import { Button, IconButton } from '../../ui/Button'
async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core')
  return (await invoke(cmd, args)) as T
}

interface DictEntry {
  char: string
  width: number
  height: number
  data_hex: string
}
interface ColorSpec {
  r: number
  g: number
  b: number
  dr: number
  dg: number
  db: number
}
interface CharSegment {
  x: number
  y: number
  width: number
  height: number
  data_hex: string
}
interface DictInfo {
  name: string
  size: number
  modified: number
}

export interface OcrDictionaryProps {
  onClose?: () => void
}

export function OcrDictionary({ onClose }: OcrDictionaryProps) {
  const [tab, setTab] = useState('auto')
  const [fgColor, setFgColor] = useState('#FFFFFF')
  const [fgDr, setFgDr] = useState(30)
  const [fgDg, setFgDg] = useState(30)
  const [fgDb, setFgDb] = useState(30)

  // 自动识别
  const [dictList, setDictList] = useState<DictInfo[]>([])
  const [selectedDictName, setSelectedDictName] = useState('__auto__')
  const [matchedDictName, setMatchedDictName] = useState('')
  const [autoPath, setAutoPath] = useState('')
  const [autoPreview, setAutoPreview] = useState('')
  const [recognizedText, setRecognizedText] = useState('')
  const [recognizedConf, setRecognizedConf] = useState(0)
  const [autoLoading, setAutoLoading] = useState(false)
  const [autoProgressText, setAutoProgressText] = useState('')
  const [searchWord, setSearchWord] = useState('') // 待查找的词（如"系统"）
  const [wordMatches, setWordMatches] = useState<
    Array<{
      word: string
      sim: number
      x: number
      y: number
      width: number
      height: number
      char_count: number
    }>
  >([])

  // 添加字典
  const [colGap, setColGap] = useState(2)
  const [rowGap, setRowGap] = useState(2)
  const [wordGap, setWordGap] = useState(4)
  const [extractPath, setExtractPath] = useState('')
  const [capturePreview, setCapturePreview] = useState('')
  const [binarizedPreview, setBinarizedPreview] = useState('')
  const [segments, setSegments] = useState<CharSegment[]>([])
  const [charNames, setCharNames] = useState<Record<number, string>>({})
  const [selectedSegIdx, setSelectedSegIdx] = useState(0)
  const [dictName, setDictName] = useState('我的字库')
  const [addLoading, setAddLoading] = useState(false)
  const [dictEntries, setDictEntries] = useState<DictEntry[]>([])
  // Existing dictionary character cache (for dedup hint)
  const [existingDict, setExistingDict] = useState<Record<string, DictEntry[]>>({})

  // 自定义字库 tab
  const [customDicts, setCustomDicts] = useState<string[]>([])
  // 字体渲染（高级选项）
  const [showFontDialog, setShowFontDialog] = useState(false)
  const [fontFamily, setFontFamily] = useState('system-ui')
  const [renderText, setRenderText] = useState('')
  const [fontSize, setFontSize] = useState(14)

  // Load existing dictionary content (for dedup)
  const loadExistingDict = useCallback(async (name: string) => {
    if (!name.trim()) {
      setExistingDict({})
      return
    }
    try {
      const r = await tauriInvoke<{ entries: DictEntry[] }>('dict_load', { dictName: name.trim() })
      const grouped: Record<string, DictEntry[]> = {}
      for (const e of r?.entries || []) {
        if (!grouped[e.char]) grouped[e.char] = []
        grouped[e.char].push(e)
      }
      setExistingDict(grouped)
    } catch (_) {
      setExistingDict({})
    }
  }, [])

  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current)
      pollRef.current = null
    }
  }, [])

  useEffect(() => {
    loadDicts()
    loadDictList()
    return () => {
      stopPolling()
    }
  }, [])

  // Load existing content when entering add dict tab or switching dictName
  useEffect(() => {
    if (tab === 'add' && dictName.trim()) {
      loadExistingDict(dictName)
    } else if (tab.startsWith('c:')) {
      const name = tab.slice(2)
      if (name.trim()) loadExistingDict(name)
    }
  }, [tab, dictName, loadExistingDict])

  // Load dictionary list (for auto recognition)
  const loadDicts = async () => {
    try {
      const list = await tauriInvoke<DictInfo[]>('dict_ocr_list_dicts')
      setDictList(list || [])
    } catch (e) {
      console.error('[OcrDict] loadDicts:', e)
    }
  }

  // Read image base64
  const loadImagePreview = async (path: string) => {
    try {
      const r = await tauriInvoke<{ base64: string; mime: string }>('read_image_base64', {
        imagePath: path,
      })
      if (!r?.base64) return ''
      return `data:${r.mime || 'image/png'};base64,${r.base64}`
    } catch (e) {
      console.error('[OcrDict] loadImagePreview:', e)
      return ''
    }
  }

  const refreshBinarizedWith = async (
    path: string,
    r: number,
    g: number,
    b: number,
    dr: number,
    dg: number,
    db: number,
  ) => {
    try {
      const result = await tauriInvoke<{ data: string }>('dict_ocr_binarize_preview', {
        imagePath: path,
        r,
        g,
        b,
        dr,
        dg,
        db,
      })
      if (!result?.data) return ''
      return `data:image/png;base64,${result.data}`
    } catch (e) {
      console.error('[OcrDict] refreshBinarizedWith:', e)
      return ''
    }
  }

  const refreshBinarized = async (path: string) => {
    const c = hexToRgb(fgColor)
    if (!c) return ''
    return refreshBinarizedWith(path, c.r, c.g, c.b, fgDr, fgDg, fgDb)
  }

  const loadDictList = async () => {
    try {
      const list = await tauriInvoke<string[]>('dict_list')
      setCustomDicts(list)
    } catch (e) {
      console.error('[OcrDict] loadDictList:', e)
    }
  }

  const startCapture = (callback: (path: string) => void) => {
    tauriInvoke<any>('start_overlay_mask', { mode: 'ocr' })
      .then(() => {
        pollRef.current = setInterval(async () => {
          try {
            const raw = await tauriInvoke<any>('take_capture_result')
            if (raw === null || raw === undefined) return
            stopPolling()
            if (!raw.cancelled && raw.path) callback(raw.path)
          } catch (e) {
            console.error('[OcrDict] poll:', e)
            stopPolling()
          }
        }, 500)
      })
      .catch(e => console.error('[OcrDict] startCapture:', e))
  }

  // ── Auto recognize ──
  const doRecognize = async (path: string, dictName: string) => {
    setAutoLoading(true)
    setAutoProgressText('')
    setMatchedDictName('')
    setWordMatches([])

    if (dictName === '__auto__') {
      let unlisten: (() => void) | null = null
      try {
        const { listen } = await import('@tauri-apps/api/event')
        const unlistenFn = await listen<any>('auto-match-progress', event => {
          const p = event.payload
          if (p.type === 'matched' || p.type === 'done') return
          if (p.type === 'progress') {
            setAutoProgressText(`比对字库 ${p.current}/${p.total}: ${p.dict_name}`)
          }
        })
        unlisten = unlistenFn
      } catch (_) {
        /* Event listener non-critical */
      }

      try {
        const analysis = await tauriInvoke<{
          foreground: { r: number; g: number; b: number; dr: number; dg: number; db: number }
        }>('dict_ocr_analyze', { imagePath: path })
        const fg = analysis.foreground
        const result = await tauriInvoke<any>('dict_ocr_auto_match', {
          imagePath: path,
          r: fg.r,
          g: fg.g,
          b: fg.b,
          dr: fg.dr,
          dg: fg.dg,
          db: fg.db,
        })
        if (result?.matched && result.text) {
          setRecognizedText(result.text)
          setRecognizedConf(result.avg_sim || 0)
          setMatchedDictName(result.dict_name || '')
        } else {
          setRecognizedText('未能从已保存字库中匹配到文字')
          setRecognizedConf(0)
        }
      } catch (e: unknown) {
        setRecognizedText('识别失败: ' + String(e))
      } finally {
        if (unlisten) unlisten()
        setAutoLoading(false)
        setAutoProgressText('')
      }
    } else {
      try {
        const analysis = await tauriInvoke<{
          foreground: { r: number; g: number; b: number; dr: number; dg: number; db: number }
        }>('dict_ocr_analyze', { imagePath: path })
        const fg = analysis.foreground
        const result = await tauriInvoke<any>('dict_ocr_recognize', {
          imagePath: path,
          dictName,
          r: fg.r,
          g: fg.g,
          b: fg.b,
          dr: fg.dr,
          dg: fg.dg,
          db: fg.db,
          minColGap: 2,
          minRowGap: 2,
          wordGap: 4,
          sim: 1.0,
          word: searchWord.trim() || null,
        })
        if (result.is_word_match) {
          // 词匹配模式
          setWordMatches(result.word_matches || [])
          setRecognizedText(result.text || '未能识别')
          setRecognizedConf(
            result.word_matches?.length
              ? result.word_matches.reduce((a: number, c: any) => Math.max(a, c.sim || 0), 0)
              : 0,
          )
        } else {
          setRecognizedText(result.text || '未能识别')
          setRecognizedConf(
            result.matches?.length
              ? result.matches.reduce((a: number, c: any) => a + c.sim, 0) / result.matches.length
              : 0,
          )
          setWordMatches([])
        }
      } catch (e: unknown) {
        setRecognizedText('识别失败: ' + String(e))
      } finally {
        setAutoLoading(false)
        setAutoProgressText('')
      }
    }
  }

  const onAutoCapture = () =>
    startCapture(async path => {
      setAutoPath(path)
      setRecognizedText('')
      setAutoPreview(await loadImagePreview(path))
      await doRecognize(path, selectedDictName)
    })

  const onReRecognize = async () => {
    if (!autoPath) return
    await doRecognize(autoPath, selectedDictName)
  }

  // ── Add dictionary: capture ──
  const onExtractCapture = () =>
    startCapture(async path => {
      setExtractPath(path)
      setSegments([])
      setCharNames({})
      setSelectedSegIdx(0)
      setCapturePreview(await loadImagePreview(path))

      try {
        const analysis = await tauriInvoke<{
          foreground: { r: number; g: number; b: number; dr: number; dg: number; db: number }
        }>('dict_ocr_analyze', { imagePath: path })
        const fg = analysis.foreground
        const hex = '#' + [fg.r, fg.g, fg.b].map(v => v.toString(16).padStart(2, '0')).join('')
        setFgColor(hex)
        setFgDr(fg.dr)
        setFgDg(fg.dg)
        setFgDb(fg.db)

        const b64 = await refreshBinarizedWith(path, fg.r, fg.g, fg.b, fg.dr, fg.dg, fg.db)
        if (b64) setBinarizedPreview(b64)

        try {
          const gaps = await tauriInvoke<{ col_gap: number; row_gap: number; word_gap: number }>(
            'dict_ocr_auto_gaps',
            {
              imagePath: path,
              r: fg.r,
              g: fg.g,
              b: fg.b,
              dr: fg.dr,
              dg: fg.dg,
              db: fg.db,
            },
          )
          setColGap(gaps.col_gap)
          setRowGap(gaps.row_gap)
          setWordGap(gaps.word_gap)
        } catch (_) {
          /* Use default values */
        }
      } catch (e) {
        console.error('[OcrDict] auto-analyze failed:', e)
        setBinarizedPreview(await refreshBinarized(path))
      }
    })

  // ── Add dictionary: extract dots ──
  const onExtractDot = async () => {
    if (!extractPath) return
    setAddLoading(true)
    try {
      const c = hexToRgb(fgColor)
      if (!c) {
        setAddLoading(false)
        return
      }
      const r = await tauriInvoke<{ segments: CharSegment[] }>('dict_ocr_extract', {
        imagePath: extractPath,
        r: c.r,
        g: c.g,
        b: c.b,
        dr: fgDr,
        dg: fgDg,
        db: fgDb,
        minColGap: colGap,
        minRowGap: rowGap,
        wordGap,
      })
      const segs = r.segments
      setSegments(segs)
      setSelectedSegIdx(0)

      // After extraction, auto-match dots with current dictionary, pre-fill existing char names (pixel-level comparison)
      if (segs.length > 0 && dictName.trim()) {
        try {
          const segPayload = segs.map((s, i) => ({
            index: i,
            x: s.x,
            y: s.y,
            width: s.width,
            height: s.height,
            data_hex: s.data_hex,
          }))
          const idResult = await tauriInvoke<{
            matches: { index: number; char: string; sim: number }[]
          }>('dict_ocr_identify_segments', {
            dictName: dictName.trim(),
            segments: segPayload,
          })
          const autoNames: Record<number, string> = {}
          for (const m of idResult.matches || []) {
            if (m.sim >= 1.0 && m.char) {
              autoNames[m.index] = m.char
            }
          }
          setCharNames(autoNames)
        } catch (_) {
          /* Auto-identify prefill failure doesn't affect main flow */
        }
      } else {
        setCharNames({})
      }
    } catch (e) {
      console.error('[OcrDict] onExtractDot:', e)
    }
    setAddLoading(false)
  }

  // ── Font rendering: render text with specified font on Canvas → extract dots ──
  const onFontRender = async () => {
    if (!renderText.trim() || !fontFamily.trim()) return
    setShowFontDialog(false)
    setAddLoading(true)
    try {
      // Render text on canvas at specified font size
      const canvas = document.createElement('canvas')
      const ctx = canvas.getContext('2d')!
      ctx.font = `${fontSize}px "${fontFamily.trim()}"`
      const metrics = ctx.measureText(renderText)
      const textWidth = Math.ceil(metrics.width) + 8
      const textHeight = Math.ceil(fontSize * 1.4)
      canvas.width = textWidth
      canvas.height = textHeight

      // White background, black text (clean binarization conditions)
      ctx.fillStyle = '#FFFFFF'
      ctx.fillRect(0, 0, canvas.width, canvas.height)
      ctx.font = `${fontSize}px "${fontFamily.trim()}"`
      ctx.fillStyle = '#000000'
      ctx.textBaseline = 'top'
      ctx.fillText(renderText, 4, 4)

      // Convert to PNG base64
      const b64 = canvas.toDataURL('image/png').split(',')[1]

      // Temp preview
      setCapturePreview(canvas.toDataURL('image/png'))

      // Save as temp file
      const saved = await tauriInvoke<{ temp_path: string }>('save_temp_image', { imageB64: b64 })

      // Use existing extract command
      const c = hexToRgb(fgColor) || { r: 0, g: 0, b: 0 }
      const result = await tauriInvoke<{ segments: CharSegment[] }>('dict_ocr_extract', {
        imagePath: saved.temp_path,
        r: 0,
        g: 0,
        b: 0, // Black text (foreground fixed to black)
        dr: 0,
        dg: 0,
        db: 0, // Solid color, no tolerance
        minColGap: colGap,
        minRowGap: rowGap,
        wordGap: wordGap,
      })

      setExtractPath(saved.temp_path)
      setSegments(result.segments || [])
      setSelectedSegIdx(0)

      // Auto-fill char names with current dictionary
      if ((result.segments || []).length > 0 && dictName.trim()) {
        try {
          const segPayload = (result.segments || []).map(
            (
              s: { x: number; y: number; width: number; height: number; data_hex: string },
              i: number,
            ) => ({
              index: i,
              x: s.x,
              y: s.y,
              width: s.width,
              height: s.height,
              data_hex: s.data_hex,
            }),
          )
          const idResult = await tauriInvoke<{
            matches: { index: number; char: string; sim: number }[]
          }>('dict_ocr_identify_segments', {
            dictName: dictName.trim(),
            segments: segPayload,
          })
          const nm: Record<number, string> = {}
          for (const m of idResult.matches || []) {
            if (m.sim >= 0.95) nm[m.index] = m.char
          }
          setCharNames(nm)
        } catch (_) {
          /* optional */
        }
      } else {
        setCharNames({})
      }

      setAddLoading(false)
    } catch (e) {
      console.error('[OcrDict] onFontRender:', e)
      setAddLoading(false)
    }
  }

  // ── Add dictionary: save ──
  const onAddToDict = async () => {
    if (!extractPath || !dictName.trim()) return
    setAddLoading(true)
    try {
      const c = hexToRgb(fgColor)
      if (!c) {
        setAddLoading(false)
        return
      }
      for (const [idx, name] of Object.entries(charNames)) {
        if (!name.trim()) continue
        const s = segments[Number(idx)]
        await tauriInvoke('dict_ocr_save_char', {
          imagePath: extractPath,
          dictName: dictName.trim(),
          char: name.trim(),
          x: s.x,
          y: s.y,
          width: s.width,
          height: s.height,
          r: c.r,
          g: c.g,
          b: c.b,
          dr: fgDr,
          dg: fgDg,
          db: fgDb,
        })
      }
      setCharNames({})
      setSegments([])
      loadDicts()
      loadDictList()
      loadExistingDict(dictName.trim())
    } catch (e) {
      console.error('[OcrDict] onAddToDict:', e)
    }
    setAddLoading(false)
  }

  // ── Delete dictionary char (confirm dialog) ──
  const onDeleteChar = async (ch: string, name?: string) => {
    const dn = name ?? dictName
    if (!confirm(`确认从字库「${dn}」中删除字符「${ch}」？`)) return
    try {
      await tauriInvoke('dict_remove_char', { dictName: dn, char: ch })
      await loadExistingDict(dn)
    } catch (e) {
      console.error('[OcrDict] onDeleteChar:', e)
    }
  }

  const onDeleteDict = async (name?: string) => {
    const dn = name ?? dictName
    if (!confirm(`确认删除字库「${dn}」？此操作不可恢复！`)) return
    try {
      await tauriInvoke('dict_delete', { dictName: dn })
      loadDicts()
      loadDictList()
      setDictEntries([])
      setExistingDict({})
    } catch (e) {
      console.error('[OcrDict] onDeleteDict:', e)
    }
  }

  const onResetSession = () => {
    setExtractPath('')
    setCapturePreview('')
    setBinarizedPreview('')
    setSegments([])
    setCharNames({})
    setSelectedSegIdx(0)
  }

  // Check if a character already exists in current dictionary
  const isCharExisting = (ch: string) => ch.trim() && existingDict[ch.trim()]?.length > 0
  // Count how many existing characters this save will involve
  const existingCount = Object.values(charNames).filter(n => n.trim() && isCharExisting(n)).length

  // ── View dictionary ──
  const handleViewDict = async (name?: string) => {
    const dn = name || dictName
    try {
      const r = await tauriInvoke<{ entries: DictEntry[] }>('dict_load', { dictName: dn })
      setDictEntries(r.entries || [])
    } catch (e) {
      console.error('[OcrDict] handleViewDict:', e)
    }
  }

  useEffect(() => {
    if (extractPath) refreshBinarized(extractPath).then(setBinarizedPreview)
  }, [fgColor, fgDr, fgDg, fgDb, extractPath])

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--glass-bg-soft)',
        backdropFilter: 'blur(24px)',
        border: '1px solid var(--glass-4)',
        borderRadius: 20,
        boxShadow: 'var(--shadow-modal)',
        overflow: 'hidden',
        maxHeight: '88vh',
      }}
    >
      <style>{`
.no-spinner::-webkit-outer-spin-button,.no-spinner::-webkit-inner-spin-button{-webkit-appearance:none;margin:0}
.no-spinner{-moz-appearance:textfield}
.extract-layout{display:flex;gap:16px;align-items:flex-start}
.extract-cards{flex:1;min-width:0}
.extract-hint{font-size:var(--fs-micro);color:var(--spark-muted);margin-bottom:8px}
.card-grid{display:flex;flex-wrap:wrap;gap:8px}
.seg-card{display:flex;align-items:center;gap:8px;padding:6px 10px;border-radius:8px;cursor:pointer;border:1px solid var(--glass-2);background:var(--void-deepest);transition:var(--transition-fast);position:relative}
.seg-card:hover{border-color:var(--accent)}
.seg-card--active{border:2px solid var(--accent);background:var(--accent-dim)}
.def-panel{display:flex;flex-direction:column;gap:12px;min-width:160px;background:var(--glass-1);border:1px solid var(--glass-2);border-radius:12px;padding:16px;flex-shrink:0}
.def-label{font-size:var(--fs-micro);font-weight:500;color:var(--spark-tertiary);letter-spacing:var(--ls-wide);text-transform:uppercase}
.def-row{display:flex;align-items:center;gap:8px}
.def-input{flex:1;padding:8px 6px;text-align:center;font-size:20px;font-family:var(--font-mono);border:1px solid var(--glass-2);background:var(--glass-0);color:var(--spark-primary);border-radius:6px;outline:none}
.def-input:focus{border-color:var(--accent)}
.def-size{font-size:var(--fs-caption);color:var(--spark-muted);white-space:nowrap}
.def-char{font-size:var(--fs-caption);color:var(--accent);font-weight:500}
.def-btn{width:100%;text-align:center}
`}</style>
      {/* Title */}
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          padding: '20px 24px 16px',
        }}
      >
        <span
          style={{
            fontSize: 'var(--fs-h2)',
            fontWeight: 'var(--fw-semibold)',
            color: 'var(--spark-primary)',
            letterSpacing: 'var(--ls-tight)',
          }}
        >
          字库文字识别
        </span>
        {onClose && (
          <button
            onClick={onClose}
            style={{
              width: 28,
              height: 28,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              border: '1px solid var(--glass-2)',
              background: 'var(--glass-0)',
              borderRadius: 8,
              color: 'var(--spark-tertiary)',
              cursor: 'pointer',
              fontSize: 16,
              lineHeight: 1,
              transition: 'var(--transition-fast)',
            }}
            onMouseEnter={e => {
              e.currentTarget.style.background = 'var(--glass-3)'
              e.currentTarget.style.color = 'var(--spark-primary)'
            }}
            onMouseLeave={e => {
              e.currentTarget.style.background = 'var(--glass-0)'
              e.currentTarget.style.color = 'var(--spark-tertiary)'
            }}
          >
            ✕
          </button>
        )}
      </div>

      {/* Tabs */}
      <div style={{ display: 'flex', gap: 4, padding: '0 24px' }}>
        {['auto', 'add'].concat(customDicts.slice(0, 2).map(d => `c:${d}`)).map(key => {
          const label = key === 'auto' ? '自动识别' : key === 'add' ? '添加字库' : key.slice(2)
          return (
            <button
              key={key}
              onClick={() => {
                setTab(key)
                if (key.startsWith('c:')) handleViewDict(key.slice(2))
              }}
              style={{
                padding: '7px 16px',
                fontSize: 'var(--fs-caption)',
                border: 'none',
                cursor: 'pointer',
                borderRadius: 8,
                background: tab === key ? 'var(--accent)' : 'var(--glass-1)',
                color: tab === key ? '#fff' : 'var(--spark-tertiary)',
                fontWeight: tab === key ? 500 : 400,
                transition: 'var(--transition-fast)',
              }}
              onMouseEnter={e => {
                if (tab !== key) {
                  e.currentTarget.style.background = 'var(--glass-2)'
                  e.currentTarget.style.color = 'var(--spark-primary)'
                }
              }}
              onMouseLeave={e => {
                if (tab !== key) {
                  e.currentTarget.style.background = 'var(--glass-1)'
                  e.currentTarget.style.color = 'var(--spark-tertiary)'
                }
              }}
            >
              {label}
            </button>
          )
        })}
      </div>

      <div
        style={{
          padding: '20px 24px',
          overflow: 'auto',
          display: 'flex',
          flexDirection: 'column',
          gap: 16,
        }}
      >
        {/* ════════════ Auto recognize ════════════ */}
        {tab === 'auto' && (
          <>
            <div
              style={{
                background: 'var(--glass-1)',
                border: '1px solid var(--glass-2)',
                borderRadius: 12,
                padding: '12px 16px',
              }}
            >
              <div
                style={{
                  fontSize: 'var(--fs-micro)',
                  fontWeight: 500,
                  color: 'var(--spark-tertiary)',
                  letterSpacing: 'var(--ls-wide)',
                  textTransform: 'uppercase',
                  marginBottom: 8,
                }}
              >
                已保存字库
              </div>
              {dictList.length === 0 && (
                <div style={{ fontSize: 13, color: 'var(--spark-tertiary)', padding: '8px 0' }}>
                  暂无字库，请先到「添加字库」选项卡从截图中提取文字制作字库
                </div>
              )}
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                {(() => {
                  const sel = selectedDictName === '__auto__'
                  return (
                    <button
                      key="__auto__"
                      onClick={() => setSelectedDictName('__auto__')}
                      style={{
                        padding: '7px 14px',
                        borderRadius: 8,
                        border: '1px solid',
                        fontSize: 'var(--fs-caption)',
                        cursor: 'pointer',
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: 8,
                        background: sel ? 'var(--accent)' : 'var(--glass-0)',
                        color: sel ? '#fff' : 'var(--spark-primary)',
                        borderColor: sel ? 'var(--accent)' : 'var(--glass-2)',
                        transition: 'var(--transition-fast)',
                        fontWeight: sel ? 600 : 400,
                        outline: 'none',
                        whiteSpace: 'nowrap',
                      }}
                      onMouseEnter={e => {
                        if (!sel) {
                          e.currentTarget.style.background = 'var(--glass-2)'
                          e.currentTarget.style.borderColor = 'var(--glass-3)'
                        }
                      }}
                      onMouseLeave={e => {
                        if (!sel) {
                          e.currentTarget.style.background = 'var(--glass-0)'
                          e.currentTarget.style.borderColor = 'var(--glass-2)'
                        }
                      }}
                    >
                      <svg
                        width="14"
                        height="14"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        style={{ opacity: 0.8, flexShrink: 0 }}
                      >
                        <circle cx="11" cy="11" r="8" />
                        <path d="m21 21-4.35-4.35" />
                      </svg>
                      自动识别
                      <span
                        style={{
                          fontSize: 9,
                          color: sel ? 'rgba(255,255,255,0.7)' : 'var(--spark-muted)',
                        }}
                      >
                        全部字库
                      </span>
                    </button>
                  )
                })()}
                {dictList.map(d => {
                  const sel = d.name === selectedDictName
                  return (
                    <button
                      key={d.name}
                      onClick={() => setSelectedDictName(d.name)}
                      style={{
                        padding: '7px 14px',
                        borderRadius: 8,
                        border: '1px solid',
                        fontSize: 'var(--fs-caption)',
                        cursor: 'pointer',
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: 8,
                        background: sel ? 'var(--accent)' : 'var(--glass-0)',
                        color: sel ? '#fff' : 'var(--spark-primary)',
                        borderColor: sel ? 'var(--accent)' : 'var(--glass-2)',
                        transition: 'var(--transition-fast)',
                        fontWeight: sel ? 600 : 400,
                        outline: 'none',
                        whiteSpace: 'nowrap',
                      }}
                      onMouseEnter={e => {
                        if (!sel) {
                          e.currentTarget.style.background = 'var(--glass-2)'
                          e.currentTarget.style.borderColor = 'var(--glass-3)'
                        }
                      }}
                      onMouseLeave={e => {
                        if (!sel) {
                          e.currentTarget.style.background = 'var(--glass-0)'
                          e.currentTarget.style.borderColor = 'var(--glass-2)'
                        }
                      }}
                    >
                      {d.name}
                      <span
                        style={{
                          fontSize: 9,
                          color: sel ? 'rgba(255,255,255,0.7)' : 'var(--spark-muted)',
                        }}
                      >
                        {Math.round(d.size / 100) / 10}KB
                      </span>
                    </button>
                  )
                })}
              </div>
              <div
                style={{
                  marginTop: 8,
                  fontSize: 'var(--fs-micro)',
                  color: 'var(--spark-muted)',
                  borderTop: '1px solid var(--glass-2)',
                  paddingTop: 8,
                  display: 'flex',
                  alignItems: 'center',
                  gap: 6,
                }}
              >
                <svg
                  width="12"
                  height="12"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  style={{ flexShrink: 0, opacity: 0.6 }}
                >
                  <circle cx="12" cy="12" r="10" />
                  <path d="M12 16v-4" />
                  <path d="M12 8h.01" />
                </svg>
                在「添加字库」选项卡从屏幕上截取文字制作字库
              </div>
            </div>

            <ActionRow>
              <Button variant="primary" onClick={onAutoCapture} disabled={autoLoading}>
                选择识别区域
              </Button>
              <Button variant="default" onClick={onReRecognize} disabled={autoLoading || !autoPath}>
                {autoLoading ? '识别中...' : '重新识别'}
              </Button>
              {selectedDictName !== '__auto__' && (
                <div style={{ display: 'flex', alignItems: 'center', gap: 4, flex: 1 }}>
                  <span style={{ fontSize: 'var(--fs-micro)', color: 'var(--spark-muted)' }}>
                    查找词
                  </span>
                  <input
                    value={searchWord}
                    onChange={e => setSearchWord(e.target.value)}
                    placeholder="留空=识别全部"
                    style={{
                      width: 120,
                      padding: '5px 8px',
                      borderRadius: 6,
                      border: '1px solid var(--glass-2)',
                      background: 'var(--glass-0)',
                      color: 'var(--spark-primary)',
                      fontSize: 'var(--fs-caption)',
                      fontFamily: 'var(--font-mono)',
                      outline: 'none',
                    }}
                    onFocus={e => (e.currentTarget.style.borderColor = 'var(--accent)')}
                    onBlur={e => (e.currentTarget.style.borderColor = 'var(--glass-2)')}
                  />
                </div>
              )}
              {autoPath && (
                <Button
                  variant="ghost"
                  onClick={() => {
                    setAutoPath('')
                    setAutoPreview('')
                    setRecognizedText('')
                    setRecognizedConf(0)
                    setWordMatches([])
                  }}
                  style={{ color: 'var(--spark-tertiary)', fontSize: 'var(--fs-caption)' }}
                >
                  清除
                </Button>
              )}
            </ActionRow>

            {autoLoading && (
              <div
                style={{
                  background: 'var(--glass-1)',
                  border: '1px solid var(--glass-2)',
                  borderRadius: 10,
                  padding: '10px 14px',
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    fontSize: 'var(--fs-caption)',
                    color: 'var(--spark-muted)',
                  }}
                >
                  <span
                    style={{
                      width: 12,
                      height: 12,
                      borderRadius: '50%',
                      border: '2px solid var(--accent)',
                      borderTopColor: 'transparent',
                      animation: 'spin 0.6s linear infinite',
                      flexShrink: 0,
                    }}
                  />
                  {autoProgressText ||
                    (selectedDictName === '__auto__'
                      ? '正在比对已保存字库...'
                      : `正在识别（字库: ${selectedDictName}）...`)}
                </div>
              </div>
            )}

            {autoPreview && (
              <PreviewSection label="选择范围预览区">
                <img
                  src={autoPreview}
                  style={{
                    maxWidth: '100%',
                    maxHeight: 200,
                    objectFit: 'scale-down',
                    imageRendering: 'pixelated',
                  }}
                />
              </PreviewSection>
            )}

            {(recognizedText || autoLoading) && (
              <div
                style={{
                  background:
                    recognizedText &&
                    !recognizedText.includes('未能') &&
                    !recognizedText.includes('失败')
                      ? wordMatches.length > 0
                        ? 'var(--accent-dim)'
                        : 'var(--glass-1)'
                      : 'var(--glass-1)',
                  border:
                    recognizedText &&
                    !recognizedText.includes('未能') &&
                    !recognizedText.includes('失败')
                      ? wordMatches.length > 0
                        ? '2px solid var(--accent)'
                        : '1px solid var(--glass-2)'
                      : '1px solid var(--glass-2)',
                  borderRadius: 12,
                  padding: '12px 16px',
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    justifyContent: 'space-between',
                    alignItems: 'center',
                    marginBottom: 8,
                  }}
                >
                  <div
                    style={{
                      fontSize: 'var(--fs-micro)',
                      color: 'var(--accent)',
                      letterSpacing: 'var(--ls-wide)',
                      textTransform: 'uppercase',
                    }}
                  >
                    {autoLoading
                      ? '识别中...'
                      : wordMatches.length > 0
                        ? `找到 ${wordMatches.length} 处「${searchWord}」`
                        : '识别结果'}
                  </div>
                  {recognizedConf > 0 && !autoLoading && (
                    <span
                      style={{
                        fontSize: 'var(--fs-micro)',
                        fontFamily: 'var(--font-mono)',
                        color: 'var(--spark-tertiary)',
                      }}
                    >
                      {wordMatches.length > 0 ? '相似度' : '置信度'}{' '}
                      {(recognizedConf * 100).toFixed(0)}%
                    </span>
                  )}
                </div>

                {/* 词匹配结果 */}
                {wordMatches.length > 0 && (
                  <div style={{ marginBottom: 10 }}>
                    {wordMatches.map((wm, idx: number) => (
                      <div
                        key={idx}
                        style={{
                          display: 'flex',
                          alignItems: 'center',
                          gap: 10,
                          padding: '6px 10px',
                          borderRadius: 8,
                          background: 'var(--glass-0)',
                          marginBottom: 4,
                          border: '1px solid var(--glass-2)',
                        }}
                      >
                        <span
                          style={{
                            fontSize: 22,
                            fontWeight: 600,
                            color: 'var(--accent)',
                            fontFamily: 'var(--font-mono)',
                            letterSpacing: 1,
                          }}
                        >
                          {wm.word}
                        </span>
                        <span style={{ fontSize: 'var(--fs-micro)', color: 'var(--spark-muted)' }}>
                          相似度 {((wm.sim || 0) * 100).toFixed(0)}%
                        </span>
                        <span
                          style={{ fontSize: 'var(--fs-micro)', color: 'var(--spark-tertiary)' }}
                        >
                          ({wm.x}, {wm.y}) {wm.width}×{wm.height} · {wm.char_count}字
                        </span>
                      </div>
                    ))}
                  </div>
                )}

                <div
                  style={{
                    fontSize: 20,
                    fontWeight: 500,
                    color: 'var(--spark-primary)',
                    fontFamily: 'var(--font-mono)',
                    letterSpacing: 2,
                    minHeight: 24,
                    lineHeight: 1.6,
                  }}
                >
                  {autoLoading ? '正在识别，请稍候…' : recognizedText}
                </div>
                {!autoLoading &&
                  recognizedText &&
                  !recognizedText.includes('失败') &&
                  !recognizedText.includes('未能') && (
                    <div
                      style={{
                        fontSize: 'var(--fs-micro)',
                        color: 'var(--spark-muted)',
                        marginTop: 6,
                      }}
                    >
                      {selectedDictName === '__auto__' && matchedDictName ? (
                        <>
                          匹配字库:{' '}
                          <strong style={{ color: 'var(--accent)' }}>{matchedDictName}</strong>{' '}
                          ·{' '}
                        </>
                      ) : (
                        <>字库: {selectedDictName} · </>
                      )}
                      已匹配 {recognizedText.replace(/\s/g, '').length} 字
                      {wordMatches.length > 0 &&
                        ` · 查找到 ${wordMatches.length} 处「${searchWord}」`}
                    </div>
                  )}
              </div>
            )}

            {!autoPath && !autoLoading && (
              <div
                style={{
                  padding: '40px 20px',
                  textAlign: 'center',
                  borderRadius: 12,
                  border: '1px dashed var(--glass-3)',
                  color: 'var(--spark-muted)',
                  fontSize: 'var(--fs-caption)',
                }}
              >
                选择「自动识别」比对全部已保存字库 或 选指定字库 → 点击「选择识别区域」框选屏幕文字
              </div>
            )}
          </>
        )}

        {/* ════════════ Add dictionary ════════════ */}
        {tab === 'add' && (
          <>
            <ParamCard>
              <ParamGroup label="字体颜色">
                <ColorPicker value={fgColor} onChange={setFgColor} />
                <div style={{ display: 'flex', gap: 4, marginTop: 6 }}>
                  <DeltaInput label="R" value={fgDr} onChange={setFgDr} />
                  <DeltaInput label="G" value={fgDg} onChange={setFgDg} />
                  <DeltaInput label="B" value={fgDb} onChange={setFgDb} />
                </div>
              </ParamGroup>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3,1fr)', gap: 8 }}>
                <ParamGroup label="行间距">
                  <input
                    type="number"
                    min={1}
                    max={50}
                    value={rowGap}
                    onChange={e => setRowGap(Number(e.target.value))}
                    className="settings-input no-spinner"
                    style={{
                      width: '100%',
                      fontFamily: 'var(--font-mono)',
                      fontSize: 'var(--fs-caption)',
                    }}
                  />
                </ParamGroup>
                <ParamGroup label="列间距">
                  <input
                    type="number"
                    min={1}
                    max={50}
                    value={colGap}
                    onChange={e => setColGap(Number(e.target.value))}
                    className="settings-input no-spinner"
                    style={{
                      width: '100%',
                      fontFamily: 'var(--font-mono)',
                      fontSize: 'var(--fs-caption)',
                    }}
                  />
                </ParamGroup>
                <ParamGroup label="字间距">
                  <input
                    type="number"
                    min={1}
                    max={50}
                    value={wordGap}
                    onChange={e => setWordGap(Number(e.target.value))}
                    className="settings-input no-spinner"
                    style={{
                      width: '100%',
                      fontFamily: 'var(--font-mono)',
                      fontSize: 'var(--fs-caption)',
                    }}
                  />
                </ParamGroup>
              </div>
            </ParamCard>

            <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
              <Button variant="primary" onClick={onExtractCapture}>
                截取文字区域
              </Button>
              <Button
                variant="default"
                onClick={onExtractDot}
                disabled={addLoading || !extractPath}
              >
                提取点阵
              </Button>
              <Button
                variant="ghost"
                onClick={() => setShowFontDialog(true)}
                title="高级选项：指定字体直接生成点阵，适合杂色/渐变背景场景"
                style={{ fontSize: 'var(--fs-caption)', color: 'var(--spark-tertiary)' }}
              >
                字体渲染
              </Button>
              {extractPath && (
                <Button
                  variant="ghost"
                  onClick={onResetSession}
                  style={{ color: 'var(--spark-tertiary)', fontSize: 'var(--fs-caption)' }}
                >
                  清空
                </Button>
              )}
              <div style={{ flex: 1 }} />
              <span style={{ fontSize: 'var(--fs-caption)', color: 'var(--spark-tertiary)' }}>
                当前字库
              </span>
              <input
                className="settings-input"
                value={dictName}
                onChange={e => {
                  setDictName(e.target.value)
                  loadExistingDict(e.target.value)
                }}
                style={{ width: 120, fontSize: 'var(--fs-caption)' }}
              />
            </div>

            {(capturePreview || binarizedPreview) && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
                {capturePreview && (
                  <PreviewSection label="选择范围预览区">
                    <img
                      src={capturePreview}
                      style={{
                        maxWidth: '100%',
                        maxHeight: 200,
                        objectFit: 'scale-down',
                        imageRendering: 'pixelated',
                      }}
                    />
                  </PreviewSection>
                )}
                {binarizedPreview && (
                  <PreviewSection label="二值化区域">
                    <img
                      src={binarizedPreview}
                      style={{ maxWidth: '100%', maxHeight: 200, imageRendering: 'pixelated' }}
                    />
                  </PreviewSection>
                )}
              </div>
            )}

            {!extractPath && (
              <div
                style={{
                  padding: '28px 20px',
                  textAlign: 'center',
                  borderRadius: 12,
                  border: '1px dashed var(--glass-3)',
                  color: 'var(--spark-muted)',
                  fontSize: 'var(--fs-caption)',
                }}
              >
                点击「截取文字区域」框选屏幕上的文字，然后「提取点阵」分割出单个字符，命名后保存到字库
              </div>
            )}

            {segments.length > 0 && (
              <PreviewSection label="提取点阵">
                <div className="extract-layout">
                  <div className="extract-cards">
                    <div className="extract-hint">点击点阵选中字符，在右侧输入框命名后保存</div>
                    <div className="card-grid">
                      {segments.map((s, i) => {
                        const named = charNames[i]?.trim()
                        const exists = named && isCharExisting(named)
                        return (
                          <div
                            key={i}
                            onClick={() => setSelectedSegIdx(i)}
                            className={`seg-card ${i === selectedSegIdx ? 'seg-card--active' : ''}`}
                            style={exists ? { borderColor: 'var(--warning)' } : {}}
                          >
                            <PixelGrid dataHex={s.data_hex} w={s.width} h={s.height} />
                            {named && (
                              <span
                                style={{
                                  position: 'absolute',
                                  bottom: -2,
                                  right: -2,
                                  fontSize: 9,
                                  padding: '1px 4px',
                                  borderRadius: 4,
                                  background: exists ? 'var(--warning)' : 'var(--accent)',
                                  color: '#000',
                                  lineHeight: '1.3',
                                }}
                              >
                                {exists ? '已有' : named}
                              </span>
                            )}
                          </div>
                        )
                      })}
                    </div>
                  </div>
                  <div className="def-panel">
                    <span className="def-label">定义字符</span>
                    <div className="def-row">
                      <input
                        key={selectedSegIdx}
                        value={charNames[selectedSegIdx] || ''}
                        onChange={e =>
                          setCharNames(p => ({ ...p, [selectedSegIdx]: e.target.value }))
                        }
                        placeholder="输入字符"
                        autoFocus
                        className="def-input"
                        onBlur={e => {
                          if (!e.currentTarget.value.trim())
                            e.currentTarget.style.borderColor = 'var(--glass-2)'
                        }}
                      />
                      <span className="def-size">
                        {segments[selectedSegIdx]?.width}x{segments[selectedSegIdx]?.height}
                      </span>
                      {(() => {
                        const n = charNames[selectedSegIdx]?.trim()
                        if (!n) return null
                        const existing = existingDict[n]
                        if (!existing?.length) return <span className="def-char">{n}</span>
                        return (
                          <span style={{ color: 'var(--warning)', fontSize: 'var(--fs-micro)' }}>
                            已有{existing.length}个
                          </span>
                        )
                      })()}
                    </div>
                    {/* Show existing templates for comparison */}
                    {(() => {
                      const n = charNames[selectedSegIdx]?.trim()
                      if (!n) return null
                      const existing = existingDict[n]
                      if (!existing?.length) return null
                      return (
                        <div
                          style={{
                            borderTop: '1px solid var(--glass-2)',
                            paddingTop: 8,
                            marginTop: -4,
                          }}
                        >
                          <div
                            style={{
                              fontSize: 'var(--fs-micro)',
                              color: 'var(--spark-muted)',
                              marginBottom: 4,
                            }}
                          >
                            字库已有 <strong style={{ color: 'var(--warning)' }}>{n}</strong> 的模板{' '}
                            {existing.length} 个：
                          </div>
                          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                            {existing.map((e, ei) => (
                              <div
                                key={ei}
                                style={{
                                  padding: 2,
                                  border: '1px solid var(--glass-2)',
                                  borderRadius: 6,
                                  background: 'var(--glass-0)',
                                  display: 'inline-flex',
                                  flexDirection: 'column',
                                  alignItems: 'center',
                                  gap: 4,
                                }}
                              >
                                <PixelGrid dataHex={e.data_hex} w={e.width} h={e.height} />
                                <span style={{ fontSize: 8, color: 'var(--spark-muted)' }}>
                                  {e.width}x{e.height}
                                </span>
                              </div>
                            ))}
                          </div>
                          <div style={{ fontSize: 10, color: 'var(--spark-muted)', marginTop: 4 }}>
                            再次保存将添加一个新模板变体，匹配时自动选最优
                          </div>
                        </div>
                      )
                    })()}
                    <Button
                      variant="primary"
                      className="def-btn"
                      onClick={onAddToDict}
                      disabled={addLoading || !Object.values(charNames).some(v => v.trim())}
                    >
                      {addLoading
                        ? '保存中...'
                        : existingCount > 0
                          ? `添加 ${Object.values(charNames).filter(n => n.trim() && !isCharExisting(n)).length} 个 · 补充 ${existingCount} 个`
                          : '添加到当前字库'}
                    </Button>
                  </div>
                </div>
              </PreviewSection>
            )}
          </>
        )}

        {/* ════════════ View dictionary ════════════ */}
        {tab.startsWith('c:') && (
          <>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <span style={{ fontSize: 'var(--fs-caption)', color: 'var(--spark-tertiary)' }}>
                字库: {tab.slice(2)}
              </span>
              <Button
                variant="default"
                onClick={() => {
                  setDictName(tab.slice(2))
                  setTab('add')
                }}
              >
                添加字符
              </Button>
              <Button
                variant="default"
                onClick={() => {
                  loadExistingDict(tab.slice(2))
                  handleViewDict(tab.slice(2))
                }}
              >
                刷新
              </Button>
              <div style={{ flex: 1 }} />
              <button
                className="btn"
                style={{ fontSize: 'var(--fs-micro)', padding: '3px 10px' }}
                onClick={() => onDeleteDict(tab.slice(2))}
              >
                ✕ 清空字库
              </button>
            </div>
            {/* ── Existing chars list (with ❌ delete) ── */}
            {Object.keys(existingDict).length > 0 && (
              <div
                style={{
                  background: 'var(--glass-1)',
                  border: '1px solid var(--glass-2)',
                  borderRadius: 12,
                  padding: '10px 14px',
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    justifyContent: 'space-between',
                    alignItems: 'center',
                    marginBottom: 8,
                  }}
                >
                  <span
                    style={{
                      fontSize: 'var(--fs-micro)',
                      fontWeight: 500,
                      color: 'var(--spark-tertiary)',
                      letterSpacing: 'var(--ls-wide)',
                      textTransform: 'uppercase',
                    }}
                  >
                    字库已有字符（{Object.keys(existingDict).length} 个）
                  </span>
                </div>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                  {Object.entries(existingDict).map(([ch, templates]) => {
                    const dn = tab.slice(2)
                    return (
                      <div
                        key={ch}
                        style={{
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: 4,
                          padding: '4px 6px 4px 10px',
                          borderRadius: 6,
                          border: '1px solid var(--glass-2)',
                          background: 'var(--glass-0)',
                          fontSize: 'var(--fs-caption)',
                          fontFamily: 'var(--font-mono)',
                        }}
                      >
                        <span style={{ color: 'var(--spark-primary)' }}>{ch}</span>
                        <span style={{ fontSize: 9, color: 'var(--spark-muted)' }}>
                          {templates.length}变体
                        </span>
                        <button
                          onClick={() => onDeleteChar(ch, dn)}
                          style={{
                            width: 18,
                            height: 18,
                            display: 'inline-flex',
                            alignItems: 'center',
                            justifyContent: 'center',
                            border: 'none',
                            borderRadius: 4,
                            cursor: 'pointer',
                            fontSize: 11,
                            lineHeight: 1,
                            background: 'transparent',
                            color: 'var(--spark-tertiary)',
                            padding: 0,
                            margin: 0,
                            transition: 'var(--transition-fast)',
                          }}
                          onMouseEnter={e => {
                            e.currentTarget.style.background = 'rgba(255,80,80,0.2)'
                            e.currentTarget.style.color = '#ff5050'
                          }}
                          onMouseLeave={e => {
                            e.currentTarget.style.background = 'transparent'
                            e.currentTarget.style.color = 'var(--spark-tertiary)'
                          }}
                          title={`删除「${ch}」`}
                        >
                          ✕
                        </button>
                      </div>
                    )
                  })}
                </div>
              </div>
            )}
            {Object.keys(existingDict).length === 0 && (
              <div
                style={{
                  padding: '28px 20px',
                  textAlign: 'center',
                  borderRadius: 12,
                  border: '1px dashed var(--glass-3)',
                  color: 'var(--spark-muted)',
                  fontSize: 'var(--fs-caption)',
                }}
              >
                字库为空，点击「添加字符」从屏幕截取文字录入
              </div>
            )}
          </>
        )}

        {/* ── Font render dialog ── */}
        {showFontDialog && (
          <div className="modal-overlay visible" onClick={() => setShowFontDialog(false)}>
            <div
              className="modal-content"
              style={{ maxWidth: 460, width: '90%' }}
              onClick={e => e.stopPropagation()}
            >
              <div className="modal-header">
                <span className="modal-title">字体渲染生成点阵</span>
                <IconButton variant="ghost" label="关闭" onClick={() => setShowFontDialog(false)}>
                  <svg
                    width="16"
                    height="16"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                  >
                    <path d="M18 6L6 18M6 6l12 12" />
                  </svg>
                </IconButton>
              </div>
              <div
                className="modal-body"
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 16,
                  padding: '16px 24px 24px',
                }}
              >
                <div>
                  <label
                    style={{
                      fontSize: 'var(--fs-caption)',
                      fontWeight: 500,
                      color: 'var(--spark-tertiary)',
                      marginBottom: 6,
                      display: 'block',
                    }}
                  >
                    字体名称 (font-family)
                  </label>
                  <input
                    className="settings-input"
                    value={fontFamily}
                    onChange={e => setFontFamily(e.target.value)}
                    placeholder="例如: Courier, sans-serif, system-ui, SimSun, 微软雅黑"
                    style={{ width: '100%', fontSize: 'var(--fs-body)' }}
                  />
                  <div
                    style={{
                      fontSize: 'var(--fs-micro)',
                      color: 'var(--spark-muted)',
                      marginTop: 4,
                    }}
                  >
                    输入系统已安装的字体名称，多个回退用逗号分隔
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 12, alignItems: 'flex-end' }}>
                  <div style={{ flex: 1 }}>
                    <label
                      style={{
                        fontSize: 'var(--fs-caption)',
                        fontWeight: 500,
                        color: 'var(--spark-tertiary)',
                        marginBottom: 6,
                        display: 'block',
                      }}
                    >
                      渲染文字
                    </label>
                    <input
                      className="settings-input"
                      value={renderText}
                      onChange={e => setRenderText(e.target.value)}
                      placeholder="输入字"
                      maxLength={1}
                      style={{
                        width: '100%',
                        fontSize: fontSize,
                        fontFamily: fontFamily || 'var(--font-mono)',
                      }}
                    />
                  </div>
                  <div style={{ width: 80 }}>
                    <label
                      style={{
                        fontSize: 'var(--fs-caption)',
                        fontWeight: 500,
                        color: 'var(--spark-tertiary)',
                        marginBottom: 6,
                        display: 'block',
                      }}
                    >
                      字号 (px)
                    </label>
                    <input
                      type="number"
                      min={6}
                      max={200}
                      value={fontSize}
                      onChange={e =>
                        setFontSize(Math.max(6, Math.min(200, Number(e.target.value) || 14)))
                      }
                      className="settings-input no-spinner"
                      style={{
                        width: '100%',
                        fontFamily: 'var(--font-mono)',
                        fontSize: 'var(--fs-mono)',
                        textAlign: 'center',
                      }}
                    />
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', marginTop: 4 }}>
                  <Button variant="default" onClick={() => setShowFontDialog(false)}>
                    取消
                  </Button>
                  <Button
                    variant="primary"
                    onClick={onFontRender}
                    disabled={!renderText.trim() || !fontFamily.trim() || addLoading}
                  >
                    {addLoading ? '处理中...' : '提取点阵'}
                  </Button>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

// ── Sub-components ──

function ParamCard({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        background: 'var(--glass-1)',
        border: '1px solid var(--glass-2)',
        borderRadius: 12,
        backdropFilter: 'blur(8px)',
        padding: 16,
        display: 'grid',
        gridTemplateColumns: '1fr 1fr 1fr',
        gap: 16,
      }}
    >
      {children}
    </div>
  )
}

function ParamGroup({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <div
        style={{
          fontSize: 'var(--fs-micro)',
          fontWeight: 500,
          color: 'var(--spark-tertiary)',
          marginBottom: 8,
          letterSpacing: 'var(--ls-wide)',
          textTransform: 'uppercase',
        }}
      >
        {label}
      </div>
      {children}
    </div>
  )
}

function ActionRow({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
      {children}
    </div>
  )
}

function PreviewSection({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div
      style={{
        background: 'var(--glass-1)',
        border: '1px solid var(--glass-2)',
        borderRadius: 12,
        padding: 12,
      }}
    >
      <div
        style={{
          fontSize: 'var(--fs-micro)',
          fontWeight: 500,
          color: 'var(--spark-tertiary)',
          marginBottom: 8,
          letterSpacing: 'var(--ls-wide)',
          textTransform: 'uppercase',
        }}
      >
        {label}
      </div>
      {children}
    </div>
  )
}

function ColorPicker({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
      <div
        style={{
          position: 'relative',
          width: 34,
          height: 34,
          borderRadius: 8,
          border: '1px solid var(--glass-3)',
          overflow: 'hidden',
          flexShrink: 0,
        }}
      >
        <input
          type="color"
          value={value}
          onChange={e => onChange(e.target.value)}
          style={{
            position: 'absolute',
            top: -4,
            left: -4,
            width: 42,
            height: 42,
            padding: 0,
            border: 'none',
            cursor: 'pointer',
          }}
        />
      </div>
      <input
        className="settings-input"
        value={value}
        onChange={e => onChange(e.target.value)}
        style={{ width: 96, fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-mono)' }}
      />
    </div>
  )
}

function DeltaInput({
  label,
  value,
  onChange,
}: {
  label: string
  value: number
  onChange: (v: number) => void
}) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
      <span style={{ fontSize: 'var(--fs-micro)', color: 'var(--spark-muted)', width: 10 }}>
        {label}
      </span>
      <input
        type="number"
        min={0}
        max={127}
        value={value}
        onChange={e => onChange(Number(e.target.value))}
        className="settings-input no-spinner"
        style={{
          width: 44,
          fontSize: 'var(--fs-micro)',
          fontFamily: 'var(--font-mono)',
          padding: '4px 6px',
        }}
      />
    </div>
  )
}

function PixelGrid({ dataHex, w, h }: { dataHex: string; w: number; h: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const maxDim = Math.max(w, h, 1)
  const px = Math.min(14, Math.max(3, Math.floor(200 / maxDim)))
  const gap = 1
  const cell = px - gap
  useEffect(() => {
    const el = canvasRef.current
    if (!el || !w || !h) return
    el.width = w * px
    el.height = h * px
    const ctx = el.getContext('2d')
    if (!ctx) return
    ctx.fillStyle = '#1a1a2e'
    ctx.fillRect(0, 0, el.width, el.height)
    const cols = Math.ceil(w / 8)
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const byteIdx = y * cols + (x >> 3)
        const bit = 7 - (x & 7)
        const on =
          byteIdx * 2 + 2 <= dataHex.length
            ? (parseInt(dataHex.substring(byteIdx * 2, byteIdx * 2 + 2), 16) >> bit) & 1
            : 0
        if (on) {
          ctx.fillStyle = '#e0e0ff'
          ctx.fillRect(x * px + gap, y * px + gap, cell, cell)
        }
      }
    }
  }, [dataHex, w, h, px])
  return (
    <canvas
      ref={canvasRef}
      style={{
        width: Math.max(w * px, 20),
        height: Math.max(h * px, 20),
        imageRendering: 'pixelated',
        display: 'block',
      }}
    />
  )
}

function hexToRgb(hex: string): ColorSpec | null {
  const m = hex.replace('#', '').match(/^([0-9a-fA-F]{2})([0-9a-fA-F]{2})([0-9a-fA-F]{2})$/)
  if (!m) return null
  return {
    r: parseInt(m[1], 16),
    g: parseInt(m[2], 16),
    b: parseInt(m[3], 16),
    dr: 30,
    dg: 30,
    db: 30,
  }
}