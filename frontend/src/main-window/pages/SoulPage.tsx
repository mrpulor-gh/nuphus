/**
 * SoulPage.tsx — 灵魂（身份：头像 + 称呼，单模块）
 *
 * 大王 2026-09-28 定稿：头像与关系本来就是一件事（「谁在说话、长什么样、叫什么」），
 * 不拆两个模块照搬照抄 —— 一个模块内：
 *   ① 「在消息中显示头像」总开关；
 *   ② 用户 / Nuphus 两条身份行：**点头像即上传**（pickAndImportImage 入库只存路径），
 *      「称呼」即关系字段 —— 保存时 saveRelation 落 localStorage + setRelation
 *      同步后端 relation.json（手机端 /identity 经 relation_cache 下发显示名）。
 *
 * 头像渲染路径 2026-09-30 改（issue #94）：原先是同步 `toAssetUrl(src)` 直接交给
 * `<img>`，而 asset 协议当时从未被编译进来，convertFileSrc 产出的 URL 无人应答，
 * `<img>` 静默 onerror → 表现为「破图 + 零提示」。现改为 useAvatarUrl 异步解出
 * （asset:// 优先 → blob: 兜底），两条都失败则落回字母头像，失败有明确视觉信号。
 *
 * 主题相关的外观调整不在这里：见 layout/AppearancePanel.tsx。
 */
import { useEffect, useState } from 'react'
import { loadRelation, saveRelation, type RelationConfig } from '../lib/relation'
import { setRelation } from '../lib/api'
import { Button } from '../../ui/Button'
import { LetterAvatar } from '../../ui/LetterAvatar'
import { releaseSkinImageUrl, resolveAvatarImageUrl } from '../../ui/assetUrl'
import { Section, FormRow } from '../../ui/PageLayout'
import { pickAndImportImage } from '../lib/localImage'
import { useLanguage } from '../../locales'
/* 头像位样式（.avatar-preview / .avatar-preview--upload / .soul-name-input）
   定义在 themes.css（与主题面板同一份样式文件的头像小节） */
import '../../styles/themes.css'

const LS_SHOW_AVATAR = 'nuphus_show_avatar'
const LS_USER_AVATAR = 'nuphus_user_avatar'
const LS_NUPHUS_AVATAR = 'nuphus_nuphus_avatar'

/** 两条身份行同构（头像位 + 称呼字段成对），仅数据不同 —— 配置化避免照搬。
    侧别不配可见文字（大王定稿）：由头像本身（U / A 字母或上传图）与固定次序区分 */
const IDENTITY_SIDES = [
  { side: 'user', nameKey: 'userLabel', placeholder: 'USER' },
  { side: 'nuphus', nameKey: 'assistantName', placeholder: 'Nuphus' },
] as const satisfies readonly {
  side: 'user' | 'nuphus'
  nameKey: keyof RelationConfig
  placeholder: string
}[]

/**
 * 把 localStorage 里的头像路径解成可渲染 URL。
 *
 * 为什么不能像原来那样同步 `<img src={toAssetUrl(src)}>`：asset 协议未启用时
 * convertFileSrc 交出去的 URL 无人应答，`<img>` 只会静默 onerror —— 就是
 * issue #94 的「破图 + 零提示」。改成异步解出：asset:// 优先、blob: 兜底，
 * 两条都失败（文件没了 / 读不了）则返回 null，由调用方落回字母头像 ——
 * 失败从此有明确视觉信号，不再是浏览器默认破图图标。
 */
function useAvatarUrl(src: string): string | null {
  const [url, setUrl] = useState<string | null>(null)
  useEffect(() => {
    if (!src) {
      setUrl(null)
      return
    }
    let alive = true
    void resolveAvatarImageUrl(src).then(u => {
      if (alive) setUrl(u)
    })
    return () => {
      alive = false
    }
  }, [src])
  useEffect(() => {
    if (!url?.startsWith('blob:')) return
    return () => releaseSkinImageUrl(url)
  }, [url])
  return url
}

/**
 * 头像位：异步解出 src → 有值渲染 `<img>`，无值（未设置或解出失败）落回字母头像。
 *
 * 抽成组件而非行内调用，是因为 useAvatarUrl 内部是 hook —— 在 `.map()` 里直接
 * 调用会让 hook 顺序随条目数变化，违反 React 规则。
 */
