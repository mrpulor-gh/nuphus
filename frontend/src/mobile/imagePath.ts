/**
 * 手机端「本机图片路径」识别（纯函数，无副作用，可单测）。
 *
 * 背景：Agent（Leader / Workflow）交付的验证截图常以电脑本地绝对路径出现在回复里
 * （如 `E:\NUS\_settings_popup_wide.png`）。桌面端 MarkdownContent 能点开预览
 * （Tauri 读本地文件），PWA 没有文件系统权限 → 路径只能当纯文本，用户看不到图。
 * 本模块只回答「哪些文本是本机图片路径」，网络拉取与渲染见 components/LocalImage.tsx。
 *
 * 与桌面端 frontend/src/main-window/chat/MarkdownContent.tsx 的路径识别刻意保持同构
 * （ABSOLUTE_PATH_RE 候选 → 排除 URL / 裸域名 → 扩展名白名单截断），差异仅在于：
 * 白名单收敛为图片类型，手机端只具备「图片缩略图 + 放大」这一种交付形态。
 */

/**
 * 图片扩展名白名单：扩展名后不得紧跟字母/数字/下划线/点/连字符
 * （避免 .md5 / .png.bak / .pngx 之类误判）。
 * ⚠️ 与后端 src-tauri/src/mobile_server.rs 的 image_mime_for 必须一致，勿单边改动。
 */
const IMAGE_EXT_RE = /\.(?:png|jpe?g|gif|webp|bmp)(?![A-Za-z0-9_.-])/i

/** 绝对路径候选：Windows 盘符 / UNC / Unix 根。与桌面端 ABSOLUTE_PATH_RE 同源 */
const ABSOLUTE_PATH_RE = /(?:[A-Za-z]:[\\/]|\\\\|\/)(?:[^\s\\/\r\n<>:"|?*]+(?:[ \t]+[^\s\\/\r\n<>:"|?*]+)*[\\/])*[^\s\\/\r\n<>:"|?*]*/g

/** 裸域名 / IPv4 开头（github.com/a.png、192.168.1.1/a.png）不是本机路径 */
const BARE_DOMAIN_RE = /^(?:(?:[A-Za-z0-9-]+\.)+[A-Za-z]{2,}|\d{1,3}(?:\.\d{1,3}){3})[\\/]/

/** URL scheme 前缀：紧跟其后的候选属于 URL 组成部分，不是本机路径 */
const URL_SCHEME_RE = /[A-Za-z][A-Za-z0-9+.-]*:\/\/$/

/** URL token 前缀（用于识别「盘符片段嵌在 URL query 里」的误判场景） */
const URL_TOKEN_RE = /^[A-Za-z][A-Za-z0-9+.-]*:\/\//

/**
 * 结构性片段——其中的路径由专门的渲染分支处理，普通文本扫描不重复接管：
 * - 行内代码 `x`：整段即路径时由调用方 isImagePath 判定（与桌面端反引号路径同款）
 * - markdown 链接 [label](url)：url 不进图片通道（协议白名单只放行 http/https/mailto）
 */
const SKIP_SPAN_RES = [/`[^`]*`/g, /\[[^\]]*\]\([^)]*\)/g]

export interface ImagePathRange {
  start: number
  end: number
}

/** 收集需跳过的片段区间（行内代码 / markdown 链接） */
function collectSkipSpans(text: string): ImagePathRange[] {
  const spans: ImagePathRange[] = []
  for (const pattern of SKIP_SPAN_RES) {
    const re = new RegExp(pattern.source, 'g')
    let m: RegExpExecArray | null
    while ((m = re.exec(text)) !== null) {
      spans.push({ start: m.index, end: m.index + m[0].length })
      if (m[0].length === 0) re.lastIndex++
    }
  }
  return spans
}

function overlaps(spans: ImagePathRange[], start: number, end: number): boolean {
  return spans.some(s => start < s.end && end > s.start)
}

/** 候选是否落在 URL token 内（如 https://relay/dev/file?path=E:\a.png 的盘符片段） */
function insideUrlToken(text: string, index: number): boolean {
  let start = index
  while (start > 0 && !/[\s"'`()<>]/.test(text[start - 1])) start--
  return URL_TOKEN_RE.test(text.slice(start))
}

/**
 * 扫描普通文本，返回本机图片绝对路径区间（按出现顺序，互不重叠）。
 *
 * 不识别的情况：URL（http://…/a.png）、URL query 内的盘符片段、裸域名/IPv4 开头、
 * 行内代码与 markdown 链接内部、非白名单扩展名。调用方对「整段行内代码就是一条路径」
 * 的场景单独用 isImagePath 判定。
 */
export function extractImagePaths(text: string): ImagePathRange[] {
  const spans = collectSkipSpans(text)
  const out: ImagePathRange[] = []
  const re = new RegExp(ABSOLUTE_PATH_RE.source, 'g')
  let m: RegExpExecArray | null

  while ((m = re.exec(text)) !== null) {
    const candidate = m[0]
    // URL scheme 紧跟其后（http://x/a.png 的首个 / 候选）→ 不识别
    if (URL_SCHEME_RE.test(text.slice(Math.max(0, m.index - 40), m.index))) continue
    if (insideUrlToken(text, m.index)) continue
    // 前一字符是路径字符 → 属于更长候选。宽泛的 / 候选可能吞掉后面的 Windows 路径，
    // 从候选起点后一位续扫，保证盘符仍可被发现（与桌面端同款回退策略）。
    if (m.index > 0 && /[A-Za-z0-9_:\\/]/.test(text[m.index - 1])) {
      re.lastIndex = m.index + 1
      continue
    }
    if (BARE_DOMAIN_RE.test(candidate)) continue
    const ext = IMAGE_EXT_RE.exec(candidate)
    if (!ext || ext.index === undefined) continue
    // 扩展名截断：候选可能把后续正文一并吞进来（路径字符类不含空白终止符）
    const start = m.index
    const end = m.index + ext.index + ext[0].length
    re.lastIndex = end
    if (overlaps(spans, start, end)) continue
    out.push({ start, end })
  }

  return out
}

/**
 * 判定「整串文本恰好是一条本机图片绝对路径」（首尾空白容忍）。
 * 用途：Agent 习惯把路径用反引号包裹（`` `E:\NUS\shot.png` ``），渲染层对
 * 「整段行内代码就是路径」单独走图片通道——与桌面端 MarkdownContent 一致。
 */
export function isImagePath(text: string): boolean {
  const trimmed = text.trim()
  if (trimmed.length === 0) return false
  const ranges = extractImagePaths(trimmed)
  return ranges.length === 1 && ranges[0].start === 0 && ranges[0].end === trimmed.length
}
