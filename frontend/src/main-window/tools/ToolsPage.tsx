// ToolsPage — 内置工具页（两级导航）
//
// 设计定位（大王定调）：工具是内部机制（invoke 命令），不是 agent 工具调用项。
// 页面采用「分类选择器 + 能力卡片 → 详情页」两级结构，降低用户认知负担：
//   Level 1: 分类 tab（全部 / 图片 / 视频 / 文档 / 音频 / PDF）+ 能力卡片网格（每卡片=一种能力）
//   Level 2: 点击卡片 → 详情页，分「文件区」（选择输入/输出 + 预览）与「设置区」（参数）
//
// 命令均在 src-tauri/src/commands/tools/*（内部机制，不进 get_tools/execute_tool）。

import { useState, useMemo, useEffect, type ReactNode } from 'react'
import { open, save } from '@tauri-apps/plugin-dialog'
import {
  IconFile,
  IconImage,
  IconPlay,
  IconRefresh,
  IconFolder,
  IconLayers,
  IconBox,
  IconType,
  IconCrop,
  IconEye,
  IconMic,
  IconGrid,
  IconArrowLeft,
  IconChevronRight,
  IconX,
  IconUpload,
  IconBook,
} from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { useLanguage } from '../../locales'
import {
  pdfMerge,
  pdfCompress,
  pdfExtractText,
  pdfImagesToPdf,
  pdfExtractPages,
  pdfPageCount,
  pdfRotate,
  imageCompress,
  imageConvert,
  imageInfo,
  imageResize,
  imageStitch,
  imageCompressBatch,
  imageConvertBatch,
  imageResizeBatch,
  videoCompress,
  videoInfo,
  videoExtractAudio,
  videoExtractFrames,
  videoToGif,
  videoCut,
  audioConvert,
  voiceClone,
  docExtractText,
  revealPath,
} from '../lib/api'
import { FilePreviewContent } from '../chat/PreviewOverlay'
import './tools.css'

type Category = 'all' | 'image' | 'video' | 'doc' | 'audio' | 'pdf'

interface DialogFilter {
  name: string
  extensions: string[]
}

const PDF_FILTERS: DialogFilter[] = [{ name: 'PDF', extensions: ['pdf'] }]
const IMAGE_FILTERS: DialogFilter[] = [
  { name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'bmp', 'gif', 'webp'] },
]
const GIF_FILTERS: DialogFilter[] = [{ name: 'GIF', extensions: ['gif'] }]
const AUDIO_FILTERS: DialogFilter[] = [
  { name: 'Audio', extensions: ['mp3', 'wav', 'm4a', 'flac', 'ogg', 'aac'] },
]
const DOC_FILTERS: DialogFilter[] = [
  { name: 'Documents', extensions: ['docx', 'pptx', 'xls', 'ods', 'odt', 'odp', 'pdf'] },
]
const VIDEO_FILTERS: DialogFilter[] = [
  { name: 'Videos', extensions: ['mp4', 'webm', 'mov', 'mkv', 'avi', 'flv', 'ts', 'm4v'] },
]

/** 设置区字段配置（驱动表单渲染，避免 12 份重复表单代码） */
interface FieldDef {
  key: string
  labelKey: string
  kind: 'number' | 'select' | 'text'
  options?: { value: string; labelKey: string }[]
  placeholder?: string
  default?: string
}

/** 能力定义：一张卡片 = 一种能力 = 一个内部机制命令 */
interface AbilityDef {
  id: string
  category: Exclude<Category, 'all'>
  icon: ReactNode
  titleKey: string
  descKey: string
  /** 合并类支持多文件输入 */
  multiInput?: boolean
  /** 是否需要输出路径（信息类能力不需要） */
  needsOutput?: boolean
  /** 输出为目录（抽帧） */
  outputIsDir?: boolean
  filters: DialogFilter[]
  /** 输出默认扩展名（save 对话框 defaultPath 用） */
  outputExt: string
  fields: FieldDef[]
  run: (v: { inputs: string[]; output: string; values: Record<string, string> }) => Promise<unknown>
  /** 从执行结果提取可预览/定位路径（默认取 res.output） */
  resultPath?: (res: Record<string, unknown>) => string | null
  /** 从执行结果提取文本内容（在预览区直接呈现，可复制） */
  resultText?: (res: Record<string, unknown>) => string | null
  /** 布局模式：split=左右分栏(默认) / stacked=单栏堆叠 */
  layout?: 'split' | 'stacked'
  /** 专用文件卡渲染（替代默认 chip 列表） */
  renderFileCard?: (props: {
    inputs: string[]
    onRemove: (path: string) => void
    onMove?: (from: number, to: number) => void
  }) => ReactNode
  /** 专用设置面板渲染（layout='stacked' 时替代 fields 表单） */
  renderSettings?: (props: {
    values: Record<string, string>
    onChange: (key: string, value: string) => void
    inputs: string[]
    result: Record<string, unknown> | null
    busy: boolean
  }) => ReactNode
  /** 专用结果渲染（替代默认 JSON） */
  renderResult?: (props: { result: Record<string, unknown> }) => ReactNode
}

/* ───────────── PDF 压缩专用组件 ───────────── */

function PdfCompressSettings({
  values,
  onChange,
  inputs,
  result,
  busy,
}: {
  values: Record<string, string>
  onChange: (key: string, value: string) => void
  inputs: string[]
  result: Record<string, unknown> | null
  busy: boolean
}) {
  const { t } = useLanguage()
  const mode = values.compressMode || 'smart'

  return (
    <>
      {/* 压缩模式 */}
      <div className="tools-zone-title">{t('tools.compressMode')}</div>
      <div className="tools-card-selector">
        {(['smart', 'target'] as const).map(m => (
          <button
            key={m}
            type="button"
            className={`tools-mode-card ${mode === m ? 'tools-mode-card--active' : ''}`}
            onClick={() => onChange('compressMode', m)}
          >
            <span className="tools-mode-card-icon">{m === 'smart' ? 'S' : 'T'}</span>
            <span className="tools-mode-card-title">
              {t(`tools.compress${m === 'smart' ? 'Smart' : 'Target'}`)}
            </span>
            <span className="tools-mode-card-desc">
              {t(`tools.compress${m === 'smart' ? 'SmartDesc' : 'TargetDesc'}`)}
            </span>
          </button>
        ))}
      </div>

      {/* 质量档位（仅智能压缩） */}
      {mode === 'smart' && (
        <>
          <div className="tools-zone-title" style={{ marginTop: 12 }}>
            {t('tools.qualityLevel')}
          </div>
          <div className="tools-quality-grid">
            {(
              [
                { value: 'low', label: t('tools.qualityLow'), pct: '30%' },
                { value: 'medium', label: t('tools.qualityMedium'), pct: '55%' },
                { value: 'high', label: t('tools.qualityHigh'), pct: '80%' },
              ] as const
            ).map(p => (
              <button
                key={p.value}
                type="button"
                className={`tools-quality-card ${(values.quality || 'medium') === p.value ? 'tools-quality-card--active' : ''}`}
                onClick={() => onChange('quality', p.value)}
              >
                <div className="tools-quality-radio">
                  {(values.quality || 'medium') === p.value ? '●' : '○'}
                </div>
                <div className="tools-quality-label">
                  {p.label}
                  {p.value === 'medium' && <span className="tools-quality-badge">RECOMMENDED</span>}
                </div>
                <div className="tools-quality-bar">
                  <div className="tools-quality-bar-fill" style={{ width: p.pct }} />
                </div>
              </button>
            ))}
          </div>
        </>
      )}

      {/* 目标大小（仅目标大小模式） */}
      {mode === 'target' && (
        <div className="tools-field" style={{ marginTop: 12 }}>
          <div className="tools-field-label">{t('tools.maxFileSize')}</div>
          <div className="tools-field-row">
            <input
              className="tools-input"
              type="number"
              min="0.1"
              step="0.1"
              value={values.targetSize ?? ''}
              onChange={e => onChange('targetSize', e.target.value)}
              placeholder="2"
            />
            <span className="tools-field-suffix">MB</span>
          </div>
        </div>
      )}

      {/* 执行后压缩结果 */}
      {result && typeof result.size_before === 'number' && <PdfCompressResult result={result} />}
    </>
  )
}

