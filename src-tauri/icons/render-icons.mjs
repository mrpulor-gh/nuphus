// render-icons.mjs — 用 headless Chrome 精确渲染 icon-source.svg 为各尺寸 PNG/ICO。
//
// 为什么不用 PIL 手绘几何：SVG 的 A（弧）命令 + round linecap + stroke 缩放
// 手写近似极易走形（第一版就把开口壳画成了「口」形）。交给浏览器渲染 = 语义精确。
//
// 用法：node render-icons.mjs
// 前置：本机装有 Chrome（msedge/chrome 任一即可，脚本自动探测）

import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, rmSync, writeFileSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const svgPath = join(here, 'icon-source.svg')
const svg = readFileSync(svgPath, 'utf8')

/** 探测可用的 Chromium 内核浏览器 */
function findBrowser() {
  const candidates = [
    'C:/Program Files/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
    'C:/Program Files/Microsoft/Edge/Application/msedge.exe',
  ]
  for (const p of candidates) if (existsSync(p)) return p
  throw new Error('未找到 Chrome/Edge，无法渲染 SVG')
}

const browser = findBrowser()

/** 用 headless 截图把 SVG 渲染成指定尺寸的 PNG（返回 Buffer） */
function renderPng(size, outFile) {
  // 用 data URI 内嵌 SVG，页面 body 无边距，精确 size×size
  const dataUri = `data:image/svg+xml;base64,${Buffer.from(svg, 'utf8').toString('base64')}`
  const html = `<!doctype html><html><head><meta charset="utf-8"><style>
    html,body{margin:0;padding:0;width:${size}px;height:${size}px;overflow:hidden;background:transparent}
    img{width:${size}px;height:${size}px;display:block}
  </style></head><body><img src="${dataUri}" alt=""></body></html>`
  // 内联 HTML 写成临时文件，用 file:// 加载（data: URI 的顶层导航在 headless 下受限）
  const tmpHtml = join(here, `.render-${size}.html`)
  writeFileSync(tmpHtml, html, 'utf8')
  execFileSync(
    browser,
    [
      '--headless',
      '--disable-gpu',
      '--hide-scrollbars',
      '--default-background-color=00000000', // 透明背景
      `--screenshot=${outFile}`,
      `--window-size=${size},${size}`,
      `file:///${tmpHtml.replace(/\\/g, '/')}`,
    ],
    { stdio: 'ignore', timeout: 30000 },
  )
  rmSync(tmpHtml, { force: true })
  if (!existsSync(outFile)) throw new Error(`渲染失败: ${outFile}`)
  return outFile
}

const PNG_TARGETS = [
  ['32x32.png', 32],
  ['128x128.png', 128],
  ['128x128@2x.png', 256],
  ['64x64.png', 64],
  ['icon.png', 512],
  ['Square30x30Logo.png', 30],
  ['Square44x44Logo.png', 44],
  ['Square71x71Logo.png', 71],
  ['Square89x89Logo.png', 89],
  ['Square107x107Logo.png', 107],
  ['Square142x142Logo.png', 142],
  ['Square150x150Logo.png', 150],
  ['Square284x284Logo.png', 284],
  ['Square310x310Logo.png', 310],
  ['StoreLogo.png', 50],
]

console.log(`browser: ${browser}`)
console.log('渲染 PNG…')
for (const [name, size] of PNG_TARGETS) {
  renderPng(size, join(here, name))
  console.log(`  ${name.padEnd(26)} ${size}x${size}`)
}

// ICO：PIL 侧合成（多帧容器），复用已渲染的最大尺寸 PNG
console.log('合成 ICO…')
const ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
const tmpDir = join(here, '.ico-src')
mkdirSync(tmpDir, { recursive: true })
for (const s of ICO_SIZES) renderPng(s, join(tmpDir, `${s}.png`))

// 直接按 ICO 文件格式写多帧容器（PIL 的 sizes/append_images 对 ICO 不可靠，
// 实测只出 1 帧）。ICO 结构：6 字节头 + 每帧 16 字节目录项 + 各帧 DIB 数据。
// 注意：本字符串会在 node 里被当模板解析，故内部只用 ASCII（注释放外面）。
const py = `
import os, struct
from PIL import Image

d = r"${tmpDir.replace(/\\/g, '\\\\')}"
out = r"${join(here, 'icon.ico').replace(/\\/g, '\\\\')}"
sizes = [16, 24, 32, 48, 64, 128, 256]

def dib(size):
    im = Image.open(os.path.join(d, f"{size}.png")).convert("RGBA")
    w = h = size
    hdr = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    px = im.tobytes("raw", "BGRA")
    # ICO/BMP spec stores DIB rows BOTTOM-UP, but PNG (and PIL tobytes) is TOP-DOWN.
    # Writing the PNG row order straight into the DIB renders the icon vertically
    # mirrored in Explorer/taskbar/tray, which moved the shell opening to the wrong
    # corner (TOP-RIGHT instead of BOT-RIGHT). Flip the row order here.
    # Verify with an independent spec-compliant reader (PIL), not by re-decoding
    # the bytes top-down -- that comparison is self-consistent and hides the bug.
    rowb = w * 4
    rows = [px[i * rowb:(i + 1) * rowb] for i in range(h)]
    px = b"".join(rows[::-1])
    row = ((w + 31) // 32) * 4
    mask = b"\\x00" * (row * h)
    return hdr + px + mask

entries, datas = [], []
offset = 6 + 16 * len(sizes)
for s in sizes:
    data = dib(s)
    w = h = s if s < 256 else 0
    entries.append(struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(data), offset))
    datas.append(data)
    offset += len(data)

with open(out, "wb") as f:
    f.write(struct.pack("<HHH", 0, 1, len(sizes)))
    for e in entries:
        f.write(e)
    for data in datas:
        f.write(data)

b = open(out, "rb").read()
print("ICO frames written:", struct.unpack("<H", b[4:6])[0], "bytes:", len(b))
`
writeFileSync(join(tmpDir, 'mk.py'), py, 'utf8')
execFileSync('python', [join(tmpDir, 'mk.py')], { stdio: 'inherit' })
rmSync(tmpDir, { recursive: true, force: true })
console.log('  icon.ico                ICO [16,24,32,48,64,128,256]')
console.log('完成')