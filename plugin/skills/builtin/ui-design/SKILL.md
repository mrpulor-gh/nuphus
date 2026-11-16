---
title: 默认设计语言
id: ui-design
type: skill
tags: [design, ui, visual-style, default-language, tokens, components, motion, copy, css]
builtin: true
---

# 默认设计语言

Nuphus 带出的界面，默认用这套语言。
**用户没有指定风格、项目里也没有既有设计系统时，直接按本文产出，不要反问「你想要什么风格」。**

取向：安静的壳，内容上前——单色 chrome、wash 分层、大圆角、一个主行动。

视觉基准（可渲染核对）：
file:///C:/Users/Administrator/Nuphus/plugin/skills/builtin/ui-design/reference/design-reference.html

可运行示例（照本文生成，可直接打开）：
file:///C:/Users/Administrator/Nuphus/plugin/skills/builtin/ui-design/example/index.html

三条判据（任何页面成立）：

1. chrome（底 / 面板 / 文字 / 线）只走墨色阶，彩色只属于内容（图片、图表、预览素材）。
2. 层级用 wash 或明度差表达；边框、阴影、渐变三者只选一种，不叠加。
3. 一屏一个主行动；其余动作降级为次按钮、图标或 hover 出现。

---

## 1 落地

### 1.1 先看有没有既有设计系统

- 项目里已有 tokens / theme / 设计系统 / UI 库主题 → **遵守它**，按开头三条判据对齐，不另起一套。
- 没有 → 按下面三步落地，这是默认路径。

### 1.2 落 token

把 §2 整段复制成 `styles/tokens.css`（路径按项目技术栈放，React / Vue / 静态页都适用），在入口引入一次。

### 1.3 落基元

把 §3 复制成 `styles/base.css`。页面只用 token 和基元类，不写裸 hex。

三步做完再动页面，不要边写页面边补 token。

---

## 2 tokens.css（可直接复制）

```css
:root {
  color-scheme: light;

  /* 表面：页面底 → 面板 → 悬浮 → 更深 */
  --bg: #ffffff;
  --surface: #f7f7f6;
  --surface-2: #f1f1f0;
  --surface-3: #ecebe7;

  /* 洗底：容器 / 控件底，代替边框 */
  --wash: rgba(23, 24, 26, .06);
  --wash-strong: rgba(23, 24, 26, .10);
  --raise: #ffffff; /* 抬升块：叠在 wash 上的选中 / 激活块，比轨道亮一档 */

  /* 墨色文字阶 */
  --ink-1: #17181a;
  --ink-2: #3c3b37;
  --ink-3: #5c5b56;
  --ink-4: #6f6e68;
  --ink-5: #85847e;

  /* 深色块与描边 */
  --solid: #0e0f11;
  --on-solid: #ffffff;
  --rule: rgba(23, 24, 26, .16);
  --rule-soft: rgba(23, 24, 26, .07);

  /* 状态：只给图标 / 徽标 / 文字，不给状态点换色 */
  --ok: #15803d;
  --signal: #2231dd;
  --ember: #b8431c;

  --font-ui: system-ui, -apple-system, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif;
  --font-mono: ui-monospace, "SF Mono", Menlo, Consolas, monospace;

  /* 字阶：11 / 12 / 13 / 16 / 19 / 30 / 40 */
  --fz-xs: 11px;  --fz-sm: 12px;  --fz-md: 13px;
  --fz-lg: 16px;  --fz-xl: 19px;
  --fz-2xl: 30px; --fz-3xl: 40px;

  /* 圆角：控件 12 / 卡片 20 / 容器 28 / 胶囊 999 */
  --r-sm: 8px; --r-md: 12px; --r-lg: 20px; --r-xl: 28px; --r-pill: 999px;

  /* 间距：4 的倍数。12 同层 / 16–24 容器 padding / 32 语义切换 / 48–64 大分区 */
  --sp-1: 4px;  --sp-2: 8px;   --sp-3: 12px; --sp-4: 16px;
  --sp-5: 20px; --sp-6: 24px;  --sp-7: 32px; --sp-8: 48px; --sp-9: 64px;

  /* 动效：四层时长，两条曲线 */
  --dur-fast: .16s;   /* 确认反馈 */
  --dur-med: .22s;    /* 状态转场 */
  --dur-slow: .42s;   /* 容器形变 */
  --ease-swift: cubic-bezier(.22, 1, .36, 1);
  --ease-soft: cubic-bezier(.33, .55, .2, 1);
}

[data-theme="dark"] {
  color-scheme: dark;
  --bg: #000000;
  --surface: #0f0f0f;
  --surface-2: #181818;
  --surface-3: #202020;
  --wash: rgba(255, 255, 255, .09);
  --wash-strong: rgba(255, 255, 255, .14);
  --raise: #2a2a2a;
  --ink-1: #ffffff;
  --ink-2: #b4b4b4;
  --ink-3: #9a9a9a;
  --ink-4: #828282;
  --ink-5: #6b6b6b;
  --solid: #000000;
  --on-solid: #ffffff;
  --rule: rgba(255, 255, 255, .14);
  --rule-soft: rgba(255, 255, 255, .07);
  --ok: #34d17a;
}
```

