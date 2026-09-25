import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { SKIN_CHANGE_EVENT, readSkinBg, applySkinBg, releaseSkinBg } from './skinBg'

/**
 * 皮肤背景的**唯一渲染点**：挂在根层（`document.body`）的整屏背景层，
 * 一图无接缝地贯穿标题栏与聊天区。
 *
 * # 为什么是"唯一"，以及为什么挂在根层
 *
 * 早先标题栏与聊天区各有一个 `::before` 画同一张皮肤图（components.css /
 * chat-messages.css），两处各写一份 opacity —— 调不透明度时只有一个动，
 * 且任何新区域想加背景都得再加一条规则。改成"一个整屏背景层 + 内部 DOM 不画背景"后，
 * 「同源同透明度」不再是需要维护的约定，而是结构上无法违反的事实：
 * 内部 DOM 根本没有第二处可以画背景。
 *
 * 层级选择有三档，本组件为什么取中间那档：
 * - `.app-shell` **之外**（body / #root 层）：本文的实现。`inset: 0` 的绝对定位
 *   取值于**整个视口**，与"是否落在 grid/flex 行里"彻底无关 —— 标题栏那 48px 是
 *   视口的一部分，天然被覆盖。这也让内部 DOM 的背景（`.app-shell` 的径向渐变、
 *   各内容区自身的底色）保持原样，不必为了透出背景而逐个改成 transparent
 *   （title-bar 那种改动会连带改变**没配皮肤时**的观感）。
 * - `.app-shell` 内部：可行（见 git 历史里的上一版），但"整屏覆盖"依赖
 *   absolute 逃逸 grid 排布，且会被 `.app-shell` 自身的背景画在底下。
 * - `html` 元素的背景图：**不可行**。`background-attachment: fixed` 在 WebView2
 *   下会触发整屏重绘（真有性能回退）；且 html 背景的绘制/裁剪与 `overflow: hidden`
 *   交互微妙，"看不见但查不出原因"的成本很高。
 *
 * # 为什么不给 App 根再加 wrapper
 *
 * `.app-island` 经 `el.closest('.app-shell')` 定位宿主（test/app-island-anchor.test.tsx），
 * 多一层容器就会截断这条链。本组件经 portal 挂到 body，在 React 树里虽位于
 * `.app-shell` 内，但**不产生任何 DOM 容器**，closest 链完全不受影响。
 *
 * # 渲染契约
 * - 外层：`position: fixed; inset: 0`，**不设 opacity** —— 外层设了会与绘制层
 *   叠成双重衰减，同源透明度立刻失真。
 * - 绘制层：`inset: 0` + `opacity: var(--skin-bg-opacity, 0.35)`，图片与透明度
 *   都只在这一处声明。
 * - `pointer-events: none` + `aria-hidden`：背景不得截获拖拽区
 *   （`data-tauri-drag-region`）与任何点击，也不该进无障碍树。
 * - 图片 URL 为空时不渲染绘制层（避免产出 `url("")` 这种空值，也免得测试与
 *   浏览器对"空背景"给出不同解释）。
 * - 因此本组件位置无关：Tauri 窗口是独立顶层文档，无 iframe 祖先，
 *   `position: fixed` 无 offsetParent 偏差。
 *
 * # 数据来源
 * 由 `ui/skinBg.ts` 广播 `SKIN_CHANGE_EVENT`；本组件订阅，并在挂载时**同步读一次**
 * localStorage 作为首帧初值 —— 异步解析完之前先显示上次的 URL，避免"先空一帧再出现"
 * 的闪烁（浅色主题下这一帧尤其明显）。
 */
export function SkinBackdrop() {
  // 首值必须同步取（见上方"数据来源"），不能只等事件。
  const [url, setUrl] = useState<string | null>(() => readSkinBg() || null)
  // 当前生效的 URL 镜像：释放旧 blob 需要它，而 setState 的更新函数必须是纯函数
  // （StrictMode 下会被调用两次，副作用写进去等于重复释放同一 URL）。
  const activeUrlRef = useRef<string | null>(url)

  useEffect(() => {
    const onSkinChange = (e: Event) => {
      const next = (e as CustomEvent<{ url: string | null }>).detail?.url ?? null
      const prev = activeUrlRef.current
      if (prev === next) return
      activeUrlRef.current = next
      setUrl(next)
      // 旧值是 blob URL → 立刻释放，避免整张图的字节活到页面关闭
      releaseSkinBg(prev)
    }
    window.addEventListener(SKIN_CHANGE_EVENT, onSkinChange)
    // App 层与 ThemesPage 都会调 applySkinBg（常见为同一路径被应用两次）：
    // 重复解析无副作用，这里补一次可确定地覆盖"本组件挂载晚于 App 层 effect"的时序。
    void applySkinBg(readSkinBg())

    return () => {
      window.removeEventListener(SKIN_CHANGE_EVENT, onSkinChange)
      // 卸载：blob URL 的字节随组件释放，不留到页面关闭
      releaseSkinBg(activeUrlRef.current)
      activeUrlRef.current = null
    }
  }, [])

  return createPortal(
    <div className="app-skin-backdrop" data-skin-backdrop aria-hidden="true">
      {url && (
        <div className="app-skin-backdrop-img" style={{ backgroundImage: `url("${url}")` }} />
      )}
    </div>,
    document.body,
  )
}
