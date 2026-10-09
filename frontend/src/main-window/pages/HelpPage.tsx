/**
 * HelpPage.tsx — 帮助（斜杠命令 / 快捷键 / 界面入口 / 运行模式 / 架构）
 *
 * 2026-09-28 按当前源码整页重写：每一条都能在源码里对上号 ——
 *   ① 斜杠命令 = ChatPanel.tsx 的 SLASH_ITEMS（13 条，顺序同源码；
 *      早已移除的 /project 不再出现，游戏入口 /snake 补上）；
 *   ② 快捷键 = App.tsx useKeyboard 的实际绑定（Ctrl+O 是未实现的 TODO 占位，
 *      没有行为就不写进帮助）；
 *   ③ 界面入口 = 聊天区头部齿轮（设置中心）/ 调色板（外观浮窗）/ 左缘色块（会话抽屉）。
 *
 * 改任何一条命令或快捷键时，先改源码，再同步本清单 —— 帮助页的价值就在于「说的都能用」。
 */
import { useLanguage } from '../../locales'
import { Section } from '../../ui/PageLayout'
import '../../styles/help.css'
import { formatPrimaryShortcut, isMacPlatform } from '../../ui/platformShortcut'

/** 斜杠命令：与 ChatPanel.tsx SLASH_ITEMS 逐条一致（顺序同源码） */
const slashCommands = [
  { cmd: '/new', key: 'slash.new' },
  { cmd: '/models', key: 'slash.models' },
  { cmd: '/themes', key: 'slash.themes' },
  { cmd: '/security', key: 'slash.security' },
  { cmd: '/browser', key: 'slash.browser' },
  { cmd: '/memories', key: 'slash.memories' },
  { cmd: '/workflow', key: 'slash.workflow' },
  { cmd: '/skills', key: 'slash.skills' },
  { cmd: '/knowledge', key: 'slash.knowledge' },
  { cmd: '/soul', key: 'slash.soul' },
  { cmd: '/reset', key: 'slash.reset' },
  { cmd: '/help', key: 'slash.help' },
  { cmd: '/snake', key: 'cmd.snakeGameDesc' },
] as const

const archItems = [
  { nameKey: 'help.arch.leader', descKey: 'help.arch.leaderDesc' },
  { nameKey: 'help.arch.exec', descKey: 'help.arch.execDesc' },
  { nameKey: 'help.arch.memory', descKey: 'help.arch.memoryDesc' },
  { nameKey: 'help.arch.workflow', descKey: 'help.arch.workflowDesc' },
] as const

const entryKeys = [
  'help.entries.settings',
  'help.entries.appearance',
  'help.entries.sessions',
  'help.entries.palette',
] as const

const tipKeys = [
  'help.tips.stuck',
  'help.tips.memory',
  'help.tips.skills',
  'help.tips.models',
  'help.tips.settings',
] as const

export function HelpPage() {
  const { t } = useLanguage()
  // 快捷键：与 App.tsx useKeyboard 绑定一一对应（输入框内行为见 ChatPanel.handleKeyDown；
  // IME 组词态例外 = 选词时 Enter 让位给输入法，见表下 help.shortcut.imeHint）
  const shortcuts = [
    { keys: 'Enter', key: 'help.shortcut.send' },
    { keys: `Shift+Enter / ${formatPrimaryShortcut('Enter')}`, key: 'help.shortcut.newline' },
    { keys: formatPrimaryShortcut('K'), key: 'help.shortcut.palette' },
    { keys: formatPrimaryShortcut('L'), key: 'help.shortcut.focusInput' },
    { keys: formatPrimaryShortcut('N'), key: 'help.shortcut.newChat' },
    { keys: `${isMacPlatform() ? 'Cmd' : 'Ctrl'}+Shift+W`, key: 'help.shortcut.workflowPanel' },
    { keys: 'Esc', key: 'help.shortcut.esc' },
  ] as const

  const modes = [
    { name: 'Leader', key: 'help.mode.leader' },
    { name: 'Workflow', key: 'help.mode.workflow' },
  ] as const

  return (
    <div className="help-page">
      {/* ── Slash Commands（输入 / 唤出菜单，可继续输入过滤）── */}
      <Section title={t('help.commands')}>
        <table className="help-table">
          <thead>
            <tr>
              <th>{t('help.command')}</th>
              <th>{t('help.description')}</th>
            </tr>
          </thead>
          <tbody>
            {slashCommands.map(({ cmd, key }) => (
              <tr key={cmd}>
                <td>
                  <span className="help-cmd">{cmd}</span>
                </td>
                <td>{t(key)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Section>

      {/* ── Keyboard Shortcuts ── */}
      <Section title={t('help.shortcuts')}>
        <table className="help-table">
          <thead>
            <tr>
              <th>{t('help.keys')}</th>
              <th>{t('help.action')}</th>
            </tr>
          </thead>
          <tbody>
            {shortcuts.map(({ keys, key }) => (
              <tr key={keys}>
                <td>
                  <span className="help-kbd">{keys}</span>
                </td>
                <td>{t(key)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {/* IME 组词态说明：Enter 发送有例外 —— 组词选字时让位给输入法。
            行为出处同 ChatPanel.handleKeyDown 的 isComposing / keyCode 229 守卫。 */}
        <p className="help-text">{t('help.shortcut.imeHint')}</p>
      </Section>

      {/* ── 界面入口（聊天区常驻的三个入口 + Ctrl+K）── */}
      <Section title={t('help.entries')}>
        <ul className="help-list">
          {entryKeys.map(key => (
            <li key={key}>{t(key)}</li>
          ))}
        </ul>
      </Section>

      {/* ── Runtime Modes ── */}
      <Section title={t('help.modes')}>
        <div className="help-modes">
          {modes.map(({ name, key }) => (
            <div key={name}>
              <span className="help-mode-badge">{name}</span>
              <span className="help-mode-desc">{t(key)}</span>
            </div>
          ))}
        </div>
      </Section>

      {/* ── Architecture ── */}
      <Section title={t('help.arch')}>
        <p className="help-text">{t('help.arch.desc')}</p>
        <ul className="help-list">
          {archItems.map(({ nameKey, descKey }) => (
            <li key={nameKey}>
              <strong>{t(nameKey)}</strong> — {t(descKey)}
            </li>
          ))}
        </ul>
      </Section>

      {/* ── Tips ── */}
      <Section title={t('help.tips')}>
        <ul className="help-list">
          {tipKeys.map(key => (
            <li key={key}>{t(key)}</li>
          ))}
        </ul>
      </Section>
    </div>
  )
}