主题切换：给 `<html>` 加/去 `data-theme="dark"`。若页面只有一个主题，删掉不需要的那半。

---

## 3 基元（可直接复制）

```css
* { box-sizing: border-box; }

body {
  margin: 0;
  background: var(--bg);
  color: var(--ink-1);
  font: 400 var(--fz-md)/1.5 var(--font-ui);
  -webkit-font-smoothing: antialiased;
}

/* 按钮：一屏只允许一个实心 */
.btn {
  height: 36px; padding: 0 16px;
  border: 0; border-radius: var(--r-pill);
  font: 500 var(--fz-md)/1 var(--font-ui);
  cursor: pointer;
  transition: opacity var(--dur-fast), scale var(--dur-fast) var(--ease-swift);
}
.btn:active { scale: .96; }
.btn-primary { background: var(--ink-1); color: var(--bg); }
.btn-primary:hover { opacity: .88; }
.btn-secondary {
  background: transparent; color: var(--ink-2);
  box-shadow: inset 0 0 0 1px var(--rule);
}
.btn-secondary:hover { color: var(--ink-1); }

/* 卡片：只有 wash 和圆角 */
.card {
  padding: var(--sp-5);
  border-radius: var(--r-lg);
  background: var(--wash);
}
.card-title { font: 500 var(--fz-lg)/1.2 var(--font-ui); margin: 0 0 6px; }
.card-desc  { font-size: var(--fz-md); color: var(--ink-4); margin: 0; }

/* 分段控件 */
.seg {
  display: inline-flex; gap: 2px; padding: 2px;
  border-radius: var(--r-pill); background: var(--wash);
}
.seg > button {
  height: 30px; padding: 0 14px; border: 0; border-radius: var(--r-pill);
  background: transparent; color: var(--ink-3);
  font: 500 var(--fz-sm)/1 var(--font-ui); cursor: pointer;
  transition: background var(--dur-fast), color var(--dur-fast);
}
.seg > button[aria-pressed="true"] { background: var(--raise); color: var(--ink-1); }

/* 输入 */
.input {
  width: 100%; height: 40px; padding: 0 14px;
  border: 0; border-radius: var(--r-md);
  background: var(--wash); color: var(--ink-1);
  font: 400 var(--fz-md)/1 var(--font-ui);
}
.input::placeholder { color: var(--ink-5); }
.input:focus-visible { outline: 2px solid var(--signal); outline-offset: 1px; }

/* 徽章与状态点：点只有两色 */
.badge {
  display: inline-flex; align-items: center; gap: 6px;
  height: 22px; padding: 0 9px;
  border-radius: var(--r-pill); background: var(--wash);
  font: 500 var(--fz-xs)/1 var(--font-ui); color: var(--ink-3);
}
.dot { width: 7px; height: 7px; border-radius: 50%; background: var(--ink-4); }
.dot[data-live] { background: var(--signal); }

/* 浮层 */
.scrim { position: fixed; inset: 0; background: color-mix(in srgb, var(--bg) 78%, transparent); backdrop-filter: blur(12px) saturate(130%); }
.panel {
  border-radius: var(--r-xl); background: var(--bg);
  box-shadow: 0 12px 40px rgba(0, 0, 0, .10);
}

/* 节奏：同层 12 / 语义切换 32；容器 padding 用 --sp-5(20) */
.stack { display: flex; flex-direction: column; gap: var(--sp-3); }
.stack.stack-group { gap: var(--sp-7); }

/* 常驻 + 滚动：根两行网格，滚动区自己滚，常驻区不动 */
.app-shell { display: grid; grid-template-rows: minmax(0, 1fr) auto; height: 100%; }
.app-scroll { min-height: 0; overflow-y: auto; }
```

