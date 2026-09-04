import React from 'react'

export type MoodState =
  | 'idle'
  | 'thinking'
  | 'working'
  | 'success'
  | 'error'
  | 'waiting'
  | 'reading'
  | 'writing'
  | 'searching'
  | 'coding'
  | 'analyzing'

interface MoodFaceProps {
  mood: MoodState
  size?: number
}

export const MoodFace: React.FC<MoodFaceProps> = ({ mood, size = 16 }) => {
  return (
    <span
      className={`mood-face ${mood}`}
      title={mood}
      style={{
        width: size,
        height: size,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <svg
        viewBox="0 0 24 24"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        style={{ width: size, height: size }}
      >
        {mood === 'idle' && (
          <g className="nuphus-idle">
            <g className="idle-bubble">
              {/* 圆角对话气泡 + 左下小尾巴：等你说话 */}
              <path
                d="M 4.5 6.5 A 2.5 2.5 0 0 1 7 4 H 17 A 2.5 2.5 0 0 1 19.5 6.5 V 14 A 2.5 2.5 0 0 1 17 16.5 H 9 L 5.5 19.5 V 16.5 A 2.5 2.5 0 0 1 4.5 14 Z"
                stroke="currentColor"
                strokeWidth="1.3"
                strokeLinejoin="round"
                strokeLinecap="round"
                fill="none"
              />
              {/* typing 三点，顺序脉动 */}
              <circle
                className="idle-dot idle-dot-1"
                cx="9"
                cy="10.5"
                r="0.9"
                fill="currentColor"
              />
              <circle
                className="idle-dot idle-dot-2"
                cx="12"
                cy="10.5"
                r="0.9"
                fill="currentColor"
              />
              <circle
                className="idle-dot idle-dot-3"
                cx="15"
                cy="10.5"
                r="0.9"
                fill="currentColor"
              />
            </g>
          </g>
        )}

        {mood === 'thinking' && (
          <g className="mood-thinking">
            {/* 外层涟漪光晕 — 扩散动画 */}
            <circle
              cx="12"
              cy="10"
              r="14"
              fill="none"
              stroke="currentColor"
              strokeWidth="0.3"
              opacity="0.15"
              className="halo-outer"
            />
            <circle
              cx="12"
              cy="10"
              r="11"
              fill="none"
              stroke="currentColor"
              strokeWidth="0.5"
              opacity="0.25"
              className="halo-mid"
            />

            {/* 灯泡玻璃主体 — 精密轮廓 */}
            <path
              d="M 12 1.5 C 7.5 1.5 4.5 5 4.5 9 C 4.5 12 6 14 7.5 15.5 L 8 16 L 16 16 L 16.5 15.5 C 18 14 19.5 12 19.5 9 C 19.5 5 16.5 1.5 12 1.5 Z"
              fill="currentColor"
              opacity="0.12"
              className="bulb-glass"
            />
            {/* 玻璃厚度边框 */}
            <path
              d="M 12 2 C 8 2 5 5.2 5 9 C 5 11.8 6.3 13.8 7.8 15.2 L 8.2 15.8 L 15.8 15.8 L 16.2 15.2 C 17.7 13.8 19 11.8 19 9 C 19 5.2 16 2 12 2 Z"
              fill="none"
              stroke="currentColor"
              strokeWidth="0.6"
              opacity="0.5"
              className="bulb-rim"
            />

            {/* 几何折线灯丝 — W形精密结构 */}
            <path
              d="M 9 13 L 9.8 10 L 10.6 11.5 L 11.4 8.5 L 12.2 10.5 L 13 7.5 L 13.8 9.5 L 14.6 8 L 15 13"
              fill="none"
              stroke="currentColor"
              strokeWidth="0.9"
              strokeLinecap="round"
              strokeLinejoin="round"
              opacity="0.9"
              className="filament"
            />
            {/* 灯丝支架 */}
            <line
              x1="9"
              y1="13"
              x2="9"
              y2="14.5"
              stroke="currentColor"
              strokeWidth="0.5"
              opacity="0.5"
              className="filament-post"
            />
            <line
              x1="15"
              y1="13"
              x2="15"
              y2="14.5"
              stroke="currentColor"
              strokeWidth="0.5"
              opacity="0.5"
              className="filament-post"
            />

            {/* 金属螺纹底座 */}
            <rect
              x="9"
              y="16"
              width="6"
              height="1.2"
              rx="0.3"
              fill="currentColor"
              opacity="0.35"
              className="base-top"
            />
            <rect
              x="9.2"
              y="17.2"
              width="5.6"
              height="1"
              rx="0.2"
              fill="currentColor"
              opacity="0.28"
              className="base-mid"
            />
            <rect
              x="9.5"
              y="18.2"
              width="5"
              height="0.9"
              rx="0.2"
              fill="currentColor"
              opacity="0.22"
              className="base-bottom"
            />
            {/* 螺纹细线 */}
            <line
              x1="9.3"
              y1="16.6"
              x2="14.7"
              y2="16.6"
              stroke="currentColor"
              strokeWidth="0.3"
              opacity="0.4"
              className="base-thread"
            />
            <line
              x1="9.4"
              y1="17.7"
              x2="14.6"
              y2="17.7"
              stroke="currentColor"
              strokeWidth="0.3"
              opacity="0.3"
              className="base-thread"
            />

            {/* 玻璃表面高光 — 左上 */}
            <ellipse
              cx="9.5"
              cy="5.5"
              rx="2.2"
              ry="1.4"
              fill="white"
              opacity="0.2"
              className="highlight-main"
            />
            {/* 玻璃底部折射 — 右下 */}
            <ellipse
              cx="14"
              cy="12"
              rx="1.5"
              ry="1"
              fill="white"
              opacity="0.08"
              className="highlight-sub"
            />

            {/* 内层稳定光晕 */}
            <circle
              cx="12"
              cy="9.5"
              r="5"
              fill="currentColor"
              opacity="0.08"
              className="halo-inner"
            />
          </g>
        )}

        {mood === 'working' && (
          <g className="mood-working">
            <path
              d="M 12 3 L 9 10 L 13 10 L 11 19 L 17 9 L 13 9 L 15 3 Z"
              fill="currentColor"
              className="bolt"
            />
          </g>
        )}

        {mood === 'success' && (
          <g className="mood-success">
            <path
              d="M 6 12 L 10 16 L 19 7"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              fill="none"
              className="check"
            />
            <line
              x1="12"
              y1="3"
              x2="12"
              y2="5"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
              className="ray ray-1"
            />
            <line
              x1="12"
              y1="19"
              x2="12"
              y2="21"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
              className="ray ray-2"
            />
            <line
              x1="3"
              y1="12"
              x2="5"
              y2="12"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
              className="ray ray-3"
            />
            <line
              x1="19"
              y1="12"
              x2="21"
              y2="12"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
              className="ray ray-4"
            />
          </g>
        )}

        {mood === 'error' && (
          <g className="mood-error">
            <line
              x1="7"
              y1="7"
              x2="17"
              y2="17"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              className="cross-1"
            />
            <line
              x1="17"
              y1="7"
              x2="7"
              y2="17"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              className="cross-2"
            />
          </g>
        )}

        {mood === 'waiting' && (
          <g className="mood-waiting">
            <circle cx="8" cy="9" r="1.5" fill="currentColor" className="eye-left" />
            <circle cx="16" cy="9" r="1.5" fill="currentColor" className="eye-right" />
            <path
              d="M 6 14 Q 12 14 18 14"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
              fill="none"
              className="mouth"
            />
            <path
              d="M 20 7 A 3 3 0 1 1 20 13"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
              fill="none"
              className="watch"
            />
          </g>
        )}

        {mood === 'reading' && (
          <g className="mood-reading">
            {/* 书脊阴影 */}
            <rect
              x="11.2"
              y="4"
              width="1.6"
              height="14"
              rx="0.3"
              fill="currentColor"
              opacity="0.15"
              className="shadow-spine"
            />
            {/* 左页 */}
            <path
              d="M 11 4 Q 7 3.5 4 5 L 4 17 Q 7 15.5 11 16 Z"
              fill="currentColor"
              opacity="0.08"
              className="page-left"
            />
            <path
              d="M 11 4 Q 7 3.5 4 5 L 4 17 Q 7 15.5 11 16 Z"
              stroke="currentColor"
              strokeWidth="1"
              strokeLinecap="round"
              strokeLinejoin="round"
              fill="none"
              className="book-cover"
            />
            {/* 右页 */}
            <path
              d="M 13 4 Q 17 3.5 20 5 L 20 17 Q 17 15.5 13 16 Z"
              fill="currentColor"
              opacity="0.08"
              className="page-right"
            />
            <path
              d="M 13 4 Q 17 3.5 20 5 L 20 17 Q 17 15.5 13 16 Z"
              stroke="currentColor"
              strokeWidth="1"
              strokeLinecap="round"
              strokeLinejoin="round"
              fill="none"
              className="book-cover"
            />
            {/* 左页文字行 */}
            <path
              d="M 6 8 Q 8 7.5 10 8.2"
              stroke="currentColor"
              strokeWidth="0.7"
              strokeLinecap="round"
              fill="none"
              opacity="0.5"
              className="text-line"
            />
            <path
              d="M 6 10.5 Q 8 10 10 10.7"
              stroke="currentColor"
              strokeWidth="0.7"
              strokeLinecap="round"
              fill="none"
              opacity="0.4"
              className="text-line"
            />
            <path
              d="M 6 13 Q 8 12.5 10 13.2"
              stroke="currentColor"
              strokeWidth="0.7"
              strokeLinecap="round"
              fill="none"
              opacity="0.3"
              className="text-line"
            />
            {/* 右页文字行 */}
            <path
              d="M 14 8.2 Q 16 7.5 18 8"
              stroke="currentColor"
              strokeWidth="0.7"
              strokeLinecap="round"
              fill="none"
              opacity="0.3"
              className="text-line"
            />
            <path
              d="M 14 10.7 Q 16 10 18 10.5"
              stroke="currentColor"
              strokeWidth="0.7"
              strokeLinecap="round"
              fill="none"
              opacity="0.4"
              className="text-line"
            />
            <path
              d="M 14 13.2 Q 16 12.5 18 13"
              stroke="currentColor"
              strokeWidth="0.7"
              strokeLinecap="round"
              fill="none"
              opacity="0.5"
              className="text-line"
            />
            {/* 书签丝带 */}
            <path
              d="M 12 4 L 12 6.5 L 11.2 5.8 L 10.4 6.5 L 10.4 4 Z"
              fill="currentColor"
              opacity="0.3"
              className="ribbon"
            />
          </g>
        )}

        {mood === 'writing' && (
          <g className="mood-writing">
            {/* 笔尖 */}
            <path
              d="M 17 4 L 19 6 L 8 17 L 6 15 Z"
              fill="currentColor"
              opacity="0.15"
              className="pen-body"
            />
            <path
              d="M 17 4 L 19 6 L 8 17 L 6 15 Z"
              stroke="currentColor"
              strokeWidth="0.8"
              strokeLinecap="round"
              strokeLinejoin="round"
              fill="none"
              className="pen-stroke"
            />
            {/* 书写流线 */}
            <path
              d="M 5 18 Q 8 16 12 17 Q 16 18 19 16"
              stroke="currentColor"
              strokeWidth="0.8"
              strokeLinecap="round"
              fill="none"
              className="write-line"
            />
            {/* 笔尖光晕 */}
            <circle cx="7" cy="16" r="1.2" fill="currentColor" className="tip-glow" />
          </g>
        )}

        {mood === 'searching' && (
          <g className="mood-searching">
            <circle
              cx="10"
              cy="10"
              r="5"
              stroke="currentColor"
              strokeWidth="1.5"
              fill="none"
              className="lens"
            />
            <line
              x1="14"
              y1="14"
              x2="19"
              y2="19"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              className="handle"
            />
            <circle
              cx="10"
              cy="10"
              r="7"
              stroke="currentColor"
              strokeWidth="0.5"
              fill="none"
              opacity="0.3"
              className="pulse"
            />
          </g>
        )}

        {mood === 'coding' && (
          <g className="mood-coding">
            {/* 编辑器背景 */}
            <rect
              x="2"
              y="3"
              width="20"
              height="18"
              rx="1.5"
              fill="currentColor"
              opacity="0.05"
              className="editor-bg"
            />
            {/* 行号标记 */}
            <circle
              cx="4.5"
              cy="8"
              r="0.6"
              fill="currentColor"
              opacity="0.2"
              className="line-dot"
            />
            <circle
              cx="4.5"
              cy="12.5"
              r="0.6"
              fill="currentColor"
              opacity="0.2"
              className="line-dot"
            />
            <circle
              cx="4.5"
              cy="17"
              r="0.6"
              fill="currentColor"
              opacity="0.2"
              className="line-dot"
            />
            {/* 行号栏分隔线 */}
            <line
              x1="5.8"
              y1="4.5"
              x2="5.8"
              y2="19.5"
              stroke="currentColor"
              strokeWidth="0.3"
              opacity="0.1"
              className="gutter-line"
            />
            {/* 代码行1：关键字 + 内容 */}
            <rect
              x="7"
              y="6.5"
              width="4.5"
              height="2.5"
              rx="0.8"
              fill="currentColor"
              opacity="0.35"
              className="keyword"
            />
            <path
              d="M 12.5 7.5 L 18 7.5"
              stroke="currentColor"
              strokeWidth="0.8"
              strokeLinecap="round"
              fill="none"
              opacity="0.2"
              className="code-text"
            />
            {/* 代码行2：缩进 + 关键字 */}
            <rect
              x="9"
              y="11"
              width="3"
              height="2.5"
              rx="0.6"
              fill="currentColor"
              opacity="0.25"
              className="keyword"
            />
            <path
              d="M 13 12 L 18 12"
              stroke="currentColor"
              strokeWidth="0.8"
              strokeLinecap="round"
              fill="none"
              opacity="0.15"
              className="code-text"
            />
            {/* 闪烁光标 */}
            <line
              x1="10.5"
              y1="15.5"
              x2="10.5"
              y2="18.5"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
              className="cursor-blink"
            />
            {/* 代码行3：大括号 */}
            <path
              d="M 7 16 L 9 16"
              stroke="currentColor"
              strokeWidth="0.8"
              strokeLinecap="round"
              fill="none"
              opacity="0.2"
              className="code-text"
            />
          </g>
        )}

        {mood === 'analyzing' && (
          <g className="mood-analyzing">
            {/* 网格背景 */}
            <path
              d="M 3 8 L 21 8 M 3 12 L 21 12 M 3 16 L 21 16"
              stroke="currentColor"
              strokeWidth="0.2"
              opacity="0.08"
              className="grid"
            />
            {/* 基线 */}
            <line
              x1="3"
              y1="18"
              x2="21"
              y2="18"
              stroke="currentColor"
              strokeWidth="0.6"
              strokeLinecap="round"
              opacity="0.3"
              className="baseline"
            />
            {/* 柱子1（最低） */}
            <rect
              x="5"
              y="13"
              width="3"
              height="5"
              rx="0.5"
              fill="currentColor"
              opacity="0.35"
              className="bar-1"
            />
            {/* 柱子2（中等） */}
            <rect
              x="10.5"
              y="10"
              width="3"
              height="8"
              rx="0.5"
              fill="currentColor"
              opacity="0.55"
              className="bar-2"
            />
            {/* 柱子3（最高） */}
            <rect
              x="16"
              y="6"
              width="3"
              height="12"
              rx="0.5"
              fill="currentColor"
              className="bar-3"
            />
            {/* 趋势线 */}
            <polyline
              points="6.5 13 12 10 17.5 6"
              stroke="currentColor"
              strokeWidth="0.8"
              strokeLinecap="round"
              strokeLinejoin="round"
              fill="none"
              opacity="0.6"
              className="trend-line"
            />
            {/* 数据点 */}
            <circle cx="6.5" cy="13" r="1" fill="currentColor" className="dot-1" />
            <circle cx="12" cy="10" r="1.2" fill="currentColor" className="dot-2" />
            <circle cx="17.5" cy="6" r="1.5" fill="currentColor" className="dot-3" />
            {/* 最后一个数据点的光晕 */}
            <circle
              cx="17.5"
              cy="6"
              r="3"
              fill="currentColor"
              opacity="0.15"
              className="dot-glow"
            />
          </g>
        )}
      </svg>
    </span>
  )
}
