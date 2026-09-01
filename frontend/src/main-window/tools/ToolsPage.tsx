// ToolsPage — 内置工具页（两级导航）
//
// 设计定位（大王定调）：工具是内部机制（invoke 命令），不是 agent 工具调用项。
// 页面采用「分类选择器 + 能力卡片 → 详情页」两级结构，降低用户认知负担：
//   Level 1: 分类 tab（ALL / PDF / IMAGE / VIDEO）+ 能力卡片网格（每卡片=一种能力）
//   Level 2: 点击卡片 → 详情页，分「文件区」（选择输入/输出 + 预览）与「设置区」（参数）
//
// 命令均在 src-tauri/src/commands/tools/*（内部机制，不进 get_tools/execute_tool）。

import { useState, type ReactNode } from 'react'
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
  pdfRotate,
  imageCompress,
  imageConvert,
  imageResize,
  imageStitch,
  imageCompressBatch,
  imageConvertBatch,
  videoCompress,
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
}

const ABILITIES: AbilityDef[] = [
  // ── 图片 ──
  {
    id: 'image-compress',
    category: 'image',
    icon: <IconImage size={16} />,
    titleKey: 'tools.ability.imageCompress',
    descKey: 'tools.ability.imageCompressDesc',
    needsOutput: true,
    filters: IMAGE_FILTERS,
    outputExt: 'png',
    fields: [
      { key: 'maxWidth', labelKey: 'tools.maxWidth', kind: 'number', default: '1920' },
      { key: 'maxHeight', labelKey: 'tools.maxHeight', kind: 'number', default: '' },
      { key: 'quality', labelKey: 'tools.quality', kind: 'number', default: '82' },
    ],
    run: ({ inputs, output, values }) =>
      imageCompress(
        inputs[0],
        output,
        numOrUndef(values.maxWidth),
        numOrUndef(values.maxHeight),
        numOrUndef(values.quality),
      ),
  },
  {
    id: 'image-convert',
    category: 'image',
    icon: <IconRefresh size={16} />,
    titleKey: 'tools.ability.imageConvert',
    descKey: 'tools.ability.imageConvertDesc',
    needsOutput: true,
    filters: IMAGE_FILTERS,
    outputExt: 'png',
    fields: [],
    run: ({ inputs, output }) => imageConvert(inputs[0], output),
  },
  {
    id: 'image-resize',
    category: 'image',
    icon: <IconCrop size={16} />,
    titleKey: 'tools.ability.imageResize',
    descKey: 'tools.ability.imageResizeDesc',
    needsOutput: true,
    filters: IMAGE_FILTERS,
    outputExt: 'png',
    fields: [
      { key: 'width', labelKey: 'tools.resizeWidth', kind: 'number', default: '800' },
      { key: 'height', labelKey: 'tools.resizeHeight', kind: 'number', default: '600' },
    ],
    run: ({ inputs, output, values }) =>
      imageResize(inputs[0], output, Number(values.width) || 800, Number(values.height) || 600),
  },
  {
    id: 'image-to-pdf',
    category: 'image',
    icon: <IconFile size={16} />,
    titleKey: 'tools.ability.imageToPdf',
    descKey: 'tools.ability.imageToPdfDesc',
    multiInput: true,
    needsOutput: true,
    filters: IMAGE_FILTERS,
    outputExt: 'pdf',
    fields: [],
    run: ({ inputs, output }) => pdfImagesToPdf(inputs, output),
  },
  {
    id: 'image-stitch',
    category: 'image',
    icon: <IconLayers size={16} />,
    titleKey: 'tools.ability.imageStitch',
    descKey: 'tools.ability.imageStitchDesc',
    multiInput: true,
    needsOutput: true,
    filters: IMAGE_FILTERS,
    outputExt: 'png',
    fields: [
      {
        key: 'direction',
        labelKey: 'tools.stitchDirection',
        kind: 'select',
        default: 'horizontal',
        options: [
          { value: 'horizontal', labelKey: 'tools.stitchHorizontal' },
          { value: 'vertical', labelKey: 'tools.stitchVertical' },
        ],
      },
    ],
    run: ({ inputs, output, values }) =>
      imageStitch(inputs, output, values.direction || 'horizontal'),
  },
  {
    id: 'image-compress-batch',
    category: 'image',
    icon: <IconBox size={16} />,
    titleKey: 'tools.ability.imageCompressBatch',
    descKey: 'tools.ability.imageCompressBatchDesc',
    multiInput: true,
    needsOutput: true,
    outputIsDir: true,
    filters: IMAGE_FILTERS,
    outputExt: '',
    fields: [
      { key: 'maxWidth', labelKey: 'tools.maxWidth', kind: 'number', default: '1920' },
      { key: 'quality', labelKey: 'tools.quality', kind: 'number', default: '82' },
    ],
    run: ({ inputs, output, values }) =>
      imageCompressBatch(
        inputs,
        output,
        numOrUndef(values.maxWidth),
        undefined,
        numOrUndef(values.quality),
      ),
    resultPath: res => (typeof res.output_dir === 'string' ? `${res.output_dir}` : null),
  },
  {
    id: 'image-convert-batch',
    category: 'image',
    icon: <IconRefresh size={16} />,
    titleKey: 'tools.ability.imageConvertBatch',
    descKey: 'tools.ability.imageConvertBatchDesc',
    multiInput: true,
    needsOutput: true,
    outputIsDir: true,
    filters: IMAGE_FILTERS,
    outputExt: '',
    fields: [
      {
        key: 'format',
        labelKey: 'tools.batchFormat',
        kind: 'select',
        default: 'jpg',
        options: [
          { value: 'png', labelKey: 'tools.fmtPng' },
          { value: 'jpg', labelKey: 'tools.fmtJpg' },
          { value: 'webp', labelKey: 'tools.fmtWebp' },
          { value: 'bmp', labelKey: 'tools.fmtBmp' },
          { value: 'gif', labelKey: 'tools.fmtGif' },
        ],
      },
    ],
    run: ({ inputs, output, values }) => imageConvertBatch(inputs, output, values.format || 'jpg'),
    resultPath: res => (typeof res.output_dir === 'string' ? `${res.output_dir}` : null),
  },
  // ── 视频 ──
  {
    id: 'video-compress',
    category: 'video',
    icon: <IconPlay size={16} />,
    titleKey: 'tools.ability.videoCompress',
    descKey: 'tools.ability.videoCompressDesc',
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
  },
  {
    id: 'video-audio',
    category: 'audio',
    icon: <IconMic size={16} />,
    titleKey: 'tools.ability.videoAudio',
    descKey: 'tools.ability.videoAudioDesc',
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
    needsOutput: true,
    outputIsDir: true,
    filters: VIDEO_FILTERS,
    outputExt: '',
    fields: [{ key: 'interval', labelKey: 'tools.frameInterval', kind: 'number', default: '1' }],
    run: ({ inputs, output, values }) =>
      videoExtractFrames(inputs[0], output, Number(values.interval) || 1),
    resultPath: res =>
      typeof res.output_dir === 'string' ? `${res.output_dir}/frame_0001.jpg` : null,
  },
  {
    id: 'video-gif',
    category: 'video',
    icon: <IconRefresh size={16} />,
    titleKey: 'tools.ability.videoGif',
    descKey: 'tools.ability.videoGifDesc',
    needsOutput: true,
    filters: GIF_FILTERS,
    outputExt: 'gif',
    fields: [
      { key: 'fps', labelKey: 'tools.gifFps', kind: 'number', default: '10' },
      { key: 'scale', labelKey: 'tools.gifScale', kind: 'number', default: '480' },
    ],
    run: ({ inputs, output, values }) =>
      videoToGif(inputs[0], output, Number(values.fps) || 10, Number(values.scale) || 480),
  },
  {
    id: 'video-cut',
    category: 'video',
    icon: <IconCrop size={16} />,
    titleKey: 'tools.ability.videoCut',
    descKey: 'tools.ability.videoCutDesc',
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
  },
  {
    id: 'audio-convert',
    category: 'audio',
    icon: <IconMic size={16} />,
    titleKey: 'tools.ability.audioConvert',
    descKey: 'tools.ability.audioConvertDesc',
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
    filters: DOC_FILTERS,
    outputExt: '',
    fields: [],
    run: ({ inputs }) => docExtractText(inputs[0]),
  },
  // ── PDF（置于末尾：大王定调 PDF 工具较少用，tab 顺序放最后）──
  {
    id: 'pdf-merge',
    category: 'pdf',
    icon: <IconLayers size={16} />,
    titleKey: 'tools.ability.pdfMerge',
    descKey: 'tools.ability.pdfMergeDesc',
    multiInput: true,
    needsOutput: true,
    filters: PDF_FILTERS,
    outputExt: 'pdf',
    fields: [],
    run: ({ inputs, output }) => pdfMerge(inputs, output),
  },
  {
    id: 'pdf-compress',
    category: 'pdf',
    icon: <IconBox size={16} />,
    titleKey: 'tools.ability.pdfCompress',
    descKey: 'tools.ability.pdfCompressDesc',
    needsOutput: true,
    filters: PDF_FILTERS,
    outputExt: 'pdf',
    fields: [],
    run: ({ inputs, output }) => pdfCompress(inputs[0], output),
  },
  {
    id: 'pdf-extract',
    category: 'pdf',
    icon: <IconType size={16} />,
    titleKey: 'tools.ability.pdfExtract',
    descKey: 'tools.ability.pdfExtractDesc',
    filters: PDF_FILTERS,
    outputExt: '',
    fields: [
      { key: 'maxPages', labelKey: 'tools.extractMaxPages', kind: 'number', default: '200' },
    ],
    run: ({ inputs, values }) =>
      pdfExtractText(inputs[0], values.maxPages ? Number(values.maxPages) : undefined),
  },
  {
    id: 'pdf-extract-pages',
    category: 'pdf',
    icon: <IconCrop size={16} />,
    titleKey: 'tools.ability.pdfExtractPages',
    descKey: 'tools.ability.pdfExtractPagesDesc',
    needsOutput: true,
    filters: PDF_FILTERS,
    outputExt: 'pdf',
    fields: [
      {
        key: 'pages',
        labelKey: 'tools.pagesList',
        kind: 'number',
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
    } catch (e) {
      setError(errText(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="tools-detail">
      <div className="tools-detail-head">
        <button type="button" className="tools-back" onClick={onBack}>
          <IconArrowLeft size={14} /> {t('tools.back')}
        </button>
        <div className="tools-detail-title">{t(ability.titleKey)}</div>
        <div className="tools-detail-desc">{t(ability.descKey)}</div>
      </div>

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
                {previewPath ? (
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
            {result && (
              <div className="tools-result">
                <div className="tools-result-title">{t('tools.resultDone')}</div>
                <pre className="tools-result-msg">{result}</pre>
                {resultPath && (
                  <div className="tools-result-actions">
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
                  </div>
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