---

## 4 布局与节奏

### 4.1 骨架

- 内容最大宽 1180px 居中；桌面页面边距 20px，移动 14px（页面边距独立于块间距阶梯，不参与下面五档）。
- 列数：1440 → 3，1024 / 768 → 2，<760 → 1。
- 列间距 = 同层卡片纵向间距 = 12px。
- 主界面优先 tile / 大卡 / 图标行 / 分段；超过 6 行且需横向比较才用表格。
- 触控目标 ≥44px；移动端输入框字号 ≥16px（防 iOS 缩放）。

### 4.2 间距阶梯（只有这五档）

| 档 | 值 | 判据：两个元素之间是什么关系 |
|---|---|---|
| 信息内部 | 4 / 8 | 同一条信息内部：标签↔值、图标↔文字、标题↔副标题 |
| 同层 | 12 | 同一容器里的兄弟元素、并列卡片、网格列 |
| 容器内边距 | 16–24（默认 20） | 卡片 / 面板 / 分组容器的 padding |
| 语义切换 | 32 | 换一组内容，读者需要「停一下」才看出这是两段 |
| 大分区 | 48 / 64 | 页面主区之间、页面底部之前 |

- 只用这五档。18 / 28 / 36 这类即兴值是错的，不是「差一点」。
- **分组的判定**：共享一个标题、同一主语、能用一句话概括的一块内容 = 一组。组内一切可见间距走 12，组与组之间走 32–48。
- **内外不倒挂**：必须满足 `12 < 容器 padding < 32`。padding ≤ 12 时块散架，读者看不出这是一组；padding ≥ 语义切换间距时内外界限消失，两段粘成一段。
- 空间不够时按序让位：说明文字 → 次要块 → 装饰。间距不让位，不靠压间距塞内容。

### 4.3 常驻区与滚动容器

- 页面根用两行网格：`grid-template-rows: minmax(0, 1fr) auto`。第一行是滚动内容，第二行是常驻区（输入框、操作条、底栏）。
- 常驻区不放进滚动容器，也不用 `absolute` / `fixed` 挂在滚动内容上——它会跟着内容滚走，滚到底部时被推出视口。
- 滚动容器自身写 `min-height: 0; overflow-y: auto`。grid / flex 子项默认 `min-height: auto`，不写 0 就不收缩，滚动会跑到整页上，常驻区随之消失。
- 常驻区与滚动区之间用一条 `--rule-soft` 或 wash 分隔，不叠阴影浮起。
- 常驻区高度 ≤ 首屏的 1/4；移动端键盘弹起时输入控件仍完整可见。

### 4.4 媒体元素

- 图 / 图表 / 视频 / iframe 一律 `width: 100%` + `aspect-ratio`（或显式高度）。不用 `height: auto` 裸放，不用 `max-height` 猜；加载前后不得改变布局高度。
- 图表容器最小高：移动 160px、桌面 220px。低于此值坐标刻度挤成糊。
- 图片预览用 `object-fit: cover` + 固定比例，不拉伸变形。

### 4.5 首屏预算

- 判据：1440×900 与 390×844 两档，不滚动即可见「产品类型（标题）+ 主对象（第一块内容）+ 唯一主行动」。
- 主行动被折叠线切掉、或需要滚动才能看到，即不达标。
- 首屏放不下时下沉内容（折叠、进二级页），不压缩首屏间距。

---

## 5 动效

