/**
 * LetterAvatar — 字母头像（聊天默认头像）
 *
 * 设计意图（克制：纯字体排印，不加任何装饰）:
 *   用户侧 = 「U」，智能体侧 = 「A」。两者**完全同构**，仅字母不同 ——
 *   角色辨识只靠字母本身，不靠底色差异、不靠描边、不靠角标。
 *
 * 为什么替换 NuphusLogo:
 *   原默认头像两侧共用同一个 `<NuphusLogo variant="mark">`（与 TitleBar 同款），
 *   为区分用户侧还把 logo 旋转 180°「拟人形」（themes.css 的
 *   `.avatar-preview--flip`）—— 那是 hack：既不成人形，也无法稳定区分，
 *   还把品牌标当成了头像。字母方案让「谁在说话」一眼可辨，同时把品牌标
 *   从头像位释放回它该在的地方（TitleBar / Splash / 关于页）。
 *
 * 尺寸语义与被替换物不同（有意为之）:
 *   NuphusLogo 是 20/22px 的小图标居中悬浮在 34/36px 容器里；
 *   字母头像 `size` 传**容器尺寸**（34/36）铺满 —— 与用户上传自定义图片时
 *   `.msg-avatar-img { width:100%; height:100% }` 的铺满行为一致，
 *   默认态与自定义态才统一（不会一个悬浮一个铺满）。
 *
 * 颜色（大王 2026-09-25 指示：头像背景用主题反色）:
 *   背景 = `--fg-1`（主题**前景**色）、字母 = `--surface-0`（主题**背景**色）
 *   —— 前/背景对调即「反色」：暗主题下头像底亮字暗，亮主题下底暗字亮，
 *   两种主题都保持高对比，且随主题（含自定义主题）自动反转。
 *   早先用 `--accent` 打底，那只是「跟着主题变色」，不是反色。
 * ⚠️ SVG 的 presentation attribute（fill=/font-family=）**不接受 var()**，
 * 因此一律走 `style` 注入 CSS 属性。
 */

interface LetterAvatarProps {
  /** 角色字母：用户侧 'U'，智能体侧 'A' */
  letter: 'U' | 'A'
  /** 铺满尺寸，传容器尺寸（ThemesPage 34 / ChatPanel 36） */
  size?: number
}

export function LetterAvatar({ letter, size = 36 }: LetterAvatarProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      style={{ display: 'block', flexShrink: 0 }}
      aria-hidden="true"
      focusable="false"
    >
      {/* 圆形底板铺满 viewBox：脱离圆形容器也自成头像，字母不会悬空 */}
      <circle cx="12" cy="12" r="12" style={{ fill: 'var(--fg-1)' }} />
      <text
        x="12"
        y="12"
        textAnchor="middle"
        dominantBaseline="central"
        style={{
          fill: 'var(--surface-0)',
          fontFamily: 'var(--font-sans, system-ui, sans-serif)',
          fontSize: 13,
          fontWeight: 600,
        }}
      >
        {letter}
      </text>
    </svg>
  )
}