function PdfCompressResult({ result }: { result: Record<string, unknown> }) {
  const { t } = useLanguage()
  const r = result as { size_before: number; size_after: number; saved_bytes: number }
  const ratio = r.size_before > 0 ? Math.round((1 - r.size_after / r.size_before) * 100) : 0
  const maxBar = 180

  return (
    <div className="tools-compress-result" style={{ marginTop: 12 }}>
      <div className="tools-compress-result-title">{t('tools.compressDone')}</div>
      <div className="tools-compare-row">
        <span className="tools-compare-label">{t('tools.compressBefore')}</span>
        <div className="tools-compare-bar" style={{ width: maxBar }}>
          <div
            className="tools-compare-bar-fill tools-compare-bar-fill--before"
            style={{ width: '100%' }}
          />
        </div>
        <span className="tools-compare-size">{formatBytes(r.size_before)}</span>
      </div>
      <div className="tools-compare-row">
        <span className="tools-compare-label">{t('tools.compressAfter')}</span>
        <div className="tools-compare-bar" style={{ width: maxBar }}>
          <div
            className="tools-compare-bar-fill tools-compare-bar-fill--after"
            style={{ width: `${100 - ratio}%` }}
          />
        </div>
        <span className="tools-compare-size">{formatBytes(r.size_after)}</span>
      </div>
      <div className="tools-compress-saved">
        {t('tools.compressSaved')} {ratio}% ({formatBytes(r.saved_bytes)})
      </div>
    </div>
  )
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`
}

function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = s % 60
  const pad = (n: number) => String(n).padStart(2, '0')
  return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`
}

