/**
 * sync-overlay-script.mjs — 把 tools/overlay-script.src.js 同步进
 * src-tauri/src/preview_protocol.rs 的 ANNOTATION_OVERLAY_SCRIPT 原始字符串。
 *
 * 为什么需要这一步（2026-10-08 实机事故的直接教训）：
 *   注入脚本约 18KB 的 JS 此前只有一处副本（.rs 里的 r####"..."#### 原始字符串），
 *   rustfmt 会顺手重排它，编辑时锚点漂移过一次就把字符串截断成语法残缺——
 *   而 cargo test 只做子串断言、前端 vitest 从不执行这段 JS，残缺能一路绿灯到底。
 *   改为「JS 独立成文件 + 本脚本做纯文本搬运 + new Function 真实解析」：
 *   搬运是确定性字符串操作（不手写大段 .rs），解析由 node 在搬运前先跑一次。
 *
 * 用法：node frontend/tools/sync-overlay-script.mjs [--check]
 *   --check 只校验，不写文件（CI / 校验脚本用；漂移时非零退出）
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const repoRoot = join(here, '..', '..')
const jsPath = join(here, 'overlay-script.src.js')
const rsPath = join(repoRoot, 'src-tauri', 'src', 'preview_protocol.rs')

const HEAD = 'const ANNOTATION_OVERLAY_SCRIPT: &str = r####"<script>\n'
const TAIL = '</script>"####;'

const js = readFileSync(jsPath, 'utf8').replace(/\r\n/g, '\n')
if (js.includes('####')) throw new Error('脚本内含 ####，会提前闭合 Rust 原始字符串')
// 真实解析：这是唯一能拦住「JS 语法错但 cargo test 全绿」的检查
new Function(js)

const block = HEAD + js + TAIL
const src = readFileSync(rsPath, 'utf8').replace(/\r\n/g, '\n')
const start = src.indexOf(HEAD)
const endMarker = TAIL
const end = src.indexOf(endMarker, start)
if (start < 0 || end < 0)
  throw new Error('未在 preview_protocol.rs 中定位到 ANNOTATION_OVERLAY_SCRIPT')
const next = src.slice(0, start) + block + src.slice(end + endMarker.length)

if (next === src.replace(/\r\n/g, '\n')) {
  console.log('overlay script: in sync')
  process.exit(0)
}
if (process.argv.includes('--check')) {
  console.error('overlay script: OUT OF SYNC with tools/overlay-script.src.js')
  process.exit(1)
}
writeFileSync(rsPath, next, 'utf8')
console.log('overlay script: synced into preview_protocol.rs')
