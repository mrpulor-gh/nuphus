/**
 * PluginComingSoon.tsx — 插件市场筹备提示（市场体系不开源阶段的入口落点）
 *
 * 生命周期：ctrl+k →「插件」→ CompactModal 弹窗内渲染本组件，仅展示筹备文案 + 官方 GitHub；
 * 不加载市场模块、不展示分类/搜索/已安装/市场区块（大王拍板「其它什么都没有」）。
 * 市场 ready 后恢复 PluginAppsPage 全屏面板（App.tsx 有可逆注释说明）。
 *
 * 视觉结构（信息阶梯 ≤3 层）：
 *   L1 状态区 — 图标 + 主标题（居中、克制）
 *   L2 说明区 — 两张信息卡片（左对齐：图标块 + 标题 + 描述），替代原两段同层级长文本
 *   L3 行动区 — GitHub 链接按钮化（静态可见容器 + hover/active/focus 三态）
 */
import { useLanguage } from '../../locales'
import { IconStore, IconSparkles, IconGlobe, IconExternalLink } from '../../ui/Icons'
import './plugin-coming-soon.css'

const GITHUB_URL = 'https://github.com/mrpulor-gh/nuphus'

export function PluginComingSoon() {
  const { t } = useLanguage()
  return (
    <div className="plugin-coming-soon">
      {/* ── L1 状态区 ── */}
      <div className="plugin-coming-soon-hero">
        <IconStore size={32} className="plugin-coming-soon-icon" />
        <p className="plugin-coming-soon-title">{t('plugins.comingSoonP1')}</p>
      </div>

      {/* ── L2 说明卡片区 ── */}
      <div className="plugin-coming-soon-cards">
        <div className="plugin-coming-soon-card">
          <div className="plugin-coming-soon-card-icon">
            <IconSparkles size={15} />
          </div>
          <div className="plugin-coming-soon-card-body">
            <div className="plugin-coming-soon-card-title">{t('plugins.comingSoonCard1Title')}</div>
            <p className="plugin-coming-soon-card-desc">{t('plugins.comingSoonP2')}</p>
          </div>
        </div>
        <div className="plugin-coming-soon-card">
          <div className="plugin-coming-soon-card-icon">
            <IconGlobe size={15} />
          </div>
          <div className="plugin-coming-soon-card-body">
            <div className="plugin-coming-soon-card-title">{t('plugins.comingSoonCard2Title')}</div>
            <p className="plugin-coming-soon-card-desc">{t('plugins.comingSoonP3')}</p>
          </div>
        </div>
      </div>

      {/* ── L3 行动区 ── */}
      <a
        className="plugin-coming-soon-github"
        href={GITHUB_URL}
        target="_blank"
        rel="noreferrer"
      >
        <IconExternalLink size={14} />
        <span>{t('plugins.comingSoonGithub')}</span>
      </a>
    </div>
  )
}