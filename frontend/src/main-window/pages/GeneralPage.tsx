/**
 * GeneralPage.tsx — 通用（设置中心分区 `general`）
 *
 * 2026-10-27 由原「语言」页（LanguagePage）改造：语言链路由**原样迁入**（zh/en
 * segmented + localStorage `nuphus_language` + apiSetLanguage，一行未改），并按三组
 * 收纳通用设置（版式参照分组列表：分组标题 + 左标题/右控件行 + 组内细线）：
 *   - 界面：语言（迁移）、会话字号（chat_font_size，注入 CSS 变量仅作用消息正文，
 *     不做全局 UI 缩放）；
 *   - 窗口：关闭按钮行为（close_action）、HUD 悬浮条显示（hud_enabled）、
 *     完成时窗口置前（raise_on_done）；
 *   - 入口：快捷键（弹窗；清单与 HelpPage 同源，可编辑）。
 *
 * 落库链路：get_general_settings / set_* 命令 → UserPreferences（默认 = 历史行为，
 * serde(default) 平滑升级）。开关/下拉先本地乐观更新（点击即刻有反馈），后端归一
 * 结果回传后校准；失败回滚——界面不得与落库状态背离（同 SecurityPage.toggle 纪律）。
 */
import { useEffect, useState } from 'react'
import { FormRow, Section } from '../../ui/PageLayout'
import {
  getLanguage,
  setLanguage as apiSetLanguage,
  getGeneralSettings,
  setCloseAction,
  setChatFontSize,
  setHudEnabled,
  setRaiseOnDone,
} from '../lib/api'
import {
  applyChatFontSize,
  CHAT_FONT_SIZE_DEFAULT,
  CHAT_FONT_SIZE_OPTIONS,
} from '../lib/chatFontSize'
import { useLanguage } from '../../locales'
import { IconChevronRight } from '../../ui/Icons'
import { ShortcutsDialog } from './ShortcutsDialog'
import '../../styles/general-page.css'

const LANG = [
  { id: 'zh', label: 'lang.zh' },
  { id: 'en', label: 'lang.en' },
] as const

const LS_LANG = 'nuphus_language'

/** 关闭行为下拉项；id 与后端 CLOSE_ACTION_* 常量逐一对应（默认 quit 排后） */
const CLOSE_ACTIONS = [
  { id: 'tray', label: 'general.closeAction.tray' },
  { id: 'quit', label: 'general.closeAction.quit' },
] as const

export interface GeneralPageProps {
  /** 快捷键覆盖表变更（弹窗保存 / 恢复后回传；App 层刷新 useKeyboard 生效链路） */
  onShortcutOverridesChange?: (overrides: Record<string, string>) => void
  /** 快捷键录制态广播（App 层挂起 Ctrl+K/F 全局分支，避免录制组合被截胡） */
  onShortcutRecordingChange?: (recording: boolean) => void
}

