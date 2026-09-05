/**
 * intentText.test.ts — buildIntentTextTemplate 纯文本模板测试
 * 对照 docs/intent-form-spec.md §3 三场景样例（简单 3 步 / 含阶段嵌套 / 混合探索）。
 */
import { describe, expect, it } from 'vitest'
import { buildIntentTextTemplate } from './intentText'
import type { IntentForm } from './intentTypes'

function stage(name: string, intents: string[]) {
  return {
    id: Math.random().toString(36).slice(2),
    name,
    steps: intents.map(i => ({ id: Math.random().toString(36).slice(2), intent: i })),
  }
}

const FOOTER_REQUIRE = /要求：意图解析、步骤前置与重置、循环合并、异常分支、缺失参数补偿；/
const FOOTER_OVERWRITE =
  /完成后覆写 plugin\/workflows\/wf-\d+\/workflow\.json（保留 id=wf-\d+）并跑通验证。/

describe('buildIntentTextTemplate', () => {
  it('场景 1：简单 3 步（无阶段嵌套 → 编号平铺）', () => {
    const form: IntentForm = {
      workflowName: '每日股价提醒',
      stages: [
        stage('', [
          '打开券商 App 首页',
          '读取自选股列表当前价格',
          '若有单只跌幅超过 3%，向我的手机发送提醒',
        ]),
      ],
    }
    const out = buildIntentTextTemplate(form, 'wf-001')
    expect(
      out.startsWith(
        '[意图表单→工作流] 请把以下意图整理为可执行 V2 工作流「每日股价提醒」（id=wf-001）：',
      ),
    ).toBe(true)
    expect(out).toContain('1. 打开券商 App 首页')
    expect(out).toContain('2. 读取自选股列表当前价格')
    expect(out).toContain('3. 若有单只跌幅超过 3%，向我的手机发送提醒')
    // 简单场景：步骤紧贴头部行，无空行、无阶段标题
    expect(out).not.toContain('阶段1')
    expect(out).toMatch(FOOTER_REQUIRE)
    expect(out).toMatch(FOOTER_OVERWRITE)
  })

  it('场景 2：含阶段嵌套（阶段名 + 缩进子步骤）', () => {
    const form: IntentForm = {
      workflowName: '新品上架巡检',
      stages: [
        stage('登录管理后台', [
          '打开浏览器访问商家后台地址',
          '用已保存的账号登录（如遇验证码请提示我手动处理）',
        ]),
        stage('巡检每个在售商品', [
          '循环读取商品列表',
          '对每个商品检查价格与库存',
          '价格低于成本或库存为 0 时记录到异常清单',
        ]),
        stage('汇总并通知', ['汇总异常清单', '发送摘要到工作群']),
      ],
    }
    const out = buildIntentTextTemplate(form, 'wf-002')
    expect(
      out.startsWith(
        '[意图表单→工作流] 请把以下意图整理为可执行 V2 工作流「新品上架巡检」（id=wf-002）：',
      ),
    ).toBe(true)
    expect(out).toContain('阶段1「登录管理后台」')
    expect(out).toContain('- 打开浏览器访问商家后台地址')
    expect(out).toContain('阶段2「巡检每个在售商品」')
    expect(out).toContain('- 价格低于成本或库存为 0 时记录到异常清单')
    expect(out).toContain('阶段3「汇总并通知」')
    // 阶段块之间以空行分隔（场景 2 样例：块间 + 尾块后接要求均空行）
    expect(out).toContain('价格低于成本或库存为 0 时记录到异常清单\n\n阶段3「汇总并通知」')
    expect(out).toContain('发送摘要到工作群\n\n要求：意图解析')
    expect(out).toMatch(FOOTER_REQUIRE)
  })

  it('场景 3：混合（阶段名 + 需探索确认标注的步骤）', () => {
    const form: IntentForm = {
      workflowName: '日报导出',
      stages: [
        stage('打开报表页', [
          '打开浏览器进入 https://console.example.com/report（登录态若变化请先探索确认入口）',
        ]),
        stage('筛选并导出', ['按昨日日期筛选（筛选控件位置请探索确认）', '导出 Excel 到下载目录']),
        stage('整理', ['读取下载的 Excel 并生成摘要']),
      ],
    }
    const out = buildIntentTextTemplate(form, 'wf-003')
    expect(out).toContain('「日报导出」（id=wf-003）')
    expect(out).toContain('阶段1「打开报表页」')
    expect(out).toContain(
      '- 打开浏览器进入 https://console.example.com/report（登录态若变化请先探索确认入口）',
    )
    expect(out).toContain('阶段2「筛选并导出」')
    expect(out).toContain('- 按昨日日期筛选（筛选控件位置请探索确认）')
    expect(out).toContain('阶段3「整理」')
    expect(out).toMatch(FOOTER_OVERWRITE)
  })

  it('fallback 名称：workflowName 空时用 fallbackName；再缺省「未命名工作流」', () => {
    const form: IntentForm = { workflowName: '', stages: [stage('', ['做一件事'])] }
    expect(buildIntentTextTemplate(form, 'wf-009', '兜底名')).toContain('「兜底名」（id=wf-009）')
    expect(buildIntentTextTemplate(form, 'wf-009')).toContain('「未命名工作流」（id=wf-009）')
  })

  it('空行子步骤在渲染时剔除；全空表单只出头部+要求', () => {
    const form: IntentForm = {
      workflowName: '清理空行',
      stages: [stage('阶段甲', ['有效步骤', '   ', '']), stage('阶段乙', [''])],
    }
    const out = buildIntentTextTemplate(form, 'wf-010')
    expect(out).toContain('- 有效步骤')
    expect(out).not.toContain('阶段乙')
    const empty: IntentForm = { workflowName: '空', stages: [] }
    const outEmpty = buildIntentTextTemplate(empty, 'wf-011')
    expect(outEmpty).toContain('[意图表单→工作流]')
    expect(outEmpty).toContain('要求：')
  })

  it('多阶段中空阶段剔除后仍保留后续阶段序号连续（不因空阶段跳号）', () => {
    const form: IntentForm = {
      workflowName: '序号连续',
      stages: [stage('第一步', ['a1']), stage('空容器', []), stage('第三步', ['c1'])],
    }
    const out = buildIntentTextTemplate(form, 'wf-012')
    expect(out).toContain('阶段1「第一步」')
    expect(out).toContain('阶段2「第三步」') // 空阶段不计入
    expect(out).not.toContain('阶段3')
  })
})
