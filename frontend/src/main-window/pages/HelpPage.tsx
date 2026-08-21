import { useLanguage } from '../../locales'
import { Section } from '../../ui/PageLayout'
import '../../styles/help.css'

const slashCommands = [
  { cmd: '/new', key: 'slash.new' },
  { cmd: '/memories', key: 'slash.memories' },
  { cmd: '/skills', key: 'slash.skills' },
  { cmd: '/knowledge', key: 'slash.knowledge' },
  { cmd: '/models', key: 'slash.models' },
  { cmd: '/themes', key: 'slash.themes' },
  { cmd: '/project', key: 'slash.project' },
  { cmd: '/security', key: 'slash.security' },
  { cmd: '/browser', key: 'slash.browser' },
  { cmd: '/soul', key: 'slash.soul' },
  { cmd: '/workflow', key: 'slash.workflow' },
  { cmd: '/reset', key: 'slash.reset' },
  { cmd: '/help', key: 'slash.help' },
] as const

const shortcuts = [
  { keys: 'Enter', key: 'help.shortcut.send' },
  { keys: 'Shift+Enter', key: 'help.shortcut.newline' },
  { keys: 'Ctrl+K', key: 'help.shortcut.palette' },
  { keys: 'Ctrl+Enter', key: 'help.shortcut.altSend' },
  { keys: 'Esc', key: 'help.shortcut.esc' },
] as const

const modes = [
  { name: 'Leader', key: 'help.mode.leader' },
  { name: 'Workflow', key: 'help.mode.workflow' },
] as const

const archItems = [
  { nameKey: 'help.arch.leader', descKey: 'help.arch.leaderDesc' },
  { nameKey: 'help.arch.exec', descKey: 'help.arch.execDesc' },
  { nameKey: 'help.arch.memory', descKey: 'help.arch.memoryDesc' },
  { nameKey: 'help.arch.workflow', descKey: 'help.arch.workflowDesc' },
] as const

export function HelpPage() {
  const { t } = useLanguage()

  return (
    <div className="help-page">
      {/* ── Slash Commands ── */}
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

      {/* ── Runtime Modes ── */}
      <Section title={t('help.modes')}>
        <div className="help-modes">
          {modes.map(({ name, key }) => (
            <div className="help-mode-item" key={name}>
              <span className="help-mode-badge">{name}</span>
              <span className="help-mode-desc">{t(key)}</span>
            </div>
          ))}
        </div>
      </Section>

      {/* ── Tips ── */}
      <Section title={t('help.tips')}>
        <ul className="help-list">
          <li>{t('help.tips.stuck')}</li>
          <li>{t('help.tips.memory')}</li>
          <li>{t('help.tips.project')}</li>
          <li>{t('help.tips.skills')}</li>
          <li>{t('help.tips.models')}</li>
        </ul>
      </Section>
    </div>
  )
}