export function GeneralPage({
  onShortcutOverridesChange,
  onShortcutRecordingChange,
}: GeneralPageProps) {
  const { t, setLang } = useLanguage()
  const [language, setLanguage] = useState('zh')
  const [fontSize, setFontSize] = useState<number>(CHAT_FONT_SIZE_DEFAULT)
  const [closeAction, setCloseActionState] = useState('quit')
  const [hudEnabled, setHudEnabledState] = useState(true)
  const [raiseOnDone, setRaiseOnDoneState] = useState(true)
  const [shortcutsOpen, setShortcutsOpen] = useState(false)

  useEffect(() => {
    // 优先读取后端语言设置，兜底 localStorage（后端返回 zh-CN/en-US，归一化到 zh/en）
    getLanguage()
      .then(backendLang => {
        const raw = backendLang || localStorage.getItem(LS_LANG) || 'zh'
        setLanguage(raw.startsWith('zh') ? 'zh' : 'en')
      })
      .catch(() => {
        const raw = localStorage.getItem(LS_LANG) || 'zh'
        setLanguage(raw.startsWith('zh') ? 'zh' : 'en')
      })
    // 通用偏好：一次读完四项（后端读数均已归一；bridge 无通道时 api 层兜底默认）
    void getGeneralSettings()
      .then(settings => {
        setFontSize(settings.chat_font_size)
        setCloseActionState(settings.close_action)
        setHudEnabledState(settings.hud_enabled)
        setRaiseOnDoneState(settings.raise_on_done)
      })
      .catch(() => {})
  }, [])

  const handleLang = (id: string) => {
    setLanguage(id)
    setLang(id)
    localStorage.setItem(LS_LANG, id)
    apiSetLanguage(id === 'zh' ? 'zh-CN' : 'en-US')
  }

  /** 会话字号：本地即时注入 CSS 变量（本会话立刻可见），落库后按归一值校准。 */
  const handleFontSize = (px: number) => {
    const prev = fontSize
    setFontSize(px)
    applyChatFontSize(px)
    void setChatFontSize(px)
      .then(applied => {
        setFontSize(applied)
        applyChatFontSize(applied)
      })
      .catch(e => {
        console.error('通用偏好保存失败（会话字号）:', e)
        setFontSize(prev)
        applyChatFontSize(prev)
      })
  }

  /** 关闭按钮行为：下拉即时落库；后端归一结果回传后校准。 */
  const handleCloseAction = (action: string) => {
    const prev = closeAction
    setCloseActionState(action)
    void setCloseAction(action)
      .then(applied => setCloseActionState(applied))
      .catch(e => {
        console.error('通用偏好保存失败（关闭行为）:', e)
        setCloseActionState(prev)
      })
  }

  /** 布尔开关的统一落库：乐观更新 + 后端归一校准 + 失败回滚。 */
  const persistToggle = (
    next: boolean,
    prev: boolean,
    setter: (v: boolean) => void,
    save: (v: boolean) => Promise<boolean>,
  ) => {
    setter(next)
    void save(next)
      .then(applied => setter(applied))
      .catch(e => {
        console.error('通用偏好保存失败（开关）:', e)
        setter(prev)
      })
  }

  return (
    <div className="general-page">
      {/* ── 界面：语言（迁移）+ 会话字号 ── */}
      <Section title={t('general.section.interface')}>
        <FormRow
          label={t('app.language')}
          hint={t('general.languageHint')}
          control={
            <div className="select-wrap">
              <select
                className="select"
                aria-label={t('app.language')}
                value={language}
                onChange={e => handleLang(e.target.value)}
              >
                {LANG.map(l => (
                  <option key={l.id} value={l.id}>
                    {t(l.label)}
                  </option>
                ))}
              </select>
            </div>
          }
        />
        <FormRow
          label={t('general.chatFontSize')}
          hint={t('general.chatFontSizeHint')}
          control={
            <div className="select-wrap">
              <select
                className="select"
                aria-label={t('general.chatFontSize')}
                value={fontSize}
                onChange={e => handleFontSize(Number(e.target.value))}
              >
                {CHAT_FONT_SIZE_OPTIONS.map(px => (
                  <option key={px} value={px}>
                    {px}px
                  </option>
                ))}
              </select>
            </div>
          }
        />
      </Section>

      {/* ── 窗口：HUD 悬浮条 + 完成置前 + 关闭行为（组内顺序固定，关闭行为居末） ── */}
      <Section title={t('general.section.window')}>
        <FormRow
          label={t('general.hud')}
          hint={t('general.hudHint')}
          control={
            <button
              type="button"
              role="switch"
              aria-checked={hudEnabled}
              aria-label={t('general.hud')}
              className="switch"
              onClick={() =>
                persistToggle(!hudEnabled, hudEnabled, setHudEnabledState, setHudEnabled)
              }
            />
          }
        />
        <FormRow
          label={t('general.raiseOnDone')}
          hint={t('general.raiseOnDoneHint')}
          control={
            <button
              type="button"
              role="switch"
              aria-checked={raiseOnDone}
              aria-label={t('general.raiseOnDone')}
              className="switch"
              onClick={() =>
                persistToggle(!raiseOnDone, raiseOnDone, setRaiseOnDoneState, setRaiseOnDone)
              }
            />
          }
        />
        <FormRow
          label={t('general.closeAction')}
          hint={t('general.closeActionHint')}
          control={
            <div className="select-wrap">
              <select
                className="select"
                aria-label={t('general.closeAction')}
                value={closeAction}
                onChange={e => handleCloseAction(e.target.value)}
              >
                {CLOSE_ACTIONS.map(item => (
                  <option key={item.id} value={item.id}>
                    {t(item.label)}
                  </option>
                ))}
              </select>
            </div>
          }
        />
      </Section>

      {/* ── 入口：整行可点（右缘 chevron），打开快捷键弹窗 ── */}
      <Section title={t('general.section.access')}>
        <button type="button" className="general-entry" onClick={() => setShortcutsOpen(true)}>
          <span className="general-entry-info">
            <span className="general-entry-label">{t('help.shortcuts')}</span>
            <span className="general-entry-hint">{t('general.shortcutsHint')}</span>
          </span>
          <span className="general-entry-chevron" aria-hidden="true">
            <IconChevronRight size={14} />
          </span>
        </button>
      </Section>

      <ShortcutsDialog
        open={shortcutsOpen}
        onClose={() => setShortcutsOpen(false)}
        onOverridesChange={onShortcutOverridesChange}
        onRecordingChange={onShortcutRecordingChange}
      />
    </div>
  )
}