/** 信息类结果渲染：把后端字段转成易读的 key-value 行（字节/时长/路径做格式化） */
function InfoResult({ result }: { result: Record<string, unknown> }) {
  const pretty = (v: unknown, k: string): string => {
    if (k === 'size_bytes' && typeof v === 'number') return formatBytes(v)
    if (k === 'duration_seconds' && typeof v === 'number') return formatDuration(v)
    if (v === null || v === undefined) return '—'
    if (typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean') return String(v)
    return JSON.stringify(v)
  }
  const rows = Object.entries(result).filter(([k]) => !k.startsWith('_'))
  return (
    <div className="tools-info-result">
      {rows.map(([k, v]) => (
        <div className="tools-info-row" key={k}>
          <span className="tools-info-key">{k.replace(/_/g, ' ')}</span>
          <span
            className="tools-info-value"
            title={k === 'path' && typeof v === 'string' ? v : undefined}
          >
            {pretty(v, k)}
          </span>
        </div>
      ))}
    </div>
  )
}

/* ───────────── 多文件列表卡片（带排序） ───────────── */

function FileListCard({
  inputs,
  onRemove,
  onMove,
}: {
  inputs: string[]
  onRemove: (path: string) => void
  onMove?: (from: number, to: number) => void
}) {
  const { t } = useLanguage()
  return (
    <div className="tools-file-cards">
      {inputs.map((p, i) => (
        <div key={p} className="tools-file-card">
          <div className="tools-file-card-icon">FILE</div>
          <div className="tools-file-card-info">
            <div className="tools-file-card-name">{fileLabel(p)}</div>
            <div className="tools-file-card-meta">
              {i + 1}/{inputs.length}
            </div>
          </div>
          {onMove && inputs.length > 1 && (
            <div className="tools-file-card-reorder">
              <button
                type="button"
                className="tools-file-card-btn"
                disabled={i === 0}
                onClick={() => onMove(i, i - 1)}
                title={t('tools.moveUp')}
              >
                ↑
              </button>
              <button
                type="button"
                className="tools-file-card-btn"
                disabled={i === inputs.length - 1}
                onClick={() => onMove(i, i + 1)}
                title={t('tools.moveDown')}
              >
                ↓
              </button>
            </div>
          )}
          <button
            type="button"
            className="tools-file-card-remove"
            onClick={() => onRemove(p)}
            title="移除"
          >
            <IconX size={14} />
          </button>
        </div>
      ))}
    </div>
  )
}

/* ───────────── 质量预设卡片（通用） ───────────── */

function QualityPresetCards({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const { t } = useLanguage()
  const presets = [
    { value: 'low', label: t('tools.qualityLow'), pct: '30%' },
    { value: 'medium', label: t('tools.qualityMedium'), pct: '55%' },
    { value: 'high', label: t('tools.qualityHigh'), pct: '80%' },
  ]
  return (
    <div className="tools-quality-grid">
      {presets.map(p => (
        <button
          key={p.value}
          type="button"
          className={`tools-quality-card ${value === p.value ? 'tools-quality-card--active' : ''}`}
          onClick={() => onChange(p.value)}
        >
          <div className="tools-quality-radio">{value === p.value ? '●' : '○'}</div>
          <div className="tools-quality-label">
            {p.label}
            {p.value === 'medium' && <span className="tools-quality-badge">RECOMMENDED</span>}
          </div>
          <div className="tools-quality-bar">
            <div className="tools-quality-bar-fill" style={{ width: p.pct }} />
          </div>
        </button>
      ))}
    </div>
  )
}

/* ───────────── 方向预设卡片（拼接方向） ───────────── */

function DirectionPresetCards({
  value,
  onChange,
}: {
  value: string
  onChange: (v: string) => void
}) {
  const { t } = useLanguage()
  const dirs = [
    { value: 'horizontal', icon: '↔', label: t('tools.stitchHorizontal') },
    { value: 'vertical', icon: '↕', label: t('tools.stitchVertical') },
  ]
  return (
    <div className="tools-card-selector">
      {dirs.map(d => (
        <button
          key={d.value}
          type="button"
          className={`tools-mode-card ${value === d.value ? 'tools-mode-card--active' : ''}`}
          onClick={() => onChange(d.value)}
        >
          <span className="tools-mode-card-icon">{d.icon}</span>
          <span className="tools-mode-card-title">{d.label}</span>
        </button>
      ))}
    </div>
  )
}

/* ───────────── 格式选择卡片（通用） ───────────── */

function FormatSelectCards({
  value,
  options,
  onChange,
}: {
  value: string
  options: { value: string; label: string; icon?: string }[]
  onChange: (v: string) => void
}) {
  return (
    <div className="tools-card-selector">
      {options.map(o => (
        <button
          key={o.value}
          type="button"
          className={`tools-mode-card ${value === o.value ? 'tools-mode-card--active' : ''}`}
          onClick={() => onChange(o.value)}
        >
          {o.icon && <span className="tools-mode-card-icon">{o.icon}</span>}
          <span className="tools-mode-card-title">{o.label}</span>
        </button>
      ))}
    </div>
  )
}

/* ───────────── 批量格式选择设置 ───────────── */

function BatchFormatSettings({
  values,
  onChange,
}: {
  values: Record<string, string>
  onChange: (k: string, v: string) => void
}) {
  const { t } = useLanguage()
  return (
    <div className="tools-stacked-section">
      <div className="tools-zone-title">{t('tools.batchFormat')}</div>
      <FormatSelectCards
        value={values.format || 'jpg'}
        options={[
          { value: 'png', label: t('tools.fmtPng'), icon: 'PNG' },
          { value: 'jpg', label: t('tools.fmtJpg'), icon: 'JPG' },
          { value: 'webp', label: t('tools.fmtWebp'), icon: 'WEBP' },
          { value: 'bmp', label: t('tools.fmtBmp'), icon: 'BMP' },
          { value: 'gif', label: t('tools.fmtGif'), icon: 'GIF' },
        ]}
        onChange={v => onChange('format', v)}
      />
    </div>
  )
}

/* ───────────── 视频质量卡片 ───────────── */

function VideoQualityCards({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const { t } = useLanguage()
  const presets = [
    { value: 'low', label: t('tools.qualityLow'), pct: '30%' },
    { value: 'medium', label: t('tools.qualityMedium'), pct: '55%' },
    { value: 'high', label: t('tools.qualityHigh'), pct: '80%' },
  ]
  return (
    <div className="tools-quality-grid">
      {presets.map(p => (
        <button
          key={p.value}
          type="button"
          className={`tools-quality-card ${value === p.value ? 'tools-quality-card--active' : ''}`}
          onClick={() => onChange(p.value)}
        >
          <div className="tools-quality-radio">{value === p.value ? '●' : '○'}</div>
          <div className="tools-quality-label">
            {p.label}
            {p.value === 'medium' && <span className="tools-quality-badge">RECOMMENDED</span>}
          </div>
          <div className="tools-quality-bar">
            <div className="tools-quality-bar-fill" style={{ width: p.pct }} />
          </div>
        </button>
      ))}
    </div>
  )
}

/* ───────────── 时间范围字段（视频裁剪） ───────────── */

function TimeRangeFields({
  values,
  onChange,
}: {
  values: Record<string, string>
  onChange: (key: string, value: string) => void
}) {
  const { t } = useLanguage()
  return (
    <>
      <div className="tools-field">
        <div className="tools-field-label">{t('tools.cutStart')}</div>
        <div className="tools-field-row">
          <input
            className="tools-input"
            type="number"
            min="0"
            step="0.1"
            value={values.start ?? ''}
            onChange={e => onChange('start', e.target.value)}
            placeholder="0"
          />
          <span className="tools-field-suffix">s</span>
        </div>
      </div>
      <div className="tools-field">
        <div className="tools-field-label">{t('tools.cutEnd')}</div>
        <div className="tools-field-row">
          <input
            className="tools-input"
            type="number"
            min="0"
            step="0.1"
            value={values.end ?? ''}
            onChange={e => onChange('end', e.target.value)}
          />
          <span className="tools-field-suffix">s</span>
        </div>
      </div>
    </>
  )
}

/* ───────────── 旋转角度卡片 ───────────── */

function RotationAngleCards({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const { t } = useLanguage()
  const angles = [
    { value: '90', label: t('tools.rotate90'), icon: '↻' },
    { value: '180', label: t('tools.rotate180'), icon: '↻↻' },
    { value: '270', label: t('tools.rotate270'), icon: '↺' },
  ]
  return (
    <div className="tools-card-selector">
      {angles.map(a => (
        <button
          key={a.value}
          type="button"
          className={`tools-mode-card ${value === a.value ? 'tools-mode-card--active' : ''}`}
          onClick={() => onChange(a.value)}
        >
          <span className="tools-mode-card-icon">{a.icon}</span>
          <span className="tools-mode-card-title">{a.label}</span>
        </button>
      ))}
    </div>
  )
}

/* ───────────── 通用预设卡片组件 ───────────── */

function ImageSizePresetCards({
  value,
  onChange,
}: {
  value: string
  onChange: (v: string) => void
}) {
  const { t } = useLanguage()
  const presets = [
    { value: '480', label: '480', desc: t('tools.sizeSmall') },
    { value: '720', label: '720', desc: t('tools.sizeMedium') },
    { value: '1080', label: '1080', desc: t('tools.sizeLarge') },
    { value: '1920', label: '1920', desc: t('tools.sizeOriginal') },
  ]
  return (
    <div className="tools-card-selector">
      {presets.map(p => (
        <button
          key={p.value}
          type="button"
          className={`tools-mode-card ${value === p.value ? 'tools-mode-card--active' : ''}`}
          onClick={() => onChange(p.value)}
        >
          <span className="tools-mode-card-title">{p.label}</span>
          <span className="tools-mode-card-desc">{p.desc}</span>
        </button>
      ))}
    </div>
  )
}

function ResizePresetCards({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const { t } = useLanguage()
  const presets = [
    { value: '1920x1080', w: '1920', h: '1080', label: '1920×1080', desc: t('tools.sizeLarge') },
    { value: '1280x720', w: '1280', h: '720', label: '1280×720', desc: t('tools.sizeMedium') },
    { value: '854x480', w: '854', h: '480', label: '854×480', desc: t('tools.sizeSmall') },
    { value: '640x360', w: '640', h: '360', label: '640×360', desc: t('tools.sizeTiny') },
  ]
  return (
    <div className="tools-card-selector">
      {presets.map(p => (
        <button
          key={p.value}
          type="button"
          className={`tools-mode-card ${value === p.value ? 'tools-mode-card--active' : ''}`}
          onClick={() => onChange(p.value)}
        >
          <span className="tools-mode-card-title">{p.label}</span>
          <span className="tools-mode-card-desc">{p.desc}</span>
        </button>
      ))}
    </div>
  )
}

function GifPresetCards({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const { t } = useLanguage()
  const presets = [
    { value: 'low', label: t('tools.qualityLow'), desc: '5fps / 320px' },
    { value: 'medium', label: t('tools.qualityMedium'), desc: '10fps / 480px' },
    { value: 'high', label: t('tools.qualityHigh'), desc: '15fps / 640px' },
  ]
  return (
    <div className="tools-quality-grid">
      {presets.map(p => (
        <button
          key={p.value}
          type="button"
          className={`tools-quality-card ${value === p.value ? 'tools-quality-card--active' : ''}`}
          onClick={() => onChange(p.value)}
        >
          <div className="tools-quality-radio">{value === p.value ? '●' : '○'}</div>
          <div className="tools-quality-label">
            {p.label}
            {p.value === 'medium' && <span className="tools-quality-badge">RECOMMENDED</span>}
          </div>
          <div className="tools-quality-desc">{p.desc}</div>
        </button>
      ))}
    </div>
  )
}

function FrameIntervalPresetCards({
  value,
  onChange,
}: {
  value: string
  onChange: (v: string) => void
}) {
  const { t } = useLanguage()
  const presets = [
    { value: '0.5', label: '0.5s', desc: t('tools.intervalDense') },
    { value: '1', label: '1s', desc: t('tools.intervalNormal') },
    { value: '2', label: '2s', desc: t('tools.intervalSparse') },
    { value: '5', label: '5s', desc: t('tools.intervalVerySparse') },
  ]
  return (
    <div className="tools-card-selector">
      {presets.map(p => (
        <button
          key={p.value}
          type="button"
          className={`tools-mode-card ${value === p.value ? 'tools-mode-card--active' : ''}`}
          onClick={() => onChange(p.value)}
        >
          <span className="tools-mode-card-title">{p.label}</span>
          <span className="tools-mode-card-desc">{p.desc}</span>
        </button>
      ))}
    </div>
  )
}

/* ───────────── 预设值映射工具函数 ───────────── */

function gifPresetToValues(preset: string): { fps: number; scale: number } {
  switch (preset) {
    case 'low':
      return { fps: 5, scale: 320 }
    case 'high':
      return { fps: 15, scale: 640 }
    default:
      return { fps: 10, scale: 480 }
  }
}

function resizePresetToValues(preset: string): { w: number; h: number } {
  const [w, h] = preset.split('x').map(Number)
  return { w: w || 800, h: h || 600 }
}

const ABILITIES: AbilityDef[] = [
  // ── 图片 ──
  {
    id: 'image-compress',
    category: 'image',
    icon: <IconImage size={16} />,
    titleKey: 'tools.ability.imageCompress',
    descKey: 'tools.ability.imageCompressDesc',
    layout: 'stacked',
    multiInput: true,
    needsOutput: true,
    filters: IMAGE_FILTERS,
    outputExt: 'png',
    fields: [],
    run: ({ inputs, output, values }) => {
      const q = values.quality || 'medium'
      const qualityMap: Record<string, number> = { low: 60, medium: 82, high: 95 }
      const quality = qualityMap[q] ?? 82
      if (inputs.length === 1) {
        return imageCompress(inputs[0], output, numOrUndef(values.maxWidth), undefined, quality)
      }
      return imageCompressBatch(
        inputs,
        outputDirFrom(output, inputs),
        numOrUndef(values.maxWidth),
        undefined,
        quality,
      )
    },
    resultPath: res =>
      typeof res.output_dir === 'string'
        ? res.output_dir
        : typeof res.output === 'string'
          ? res.output
          : null,
    renderFileCard: props => <FileListCard {...props} />,
    renderSettings: ({ values, onChange }) => (
      <>
        <div className="tools-zone-title">最大宽度</div>
        <ImageSizePresetCards
          value={values.maxWidth || '1920'}
          onChange={v => onChange('maxWidth', v)}
        />
        <div className="tools-zone-title" style={{ marginTop: 12 }}>
          质量
        </div>
        <QualityPresetCards
          value={values.quality || 'medium'}
          onChange={v => onChange('quality', v)}
        />
      </>
    ),
  },
  {
    id: 'image-convert',
    category: 'image',
    icon: <IconRefresh size={16} />,
    titleKey: 'tools.ability.imageConvert',
    descKey: 'tools.ability.imageConvertDesc',
    layout: 'stacked',
    multiInput: true,
    needsOutput: true,
    filters: IMAGE_FILTERS,
    outputExt: 'png',
    fields: [],
    run: ({ inputs, output, values }) => {
      const fmt = values.format || 'png'
      if (inputs.length === 1) {
        const out = output.replace(/\.[^.\\/]+$/, `.${fmt}`)
        return imageConvert(inputs[0], out)
      }
      return imageConvertBatch(inputs, outputDirFrom(output, inputs), fmt)
    },
    resultPath: res =>
      typeof res.output_dir === 'string'
        ? res.output_dir
        : typeof res.output === 'string'
          ? res.output
          : null,
    renderFileCard: props => <FileListCard {...props} />,
    renderSettings: ({ values, onChange }) => (
      <>
        <div className="tools-zone-title">输出格式</div>
        <FormatSelectCards
          value={values.format || 'png'}
          options={[
            { value: 'png', label: 'PNG' },
            { value: 'jpg', label: 'JPG' },
            { value: 'webp', label: 'WebP' },
            { value: 'bmp', label: 'BMP' },
            { value: 'gif', label: 'GIF' },
          ]}
          onChange={v => onChange('format', v)}
        />
      </>
    ),
  },
  {
    id: 'image-resize',
    category: 'image',
    icon: <IconCrop size={16} />,
    titleKey: 'tools.ability.imageResize',
    descKey: 'tools.ability.imageResizeDesc',
    layout: 'stacked',
    multiInput: true,
    needsOutput: true,
    filters: IMAGE_FILTERS,
    outputExt: 'png',
    fields: [],
    run: ({ inputs, output, values }) => {
      const { w, h } = resizePresetToValues(values.resizePreset || '1280x720')
      if (inputs.length === 1) {
        return imageResize(inputs[0], output, w, h)
      }
      return imageResizeBatch(inputs, outputDirFrom(output, inputs), w, h)
    },
    resultPath: res =>
      typeof res.output_dir === 'string'
        ? res.output_dir
        : typeof res.output === 'string'
          ? res.output
          : null,
    renderFileCard: props => <FileListCard {...props} />,
    renderSettings: ({ values, onChange }) => (
      <>
        <div className="tools-zone-title">输出尺寸</div>
        <ResizePresetCards
          value={values.resizePreset || '1280x720'}
          onChange={v => onChange('resizePreset', v)}
        />
      </>
    ),
  },
  {
    id: 'image-info',
    category: 'image',
    icon: <IconEye size={16} />,
    titleKey: 'tools.ability.imageInfo',
    descKey: 'tools.ability.imageInfoDesc',
    layout: 'stacked',
    filters: IMAGE_FILTERS,
    outputExt: '',
    fields: [],
    run: ({ inputs }) => imageInfo(inputs[0]),
    resultText: res => {
      const keys = ['path', 'format', 'width', 'height', 'size_bytes']
      return keys
        .filter(k => k in res)
        .map(k => {
          const v = res[k]
          const label = k.replace(/_/g, ' ')
          if (k === 'size_bytes' && typeof v === 'number')
            return `${label}: ${(v / 1024).toFixed(1)} KB`
          return `${label}: ${v}`
        })
        .join('\n')
    },
    renderResult: ({ result }) => <InfoResult result={result} />,
  },
  {
    id: 'image-to-pdf',
    category: 'image',
    icon: <IconFile size={16} />,
    titleKey: 'tools.ability.imageToPdf',
    descKey: 'tools.ability.imageToPdfDesc',
    multiInput: true,
    layout: 'stacked',
    needsOutput: true,
    filters: IMAGE_FILTERS,
    outputExt: 'pdf',
    fields: [],
    run: ({ inputs, output }) => pdfImagesToPdf(inputs, output),
    renderFileCard: props => <FileListCard {...props} />,
  },
  {
    id: 'image-stitch',
    category: 'image',
    icon: <IconLayers size={16} />,
    titleKey: 'tools.ability.imageStitch',
    descKey: 'tools.ability.imageStitchDesc',
    multiInput: true,
    layout: 'stacked',
    needsOutput: true,
    filters: IMAGE_FILTERS,
    outputExt: 'png',
    fields: [],
    run: ({ inputs, output, values }) =>
      imageStitch(inputs, output, values.direction || 'horizontal'),
    renderFileCard: props => <FileListCard {...props} />,
    renderSettings: ({ values, onChange }) => (
      <DirectionPresetCards
        value={values.direction || 'horizontal'}
        onChange={v => onChange('direction', v)}
      />
    ),
  },
  {
    id: 'image-compress-batch',
    category: 'image',
    icon: <IconBox size={16} />,
    titleKey: 'tools.ability.imageCompressBatch',
    descKey: 'tools.ability.imageCompressBatchDesc',
    multiInput: true,
    layout: 'stacked',
    needsOutput: true,
    outputIsDir: true,
    filters: IMAGE_FILTERS,
    outputExt: '',
    fields: [],
    run: ({ inputs, output, values }) => {
      const q = values.quality || 'medium'
      const qualityMap: Record<string, number> = { low: 60, medium: 82, high: 95 }
      return imageCompressBatch(
        inputs,
        output,
        numOrUndef(values.maxWidth),
        undefined,
        qualityMap[q] ?? 82,
      )
    },
    resultPath: res => (typeof res.output_dir === 'string' ? `${res.output_dir}` : null),
    renderFileCard: props => <FileListCard {...props} />,
    renderSettings: ({ values, onChange }) => (
      <>
        <div className="tools-zone-title">最大宽度</div>
        <ImageSizePresetCards
          value={values.maxWidth || '1920'}
          onChange={v => onChange('maxWidth', v)}
        />
        <div className="tools-zone-title" style={{ marginTop: 12 }}>
          质量
        </div>
        <QualityPresetCards
          value={values.quality || 'medium'}
          onChange={v => onChange('quality', v)}
        />
      </>
    ),
  },
  {
    id: 'image-convert-batch',
    category: 'image',
    icon: <IconRefresh size={16} />,
    titleKey: 'tools.ability.imageConvertBatch',
    descKey: 'tools.ability.imageConvertBatchDesc',
    multiInput: true,
    layout: 'stacked',
    needsOutput: true,
    outputIsDir: true,
    filters: IMAGE_FILTERS,
    outputExt: '',
    fields: [],
    run: ({ inputs, output, values }) => imageConvertBatch(inputs, output, values.format || 'jpg'),
    resultPath: res => (typeof res.output_dir === 'string' ? `${res.output_dir}` : null),
    renderFileCard: props => <FileListCard {...props} />,
    renderSettings: props => <BatchFormatSettings {...props} />,
  },
  // ── 视频 ──
  {
    id: 'video-compress',
    category: 'video',
    icon: <IconPlay size={16} />,
    titleKey: 'tools.ability.videoCompress',
    descKey: 'tools.ability.videoCompressDesc',
    layout: 'stacked',
    needsOutput: true,
    filters: VIDEO_FILTERS,
    outputExt: 'mp4',
    fields: [
      {
        key: 'quality',
        labelKey: 'tools.videoQuality',
        kind: 'select',
        default: 'medium',
        options: [
          { value: 'low', labelKey: 'tools.qualityLow' },
          { value: 'medium', labelKey: 'tools.qualityMedium' },
          { value: 'high', labelKey: 'tools.qualityHigh' },
        ],
      },
    ],
    run: ({ inputs, output, values }) =>
      videoCompress(inputs[0], output, values.quality || 'medium'),
    renderSettings: ({ values, onChange }) => (
      <VideoQualityCards
        value={values.quality || 'medium'}
        onChange={v => onChange('quality', v)}
      />
    ),
  },
  {
    id: 'video-audio',
    category: 'audio',
    icon: <IconMic size={16} />,
    titleKey: 'tools.ability.videoAudio',
    descKey: 'tools.ability.videoAudioDesc',
    layout: 'stacked',
    needsOutput: true,
    filters: VIDEO_FILTERS,
    outputExt: 'mp3',
    fields: [
      {
        key: 'format',
        labelKey: 'tools.audioFormat',
        kind: 'select',
        default: 'mp3',
        options: [
          { value: 'mp3', labelKey: 'tools.audioMp3' },
          { value: 'wav', labelKey: 'tools.audioWav' },
        ],
      },
    ],
    run: ({ inputs, output, values }) =>
      videoExtractAudio(inputs[0], output, values.format || 'mp3'),
  },
  {
    id: 'video-frames',
    category: 'video',
    icon: <IconGrid size={16} />,
    titleKey: 'tools.ability.videoFrames',
    descKey: 'tools.ability.videoFramesDesc',
    layout: 'stacked',
    needsOutput: true,
    outputIsDir: true,
    filters: VIDEO_FILTERS,
    outputExt: '',
    fields: [],
    run: ({ inputs, output, values }) =>
      videoExtractFrames(inputs[0], output, Number(values.interval) || 1),
    resultPath: res => (typeof res.output_dir === 'string' ? res.output_dir : null),
    renderSettings: ({ values, onChange }) => (
      <>
        <div className="tools-zone-title">抽帧间隔</div>
        <FrameIntervalPresetCards
          value={values.interval || '1'}
          onChange={v => onChange('interval', v)}
        />
      </>
    ),
  },
  {
    id: 'video-info',
    category: 'video',
    icon: <IconEye size={16} />,
    titleKey: 'tools.ability.videoInfo',
    descKey: 'tools.ability.videoInfoDesc',
    layout: 'stacked',
    filters: VIDEO_FILTERS,
    outputExt: '',
    fields: [],
    run: ({ inputs }) => videoInfo(inputs[0]),
    resultText: res => {
      const parts: string[] = []
      if (typeof res.duration_seconds === 'number')
        parts.push(`Duration: ${res.duration_seconds.toFixed(1)}s`)
      if (res.video_codec) parts.push(`Video: ${res.video_codec}`)
      if (res.audio_codec) parts.push(`Audio: ${res.audio_codec}`)
      if (res.width && res.height) parts.push(`Resolution: ${res.width}×${res.height}`)
      if (typeof res.size_bytes === 'number')
        parts.push(`Size: ${(res.size_bytes / 1024 / 1024).toFixed(1)} MB`)
      return parts.join('\n') || null
    },
    renderResult: ({ result }) => <InfoResult result={result} />,
  },
  {
    id: 'video-gif',
    category: 'video',
    icon: <IconRefresh size={16} />,
    titleKey: 'tools.ability.videoGif',
    descKey: 'tools.ability.videoGifDesc',
    layout: 'stacked',
    needsOutput: true,
    filters: GIF_FILTERS,
    outputExt: 'gif',
    fields: [],
    run: ({ inputs, output, values }) => {
      const { fps, scale } = gifPresetToValues(values.gifQuality || 'medium')
      return videoToGif(inputs[0], output, fps, scale)
    },
    renderSettings: ({ values, onChange }) => (
      <>
        <div className="tools-zone-title">GIF 质量</div>
        <GifPresetCards
          value={values.gifQuality || 'medium'}
          onChange={v => onChange('gifQuality', v)}
        />
      </>
    ),
  },
  {
    id: 'video-cut',
    category: 'video',
    icon: <IconCrop size={16} />,
    titleKey: 'tools.ability.videoCut',
    descKey: 'tools.ability.videoCutDesc',
    layout: 'stacked',
    needsOutput: true,
    filters: VIDEO_FILTERS,
    outputExt: 'mp4',
    fields: [
      { key: 'start', labelKey: 'tools.cutStart', kind: 'number', default: '0' },
      { key: 'end', labelKey: 'tools.cutEnd', kind: 'number', default: '' },
    ],
    run: ({ inputs, output, values }) => {
      const start = Number(values.start) || 0
      const end = values.end !== '' ? Number(values.end) : undefined
      if (end !== undefined && !(end > start)) throw '结束时间需大于开始时间'
      return videoCut(inputs[0], output, start, end)
    },
    renderSettings: props => <TimeRangeFields values={props.values} onChange={props.onChange} />,
  },
  {
    id: 'audio-convert',
    category: 'audio',
    icon: <IconMic size={16} />,
    titleKey: 'tools.ability.audioConvert',
    descKey: 'tools.ability.audioConvertDesc',
    layout: 'stacked',
    needsOutput: true,
    filters: AUDIO_FILTERS,
    outputExt: 'mp3',
    fields: [],
    run: ({ inputs, output }) => audioConvert(inputs[0], output),
  },
  {
    id: 'voice-clone',
    category: 'audio',
    icon: <IconMic size={16} />,
    titleKey: 'tools.ability.voiceClone',
    descKey: 'tools.ability.voiceCloneDesc',
    layout: 'stacked',
    needsOutput: true,
    filters: AUDIO_FILTERS,
    outputExt: 'mp3',
    fields: [
      {
        key: 'text',
        labelKey: 'tools.voiceCloneText',
        kind: 'text',
        placeholder: '输入要合成的文字',
        default: '',
      },
    ],
    run: ({ inputs, output, values }) => voiceClone(inputs[0], values.text || '', output),
  },
  {
    id: 'doc-extract',
    category: 'doc',
    icon: <IconBook size={16} />,
    titleKey: 'tools.ability.docExtract',
    descKey: 'tools.ability.docExtractDesc',
    layout: 'stacked',
    filters: DOC_FILTERS,
    outputExt: '',
    fields: [],
    run: ({ inputs }) => docExtractText(inputs[0]),
    resultText: res => (typeof res.text === 'string' ? res.text : null),
  },
  // ── PDF（置于末尾：大王定调 PDF 工具较少用，tab 顺序放最后）──
  {
    id: 'pdf-page-count',
    category: 'pdf',
    icon: <IconEye size={16} />,
    titleKey: 'tools.ability.pdfPages',
    descKey: 'tools.ability.pdfPagesDesc',
    layout: 'stacked',
    filters: PDF_FILTERS,
    outputExt: '',
    fields: [],
    run: ({ inputs }) => pdfPageCount(inputs[0]),
    resultText: res => (typeof res.pages === 'number' ? `Pages: ${res.pages}` : null),
    renderResult: ({ result }) => <InfoResult result={result} />,
  },
  {
    id: 'pdf-merge',
    category: 'pdf',
    icon: <IconLayers size={16} />,
    titleKey: 'tools.ability.pdfMerge',
    descKey: 'tools.ability.pdfMergeDesc',
    multiInput: true,
    layout: 'stacked',
    needsOutput: true,
    filters: PDF_FILTERS,
    outputExt: 'pdf',
    fields: [],
    run: ({ inputs, output }) => pdfMerge(inputs, output),
    renderFileCard: props => <FileListCard {...props} />,
  },
  {
    id: 'pdf-compress',
    category: 'pdf',
    icon: <IconBox size={16} />,
    titleKey: 'tools.ability.pdfCompress',
    descKey: 'tools.ability.pdfCompressDesc',
    layout: 'stacked',
    needsOutput: true,
    filters: PDF_FILTERS,
    outputExt: 'pdf',
    fields: [],
    run: ({ inputs, output }) => pdfCompress(inputs[0], output),
    resultPath: res => (typeof res.output === 'string' ? res.output : null),
  },
  {
    id: 'pdf-extract',
    category: 'pdf',
    icon: <IconType size={16} />,
    titleKey: 'tools.ability.pdfExtract',
    descKey: 'tools.ability.pdfExtractDesc',
    layout: 'stacked',
    filters: PDF_FILTERS,
    outputExt: '',
    fields: [
      { key: 'maxPages', labelKey: 'tools.extractMaxPages', kind: 'number', default: '200' },
    ],
    run: ({ inputs, values }) =>
      pdfExtractText(inputs[0], values.maxPages ? Number(values.maxPages) : undefined),
    resultText: res => (typeof res.text === 'string' ? res.text : null),
  },
  {
    id: 'pdf-extract-pages',
    category: 'pdf',
    icon: <IconCrop size={16} />,
    titleKey: 'tools.ability.pdfExtractPages',
    descKey: 'tools.ability.pdfExtractPagesDesc',
    layout: 'stacked',
    needsOutput: true,
    filters: PDF_FILTERS,
    outputExt: 'pdf',
    fields: [
      {
        key: 'pages',
        labelKey: 'tools.pagesList',
        kind: 'text',
        placeholder: '如 1,3,5',
        default: '',
      },
    ],
    run: ({ inputs, output, values }) => {
      const pages = (values.pages ?? '')
        .split(/[,，\s]+/)
        .map(s => Number(s.trim()))
        .filter(n => Number.isFinite(n) && n > 0)
      if (pages.length === 0) throw '请填写要提取的页码（如 1,3,5）'
      return pdfExtractPages(inputs[0], pages, output)
    },
  },
  {
    id: 'pdf-rotate',
    category: 'pdf',
    icon: <IconRefresh size={16} />,
    titleKey: 'tools.ability.pdfRotate',
    descKey: 'tools.ability.pdfRotateDesc',
    layout: 'stacked',
    needsOutput: true,
    filters: PDF_FILTERS,
    outputExt: 'pdf',
    fields: [
      {
        key: 'degrees',
        labelKey: 'tools.rotateDegrees',
        kind: 'select',
        default: '90',
        options: [
          { value: '90', labelKey: 'tools.rotate90' },
          { value: '180', labelKey: 'tools.rotate180' },
          { value: '270', labelKey: 'tools.rotate270' },
        ],
      },
    ],
    run: ({ inputs, output, values }) => pdfRotate(inputs[0], output, Number(values.degrees) || 90),
    renderSettings: ({ values, onChange }) => (
      <RotationAngleCards value={values.degrees || '90'} onChange={v => onChange('degrees', v)} />
    ),
  },
]

function numOrUndef(v: string): number | undefined {
  const n = Number(v)
  return Number.isFinite(n) && n > 0 ? n : undefined
}

function errText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : String(e)
}

function fileLabel(p: string): string {
  return p.split(/[\\/]/).pop() || p
}

/** 从输出路径提取目录；裸文件名（无目录前缀）时回退到第一个输入文件的同级目录 */
function outputDirFrom(output: string, inputs: string[]): string {
  const dir = output.replace(/[\\/][^\\/]+$/, '')
  if (dir) return dir
  if (inputs.length > 0) {
    const firstDir = inputs[0].replace(/[\\/][^\\/]+$/, '')
    if (firstDir) return firstDir
  }
  return '.'
}

/* ────────────────────────── Level 1: 分类选择器 + 能力卡片 ────────────────────────── */

function ToolsHome({ onPick }: { onPick: (id: string) => void }) {
  const { t } = useLanguage()
  const [cat, setCat] = useState<Category>('all')
  const cats: { id: Category; labelKey: string; icon: ReactNode }[] = [
    { id: 'all', labelKey: 'tools.catAll', icon: <IconGrid size={13} /> },
    { id: 'image', labelKey: 'tools.tabImage', icon: <IconImage size={13} /> },
    { id: 'video', labelKey: 'tools.tabVideo', icon: <IconPlay size={13} /> },
    { id: 'doc', labelKey: 'tools.tabDoc', icon: <IconType size={13} /> },
    { id: 'audio', labelKey: 'tools.tabAudio', icon: <IconMic size={13} /> },
    { id: 'pdf', labelKey: 'tools.tabPdf', icon: <IconFile size={13} /> },
  ]
  const list = cat === 'all' ? ABILITIES : ABILITIES.filter(a => a.category === cat)

  return (
    <div className="tools-home">
      <div className="tools-cats" role="tablist" aria-label={t('app.tools')}>
        {cats.map(c => (
          <button
            key={c.id}
            type="button"
            role="tab"
            aria-selected={cat === c.id}
            className={['tools-cat', cat === c.id && 'tools-cat--active'].filter(Boolean).join(' ')}
            onClick={() => setCat(c.id)}
          >
            {c.icon}
            {t(c.labelKey)}
          </button>
        ))}
      </div>
      <div className="tools-grid">
        {list.map(a => (
          <button key={a.id} type="button" className="tools-card" onClick={() => onPick(a.id)}>
            <span className="tools-card-icon">{a.icon}</span>
            <span className="tools-card-body">
              <span className="tools-card-title">{t(a.titleKey)}</span>
              <span className="tools-card-desc">{t(a.descKey)}</span>
            </span>
            <IconChevronRight className="tools-card-arrow" size={14} />
          </button>
        ))}
      </div>
    </div>
  )
}

/* ────────────────────────── Level 2: 能力详情页（文件区 + 设置区） ────────────────────────── */

function ToolDetail({ ability, onBack }: { ability: AbilityDef; onBack: () => void }) {
  const { t } = useLanguage()
  const [inputs, setInputs] = useState<string[]>([])
  const [output, setOutput] = useState('')
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(ability.fields.map(f => [f.key, f.default ?? ''])),
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<string | null>(null)
  const [resultPath, setResultPath] = useState<string | null>(null)
  const [resultText, setResultText] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const [previewPath, setPreviewPath] = useState<string | null>(null)

  const pickInputs = async () => {
    const sel = await open({
      multiple: ability.multiInput,
      directory: false,
      filters: ability.filters,
    })
    if (sel) setInputs(Array.isArray(sel) ? sel : [sel])
  }
  const pickOutput = async () => {
    if (ability.outputIsDir) {
      const dir = await open({ multiple: false, directory: true })
      if (typeof dir === 'string') setOutput(dir)
      return
    }
    const base = (inputs[0] ?? '').replace(/\.[^.\\/]+$/, '') || ability.id
    const sel = await save({
      defaultPath: `${base}-out.${ability.outputExt}`,
      filters: ability.filters,
    })
    if (sel) setOutput(sel)
  }

  const removeInput = (p: string) => {
    setInputs(prev => {
      const next = prev.filter(x => x !== p)
      if (previewPath === p) setPreviewPath(null)
      return next
    })
  }

  const moveInput = (from: number, to: number) => {
    setInputs(prev => {
      if (to < 0 || to >= prev.length) return prev
      const next = [...prev]
      const [item] = next.splice(from, 1)
      next.splice(to, 0, item)
      return next
    })
  }

  const run = async () => {
    if (inputs.length === 0) {
      setError(t('tools.errNoInput'))
      return
    }
    if (ability.needsOutput && !output) {
      setError(t('tools.errNoOutput'))
      return
    }
    setBusy(true)
    setError(null)
    setResult(null)
    setResultPath(null)
    setResultText(null)
    setCopied(false)
    try {
      const data: unknown = await ability.run({ inputs, output, values })
      const res = (data ?? {}) as Record<string, unknown>
      setResult(JSON.stringify(res, null, 2))
      setResultPath(
        ability.resultPath
          ? ability.resultPath(res)
          : typeof res.output === 'string'
            ? res.output
            : null,
      )
      setResultText(ability.resultText ? ability.resultText(res) : null)
    } catch (e) {
      setError(errText(e))
    } finally {
      setBusy(false)
    }
  }

  // 目录型输出由用户显式选择目录，不自动拼默认文件名
  const defaultOutputName = useMemo(() => {
    if (ability.outputIsDir) return ''
    const base = (inputs[0] ?? '').replace(/\.[^.\\/]+$/, '') || ability.id
    return `${base}-out.${ability.outputExt}`
  }, [inputs, ability])

  // 输入文件变化时，若输出为空则自动填充默认名（仅文件型输出）
  useEffect(() => {
    if (inputs.length > 0 && !output && !ability.outputIsDir) setOutput(defaultOutputName)
  }, [inputs, defaultOutputName, ability.outputIsDir])

  const pickOutputDir = async () => {
    const dir = await open({ multiple: false, directory: true })
    if (typeof dir === 'string') setOutput(dir)
  }

  const updateValue = (key: string, value: string) => setValues(v => ({ ...v, [key]: value }))

  const head = (
    <div className="tools-detail-head">
      <button type="button" className="tools-back" onClick={onBack}>
        <IconArrowLeft size={14} /> {t('tools.back')}
      </button>
      <div className="tools-detail-title">{t(ability.titleKey)}</div>
      <div className="tools-detail-desc">{t(ability.descKey)}</div>
    </div>
  )

  if (ability.layout === 'stacked') {
    return (
      <div className="tools-detail tools-detail--stacked">
        {head}
        <div className="tools-stacked-body">
          {/* 文件区 */}
          {inputs.length === 0 ? (
            <div
              className="tools-dropzone"
              role="button"
              tabIndex={0}
              onClick={() => void pickInputs()}
              onKeyDown={e => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault()
                  void pickInputs()
                }
              }}
            >
              <IconUpload size={28} />
              <div className="tools-drop-title">{t('tools.dropTitle')}</div>
              <div className="tools-drop-sub">{t('tools.dropSub')}</div>
              <div className="tools-drop-btn">
                <Button variant="primary" size="sm">
                  <IconFolder size={13} /> {t('tools.selectFiles')}
                </Button>
              </div>
              <div className="tools-drop-hint">{t('tools.dropHint')}</div>
            </div>
          ) : (
            <>
              {ability.renderFileCard ? (
                ability.renderFileCard({ inputs, onRemove: removeInput, onMove: moveInput })
              ) : (
                <div className="tools-file-cards">
                  {inputs.map(p => (
                    <div key={p} className="tools-file-card">
                      <div className="tools-file-card-icon">PDF</div>
                      <div className="tools-file-card-info">
                        <div className="tools-file-card-name">{fileLabel(p)}</div>
                      </div>
                      <button
                        type="button"
                        className="tools-file-card-remove"
                        onClick={() => removeInput(p)}
                        title="移除"
                      >
                        <IconX size={14} />
                      </button>
                    </div>
                  ))}
                  <Button variant="default" size="sm" onClick={() => void pickInputs()}>
                    <IconFolder size={13} /> {t('tools.addFiles')}
                  </Button>
                </div>
              )}

              {/* 设置面板 */}
              {ability.renderSettings ? (
                ability.renderSettings({
                  values,
                  onChange: updateValue,
                  inputs,
                  result: result ? JSON.parse(result) : null,
                  busy,
                })
              ) : ability.fields.length > 0 ? (
                <div className="tools-stacked-section">
                  <div className="tools-zone-title">{t('tools.settingsArea')}</div>
                  {ability.fields.map(f => (
                    <div key={f.key} className="tools-field">
                      <div className="tools-field-label">{t(f.labelKey)}</div>
                      {f.kind === 'select' ? (
                        <select
                          className="tools-input"
                          value={values[f.key] ?? ''}
                          onChange={e => updateValue(f.key, e.target.value)}
                        >
                          {f.options?.map(o => (
                            <option key={o.value} value={o.value}>
                              {t(o.labelKey)}
                            </option>
                          ))}
                        </select>
                      ) : f.kind === 'text' ? (
                        <textarea
                          className="tools-input tools-input--textarea"
                          rows={4}
                          value={values[f.key] ?? ''}
                          onChange={e => updateValue(f.key, e.target.value)}
                          placeholder={f.placeholder}
                        />
                      ) : (
                        <input
                          className="tools-input"
                          value={values[f.key] ?? ''}
                          onChange={e => updateValue(f.key, e.target.value)}
                          placeholder={f.placeholder}
                        />
                      )}
                    </div>
                  ))}
                </div>
              ) : null}

              {/* 输出路径 */}
              {ability.needsOutput && (
                <div className="tools-stacked-section">
                  <div className="tools-zone-title">
                    {ability.outputIsDir ? t('tools.outputDir') : t('tools.outputPath')}
                  </div>
                  <div className="tools-field-row">
                    <input
                      className="tools-input"
                      value={output}
                      onChange={e => setOutput(e.target.value)}
                    />
                    <Button
                      variant="default"
                      size="sm"
                      onClick={() => void (ability.outputIsDir ? pickOutputDir() : pickOutput())}
                    >
                      <IconFolder size={13} />{' '}
                      {ability.outputIsDir ? t('tools.chooseDir') : t('tools.browse')}
                    </Button>
                  </div>
                </div>
              )}

              {/* 执行按钮 + 结果 */}
              <div className="tools-stacked-section tools-stacked-section--action">
                <Button
                  variant="primary"
                  className="tools-run-btn"
                  disabled={busy || inputs.length === 0}
                  onClick={() => void run()}
                >
                  {busy ? <IconRefresh size={13} className="tools-spin" /> : null}
                  {busy ? t('tools.running') : t('tools.save')}
                </Button>
                {error && (
                  <div className="tools-result tools-result--error">
                    <div className="tools-result-title">{t('tools.resultFailed')}</div>
                    <div className="tools-result-msg">{error}</div>
                  </div>
                )}
                {result &&
                  (ability.renderResult ? (
                    ability.renderResult({ result: JSON.parse(result) })
                  ) : (
                    <div className="tools-result">
                      <div className="tools-result-title">{t('tools.resultDone')}</div>
                      <pre className="tools-result-msg">{result}</pre>
                    </div>
                  ))}
                {result && resultPath && (
                  <div className="tools-result-actions">
                    {ability.outputIsDir ? (
                      <Button
                        variant="default"
                        size="sm"
                        onClick={() => void revealPath(resultPath!).catch(() => undefined)}
                      >
                        <IconFolder size={13} /> {t('tools.openDir')}
                      </Button>
                    ) : (
                      <>
                        <Button
                          variant="default"
                          size="sm"
                          onClick={() => setPreviewPath(resultPath!)}
                        >
                          <IconEye size={13} /> {t('tools.preview')}
                        </Button>
                        <Button
                          variant="default"
                          size="sm"
                          onClick={() => void revealPath(resultPath!).catch(() => undefined)}
                        >
                          <IconFolder size={13} /> {t('tools.reveal')}
                        </Button>
                      </>
                    )}
                  </div>
                )}
                {previewPath && (
                  <div className="tools-preview-panel tools-preview-panel--main">
                    <div className="tools-preview-head">
                      <span className="tools-preview-title" title={previewPath}>
                        {fileLabel(previewPath)}
                      </span>
                      <button
                        type="button"
                        className="tools-preview-close"
                        onClick={() => setPreviewPath(null)}
                        title={t('tools.closePreview')}
                      >
                        <IconX size={14} />
                      </button>
                    </div>
                    <FilePreviewContent path={previewPath} />
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    )
  }

  return (
    <div className="tools-detail">
      {head}

      <div className="tools-detail-split">
        {/* ── 左侧：上传 + 预览合并为一个大容器（空态=拖放引导，选中后=文件列表+预览） ── */}
        <div className="tools-upload-preview">
          {inputs.length === 0 ? (
            <div
              className="tools-dropzone"
              role="button"
              tabIndex={0}
              onClick={() => void pickInputs()}
              onKeyDown={e => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault()
                  void pickInputs()
                }
              }}
            >
              <IconUpload size={28} />
              <div className="tools-drop-title">{t('tools.dropTitle')}</div>
              <div className="tools-drop-sub">{t('tools.dropSub')}</div>
              <div className="tools-drop-btn">
                <Button variant="primary" size="sm">
                  <IconFolder size={13} /> {t('tools.selectFiles')}
                </Button>
              </div>
              <div className="tools-drop-hint">{t('tools.dropHint')}</div>
            </div>
          ) : (
            <>
              <div className="tools-file-strip">
                <div className="tools-file-label">{t('tools.selectedFiles')}</div>
                <div className="tools-file-chips">
                  {inputs.map(p => (
                    <div key={p} className="tools-file-chip">
                      <span className="tools-file-chip-name" title={p}>
                        {fileLabel(p)}
                      </span>
                      <button
                        type="button"
                        className="tools-file-chip-btn"
                        onClick={() => setPreviewPath(p)}
                        title={t('tools.preview')}
                      >
                        <IconEye size={12} />
                      </button>
                      <button
                        type="button"
                        className="tools-file-chip-btn"
                        onClick={() => removeInput(p)}
                        title="移除"
                      >
                        <IconX size={12} />
                      </button>
                    </div>
                  ))}
                </div>
                <Button variant="default" size="sm" onClick={() => void pickInputs()}>
                  <IconFolder size={13} /> {t('tools.addFiles')}
                </Button>
              </div>

              {/* ── 预览（同一容器内，常驻） ── */}
              <div className="tools-preview-panel tools-preview-panel--main">
                {resultText ? (
                  <>
                    <div className="tools-preview-head">
                      <span className="tools-preview-title">{t('tools.extractedText')}</span>
                      <div style={{ display: 'flex', gap: 4 }}>
                        <button
                          type="button"
                          className="tools-preview-close"
                          onClick={() => {
                            void navigator.clipboard.writeText(resultText)
                            setCopied(true)
                            setTimeout(() => setCopied(false), 1500)
                          }}
                          title={t('tools.copyText')}
                        >
                          {copied ? '✓' : <IconUpload size={14} />}
                        </button>
                        <button
                          type="button"
                          className="tools-preview-close"
                          onClick={() => setResultText(null)}
                          title={t('tools.closePreview')}
                        >
                          <IconX size={14} />
                        </button>
                      </div>
                    </div>
                    <pre
                      style={{
                        flex: 1,
                        margin: 0,
                        padding: '8px 12px',
                        overflow: 'auto',
                        fontSize: 13,
                        lineHeight: 1.6,
                        whiteSpace: 'pre-wrap',
                        wordBreak: 'break-word',
                        fontFamily: 'inherit',
                      }}
                    >
                      {resultText}
                    </pre>
                  </>
                ) : previewPath ? (
                  <>
                    <div className="tools-preview-head">
                      <span className="tools-preview-title" title={previewPath}>
                        {fileLabel(previewPath)}
                      </span>
                      <button
                        type="button"
                        className="tools-preview-close"
                        onClick={() => setPreviewPath(null)}
                        title="关闭预览"
                      >
                        <IconX size={14} />
                      </button>
                    </div>
                    <FilePreviewContent path={previewPath} />
                  </>
                ) : (
                  <div className="tools-preview-empty">{t('tools.previewEmpty')}</div>
                )}
              </div>
            </>
          )}
        </div>

        {/* ── 右侧：设置 + 保存 + 状态合并为一个面板 ── */}
        <div className="tools-side">
          <div className="tools-side-title">{t(ability.titleKey)}</div>

          {ability.fields.length > 0 && (
            <div className="tools-side-section">
              <div className="tools-zone-title">{t('tools.settingsArea')}</div>
              {ability.fields.map(f => (
                <div key={f.key} className="tools-field">
                  <div className="tools-field-label">{t(f.labelKey)}</div>
                  {f.kind === 'select' ? (
                    <select
                      className="tools-input"
                      value={values[f.key] ?? ''}
                      onChange={e => setValues(v => ({ ...v, [f.key]: e.target.value }))}
                    >
                      {f.options?.map(o => (
                        <option key={o.value} value={o.value}>
                          {t(o.labelKey)}
                        </option>
                      ))}
                    </select>
                  ) : f.kind === 'text' ? (
                    <textarea
                      className="tools-input tools-input--textarea"
                      rows={4}
                      value={values[f.key] ?? ''}
                      onChange={e => setValues(v => ({ ...v, [f.key]: e.target.value }))}
                      placeholder={f.placeholder}
                    />
                  ) : (
                    <input
                      className="tools-input"
                      value={values[f.key] ?? ''}
                      onChange={e => setValues(v => ({ ...v, [f.key]: e.target.value }))}
                      placeholder={f.placeholder}
                    />
                  )}
                </div>
              ))}
            </div>
          )}

          {ability.needsOutput && (
            <div className="tools-side-section">
              <div className="tools-zone-title">
                {ability.outputIsDir ? t('tools.outputDir') : t('tools.outputPath')}
              </div>
              <div className="tools-field-row">
                <input
                  className="tools-input"
                  value={output}
                  onChange={e => setOutput(e.target.value)}
                  placeholder={t('tools.outputPlaceholder')}
                />
                <Button variant="default" size="sm" onClick={() => void pickOutput()}>
                  <IconFolder size={13} />
                  {ability.outputIsDir ? t('tools.chooseDir') : t('tools.browse')}
                </Button>
              </div>
            </div>
          )}

          <div className="tools-side-section tools-side-section--action">
            <Button
              variant="primary"
              className="tools-run-btn"
              disabled={busy}
              onClick={() => void run()}
            >
              {busy ? <IconRefresh size={13} className="tools-spin" /> : null}
              {busy ? t('tools.running') : t('tools.save')}
            </Button>
            {error && (
              <div className="tools-result tools-result--error">
                <div className="tools-result-title">{t('tools.resultFailed')}</div>
                <div className="tools-result-msg">{error}</div>
              </div>
            )}
            {result &&
              (ability.renderResult ? (
                ability.renderResult({ result: JSON.parse(result) })
              ) : (
                <div className="tools-result">
                  <div className="tools-result-title">{t('tools.resultDone')}</div>
                  <pre className="tools-result-msg">{result}</pre>
                </div>
              ))}
            {result && resultPath && (
              <div className="tools-result-actions">
                {ability.outputIsDir ? (
                  <Button
                    variant="default"
                    size="sm"
                    onClick={() => void revealPath(resultPath!).catch(() => undefined)}
                  >
                    <IconFolder size={13} /> {t('tools.openDir')}
                  </Button>
                ) : (
                  <>
                    <Button variant="default" size="sm" onClick={() => setPreviewPath(resultPath!)}>
                      <IconEye size={13} /> {t('tools.preview')}
                    </Button>
                    <Button
                      variant="default"
                      size="sm"
                      onClick={() => void revealPath(resultPath!).catch(() => undefined)}
                    >
                      <IconFolder size={13} /> {t('tools.reveal')}
                    </Button>
                  </>
                )}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

/* ────────────────────────── 入口（全屏壳） ────────────────────────── */

export function ToolsPage({ onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [abilityId, setAbilityId] = useState<string | null>(null)
  const ability = ABILITIES.find(a => a.id === abilityId) ?? null
  return (
    <div className="tools-page">
      {/* ── 顶部工具栏：关闭最左，标题，spacer 后为当前视图状态（对齐 preview 壳） ── */}
      <div className="tools-toolbar">
        <button type="button" className="tools-toolbar-close" onClick={onClose} title="关闭工具">
          <IconX size={15} />
        </button>
        <span className="tools-toolbar-title">{t('app.tools')}</span>
        <div className="tools-toolbar-spacer" />
        <span className="tools-toolbar-view">
          {ability ? t(ability.titleKey) : t('tools.catAll')}
        </span>
      </div>
      {ability ? (
        <ToolDetail ability={ability} onBack={() => setAbilityId(null)} />
      ) : (
        <ToolsHome onPick={setAbilityId} />
      )}
    </div>
  )
}
