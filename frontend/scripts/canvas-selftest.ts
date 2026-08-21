/**
 * canvas-selftest.ts — 投影层自测（质量基线 3）
 *
 * 用 plugin/workflows/ 下真实 workflow JSON 跑 projection，
 * 断言输出与设计文档 1.8 实例结构一致。
 *
 * 运行（frontend/ 目录）：
 *   npx esbuild scripts/canvas-selftest.ts --bundle --platform=node --format=cjs --outfile=scripts/.canvas-selftest.cjs
 *   node scripts/.canvas-selftest.cjs
 */

import { projectWorkflow } from '../src/main-window/workflow-canvas/projection'
import { layoutLayer } from '../src/main-window/workflow-canvas/layout'

// 无 @types/node 环境的最小声明（tsc 直编 CJS 运行）
declare const require: (m: string) => any
declare const process: { exit(code: number): void }
declare const __dirname: string
const { readFileSync } = require('fs')
const { join } = require('path')

// 编译产物位于 frontend/scripts/（.canvas-selftest.cjs），上溯 2 级到仓库根
const WF_DIR = join(__dirname, '..', '..', 'plugin', 'workflows')

let failures = 0
function assert(cond: boolean, msg: string): void {
  if (cond) {
    console.log(`  ✓ ${msg}`)
  } else {
    failures++
    console.error(`  ✗ ${msg}`)
  }
}

function loadJson(p: string): any {
  return JSON.parse(readFileSync(p, 'utf-8'))
}

// ── 实例：nuphus-tour-v4（设计文档 1.8）──

const ir = loadJson(join(WF_DIR, 'nuphus-tour-v4', 'workflow.json'))
const proj = projectWorkflow(ir)

console.log('L0 Root 层')
const root = proj.layers.get('root')!
assert(!!root, 'root 层存在')
const rootIds = root.nodes.map(n => n.id)
assert(rootIds.includes('init') && rootIds.includes('tour'), 'root 层含 init/tour 节点')
const initNode = root.nodes.find(n => n.id === 'init')!
assert(initNode.category === 'container' && initNode.childCount === 2, 'init 是 seq 容器 · 2 步')
// 布局回归：dagre 原地写坐标，nodeSize 必须返回新对象，否则同类节点坐标互相覆盖
// root 层 rankdir=LR（同级横向流）：init→tour 相邻 rank → 同 y（容差 1px）且 tour 在 init 右侧
const rootPos = layoutLayer(root, undefined)
const initPos = rootPos.get('init')!
const tourPos = rootPos.get('tour')!
assert(
  Math.abs(initPos.y - tourPos.y) <= 1 && tourPos.x > initPos.x,
  `root 层 LR 横向布局（init(${initPos.x},${initPos.y}) → tour(${tourPos.x},${tourPos.y}) 同 y 右移）`,
)
const tourNode = root.nodes.find(n => n.id === 'tour')!
assert(
  tourNode.category === 'container' &&
    tourNode.childCount === 7 &&
    (tourNode.containerSummary ?? '').includes('panels') &&
    (tourNode.containerSummary ?? '').includes('p'),
  `tour 是 loop 容器 · 7 步 · for_each panels as p（摘要: ${tourNode.containerSummary}）`,
)
assert(
  root.edges.some(e => e.kind === 'sequence' && e.source === 'init' && e.target === 'tour'),
  '顺序边 init → tour',
)
const crossEdge = root.edges.find(
  e => e.kind === 'data' && e.source === 'init' && e.target === 'tour' && e.label === 'panels',
)
assert(!!crossEdge, '跨层数据边 init → tour（标签 panels）')
assert(crossEdge?.producerStepId === 'init_compute', '跨层边真实生产者 = init_compute')

console.log('L1 init 子层')
const initLayer = proj.layers.get('init')!
assert(!!initLayer, 'init 子层存在')
assert(
  initLayer.edges.some(e => e.kind === 'sequence' && e.source === 'init_wins' && e.target === 'init_compute'),
  '顺序边 init_wins → init_compute',
)
assert(
  initLayer.edges.some(
    e => e.kind === 'data' && e.source === 'init_wins' && e.target === 'init_compute' && e.label === 'wins',
  ),
  '数据边 init_wins ~~wins~~> init_compute（script code 内含 {{wins}}）',
)
assert(
  initLayer.nodes.some(n => n.synthetic === 'external' && n.externalVar === 'panels') &&
    initLayer.edges.some(
      e => e.kind === 'external' && e.source === 'init_compute' && e.label === 'panels',
    ),
  '出口数据边 init_compute ~~panels~~> 层外去向锚点',
)

console.log('L1 tour 子层')
const tourLayer = proj.layers.get('tour')!
assert(!!tourLayer, 'tour 子层存在')
const entryId = 'tour::entry'
assert(tourLayer.nodes.some(n => n.id === entryId && n.synthetic === 'entry'), '入口锚点存在')
const tourStepIds = tourLayer.nodes.filter(n => !n.synthetic).map(n => n.id)
assert(tourStepIds.length === 7, `循环体 7 步（实际 ${tourStepIds.length}）`)
const loopback = tourLayer.edges.find(e => e.kind === 'loopback')
assert(
  !!loopback && loopback.target === entryId && loopback.source === tourStepIds[tourStepIds.length - 1],
  '回环装饰边：尾节点 → 入口锚点',
)
assert(loopback?.label === 'max 100', `回环边标注 max 100（实际 ${loopback?.label}）`)
for (const consumer of ['activate', 'open_cmd', 'close_mask']) {
  assert(
    tourLayer.edges.some(
      e => e.kind === 'data' && e.source === entryId && e.target === consumer && e.label === 'p',
    ),
    `锚点边 入口 ~~p~~> ${consumer}`,
  )
}

console.log('索引')
assert(proj.index.parentOf.get('init_wins') === 'init', 'parentOf: init_wins 的父是 init')
assert(proj.index.parentOf.get('init') === null, 'parentOf: init 的父是 null（root）')
assert(proj.index.layerOf.get('activate') === 'tour', 'layerOf: activate 在 tour 层')
assert(!proj.index.hasCustomNodes, 'V2 数据无 Custom 节点')
assert(proj.index.containerIds.has('init') && proj.index.containerIds.has('tour'), '容器索引含 init/tour')

// ── 旧格式检测（R1/V13）──

console.log('旧格式识别（nuphus-tour-v4.json legacy）')
const legacy = loadJson(join(WF_DIR, 'nuphus-tour-v4', 'nuphus-tour-v4.json'))
const legacyProj = projectWorkflow(legacy)
assert(legacyProj.index.hasCustomNodes, '旧格式被识别为 Custom（整树只读 + 迁移横幅）')
const legacyRoot = legacyProj.layers.get('root')!
assert(
  legacyRoot.nodes.every(n => n.category === 'unknown'),
  '旧格式节点全部渲染为 unknown（不兼容）类别',
)

// ── chatagent-opencode-verify（V2 单 chat 步骤）──

console.log('chatagent-opencode-verify')
const chat = loadJson(join(WF_DIR, 'chatagent-opencode-verify', 'workflow.json'))
const chatProj = projectWorkflow(chat)
assert(!chatProj.index.hasCustomNodes, 'chat workflow 无 Custom 节点')
assert((chatProj.layers.get('root')?.nodes.length ?? 0) === 1, 'root 层 1 个节点')

console.log(failures === 0 ? '\n全部断言通过' : `\n${failures} 条断言失败`)
process.exit(failures === 0 ? 0 : 1)