function AvatarSlot({ src, side }: { src: string; side: 'user' | 'nuphus' }) {
  const url = useAvatarUrl(src)
  if (!src || !url) {
    return <LetterAvatar letter={side === 'user' ? 'U' : 'A'} size={34} />
  }
  return <img src={url} alt="" />
}

export function SoulPage({ onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [rel, setRel] = useState(() => loadRelation())
  const [saved, setSaved] = useState(false)
  const [showAvatar, setShowAvatar] = useState(false)
  const [userAvatar, setUserAvatar] = useState('')
  const [nuphusAvatar, setNuphusAvatar] = useState('')

  useEffect(() => {
    setShowAvatar(localStorage.getItem(LS_SHOW_AVATAR) === 'true')
    setUserAvatar(localStorage.getItem(LS_USER_AVATAR) || '')
    setNuphusAvatar(localStorage.getItem(LS_NUPHUS_AVATAR) || '')
  }, [])

  const handleSave = () => {
    saveRelation(rel)
    // 同步持久化到后端 relation.json（手机端 /identity 经 relation_cache 下发显示名）
    void setRelation(rel).catch(() => {})
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  const update = (key: keyof RelationConfig, val: string) => {
    setRel(prev => ({ ...prev, [key]: val }))
    setSaved(false)
  }

  const handleAvatarSelect = async (type: 'user' | 'nuphus') => {
    // 与皮肤背景同一套本地架构：入库到磁盘、只存路径（见 lib/localImage.ts）
    try {
      const path = await pickAndImportImage()
      if (!path) return
      const key = type === 'user' ? LS_USER_AVATAR : LS_NUPHUS_AVATAR
      const setter = type === 'user' ? setUserAvatar : setNuphusAvatar
      setter(path)
      localStorage.setItem(key, path)
    } catch (e) {
      console.error('头像入库失败:', e)
      alert(t('themes.avatarImportFailed'))
    }
  }

  const clearAvatar = (type: 'user' | 'nuphus') => {
    const key = type === 'user' ? LS_USER_AVATAR : LS_NUPHUS_AVATAR
    ;(type === 'user' ? setUserAvatar : setNuphusAvatar)('')
    localStorage.removeItem(key)
  }

  const handleToggleAvatar = () => {
    const next = !showAvatar
    setShowAvatar(next)
    localStorage.setItem(LS_SHOW_AVATAR, String(next))
  }

  return (
    <Section title={t('app.soul')}>
      {/* ① 总开关：消息里是否显示头像 */}
      <FormRow
        label={t('themes.showAvatar')}
        control={
          <button
            type="button"
            role="switch"
            aria-checked={showAvatar}
            className="switch"
            onClick={handleToggleAvatar}
          />
        }
      />

      {/* ② 两条身份行：头像（点击即上传）+ 称呼输入，无解释性文字 */}
      {IDENTITY_SIDES.map(({ side, nameKey, placeholder }) => {
        const avatar = side === 'user' ? userAvatar : nuphusAvatar
        return (
          <FormRow
            key={side}
            label={
              <button
                type="button"
                className="avatar-preview avatar-preview--upload"
                title={t('themes.upload')}
                aria-label={t('themes.upload')}
                onClick={() => handleAvatarSelect(side)}
              >
                <AvatarSlot src={avatar} side={side} />
              </button>
            }
            control={
              <>
                <input
                  className="input soul-name-input"
                  value={rel[nameKey]}
                  onChange={e => update(nameKey, e.target.value)}
                  placeholder={placeholder}
                  aria-label={t('soul.nameField')}
                />
                {avatar && (
                  <Button variant="danger" size="sm" onClick={() => clearAvatar(side)}>
                    {t('themes.clearBg')}
                  </Button>
                )}
              </>
            }
          />
        )
      })}

      <div className="form-footer">
        {saved && <span className="badge badge-success">{t('common.saved')}</span>}
        <Button variant="primary" onClick={handleSave}>
          {t('common.save')}
        </Button>
      </div>
    </Section>
  )
}