| 层 | 时长 | 用途 |
|---|---|---|
| 手指直控 | 0（无 transition） | 拖拽 / 滑动 / 进度，走 CSS 变量实时映射 |
| 确认反馈 | 160ms | 按钮、hover、颜色 |
| 状态转场 | 220ms | 淡入、折叠、图标形变 |
| 容器形变 | 420ms | 宽度 / 高度 / 抽屉 |
| 退场 | ≤ 进场的 85% | 关闭、消失 |

四条硬约束：

1. **手指直接驱动的量不许有 transition**，否则手感发飘。
2. **弹性做成参数喂给贝塞尔**：`--ov: 1 + bounce * 0.7` → `cubic-bezier(.3, var(--ov), .4, 1)`，不要为每个幅度复制 keyframes。
3. **入场 keyframes 只写 `from`**，终点交给元素默认值；省一半代码且不会结束时跳一下。
4. **进度用一个 CSS 变量 + `calc()` 驱动多元素**（`opacity: calc(...)`），不要逐帧改样式或触发重渲染。

界面动效不超过 600ms；不做弹跳、旋转、装饰性入场；流式日志与实时列表不加逐条动画；必须支持 `prefers-reduced-motion`。

---

## 6 文案

- 写标签和动作，不写段落。标题 + 一行说明 + 状态 + 动作。
- 按钮写动词，一个按钮只做一件事。
- 状态 = 文字或图标，不只靠颜色。
- 只陈述事实：「已保存」而不是「太棒了，已保存！」。
- 数字、单位、日期格式统一。
- 不拟人、不煽情、不用感叹号、不堆形容词。

---

## 7 禁止

1. 品牌色 / 渐变出现在界面 chrome 上。
2. 一屏两个实心主按钮。
3. 主界面堆段落、设计说明、方案解释。
4. 卡片套卡片；用容器包没有分组意义的元素。
5. 同时使用边框 + 阴影 + 渐变分层。
6. 静态卡片加投影。
7. 只用颜色表达状态。
8. 页面 CSS 写裸 hex、新字号、新圆角、新缓动（token 之外的即兴值）。
9. 装饰性入场动画、流式内容逐条动画、hover 位移或弹跳。
10. 表达失度：emoji、感叹号、形容词堆叠、拟人化营销腔、「高级感 / 极致体验」类空话。
11. 状态圆点用多色区分；同一屏出现两种以上颜色的状态点。状态靠形状 / 文字 / 图标表达。
12. 图标用 emoji、Unicode 符号（对勾 / 星 / 箭头类）、CSS 伪元素或位图；一律标准 SVG。
13. 常驻区（输入框 / 操作条 / 底栏）放进滚动容器，或用 `absolute` / `fixed` 挂在滚动内容上。
14. 间距出现 18 / 28 / 36 类即兴值，或用压间距的办法把内容塞进一屏。

---

## 8 验收

- [ ] 首屏只有一个主行动。
- [ ] 主界面说明 ≤2 行，长内容进抽屉 / tooltip / 二级页。
- [ ] 大容器 28、卡片 20、控件 12，内层递减，没有统一小圆角。
- [ ] 间距只用 4 / 8 / 12 / 16–24 / 32 / 48–64 这几档，没有 18 / 28 / 36 类即兴值。
- [ ] 每个容器满足 `12 < padding < 32`，没有「内边距小于块间距」或「内边距吃进分组间距」。
- [ ] 常驻输入区 / 底栏在滚动容器外，滚到底部仍完整可见，没有被内容推走。
- [ ] 图 / 图表 / 视频有 aspect-ratio 或显式高度，加载前后无高度跳动；图表高度 ≥160 / 220。
- [ ] 1440×900 与 390×844 不滚动即可见标题、主对象、主行动。
- [ ] chrome 全部走 token，没有未登记颜色。
- [ ] 层级靠 wash / 明度差，没有边框 + 阴影 + 渐变三件套。
- [ ] 状态有文字或图标；同一屏状态点不超过一种颜色。
- [ ] 没有 emoji，图标全部是标准 SVG。
- [ ] 文案只陈述事实，没有感叹号与营销形容词。
- [ ] 390 / 768 / 1440 三档都看过，light / dark 都在，空 / 载 / 错都在。
- [ ] 键盘可达、焦点环可见、正文对比度 ≥4.5:1。
