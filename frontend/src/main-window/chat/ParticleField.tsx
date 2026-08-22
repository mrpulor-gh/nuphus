import { useEffect, useRef } from 'react'

interface Particle {
  x: number
  y: number
  size: number
  vx: number
  vy: number
}

/**
 * ParticleField — 欢迎页粒子场
 * 点状物吸附 + 蛛网连线，颜色跟随主题前景色 --fg-1（亮/暗自动切换）。
 * 覆盖层 pointer-events:none，不拦截鼠标。
 */
export function ParticleField() {
  const canvasRef = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    let W = 0
    let H = 0
    let canvasRect = { left: 0, top: 0 }
    let particles: Particle[] = []
    let rafId = 0
    let fgRgb = '245,245,250'
    // 主题感知 alpha：浅色主题（深色前景）需更强反差，暗色主题（浅色前景）保持克制
    let particleAlpha = 0.22
    let linkAlpha = 0.13
    const mouse = { x: null as number | null, y: null as number | null, radius: 150 }

    // 从 CSS 变量 --fg-1 读取主题前景色，转 "r,g,b" 供 rgba 使用
    const refreshFg = () => {
      const fg = getComputedStyle(document.documentElement).getPropertyValue('--fg-1').trim()
      const m = /^#([0-9a-f]{6})$/i.exec(fg)
      if (m) {
        const h = m[1]
        const r = parseInt(h.slice(0, 2), 16)
        const g = parseInt(h.slice(2, 4), 16)
        const b = parseInt(h.slice(4, 6), 16)
        fgRgb = `${r},${g},${b}`
        // 前景越暗（浅色主题墨色文字）→ 背景越浅 → 需更强 alpha 反差
        const lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255
        particleAlpha = lum < 0.5 ? 0.38 : 0.22
        linkAlpha = lum < 0.5 ? 0.2 : 0.13
      }
    }

    const initParticles = () => {
      particles = []
      const density = Math.floor((W * H) / 6200)
      for (let i = 0; i < density; i++) {
        particles.push({
          x: Math.random() * W,
          y: Math.random() * H,
          size: 2,
          vx: (Math.random() - 0.5) * 0.5,
          vy: (Math.random() - 0.5) * 0.5,
        })
      }
    }

    const resize = () => {
      const rect = canvas.getBoundingClientRect()
      canvasRect = { left: rect.left, top: rect.top }
      W = canvas.width = Math.max(1, Math.floor(rect.width))
      H = canvas.height = Math.max(1, Math.floor(rect.height))
      initParticles()
    }

    const animate = () => {
      ctx.clearRect(0, 0, W, H)
      for (const p of particles) {
        p.x += p.vx
        p.y += p.vy
        if (p.x < 0 || p.x > W) p.vx *= -1
        if (p.y < 0 || p.y > H) p.vy *= -1

        // 鼠标吸附
        if (mouse.x !== null && mouse.y !== null) {
          const dx = mouse.x - p.x
          const dy = mouse.y - p.y
          const dist = Math.sqrt(dx * dx + dy * dy)
          if (dist < mouse.radius) {
            const force = (mouse.radius - dist) / mouse.radius
            p.x += (dx / dist) * force * 2
            p.y += (dy / dist) * force * 2
          }
        }

        // 粒子点
        ctx.fillStyle = `rgba(${fgRgb},${particleAlpha})`
        ctx.beginPath()
        ctx.arc(p.x, p.y, p.size, 0, Math.PI * 2)
        ctx.fill()

        // 蛛网连线（鼠标与近距离粒子）
        if (mouse.x !== null && mouse.y !== null) {
          const dx = mouse.x - p.x
          const dy = mouse.y - p.y
          const dist = Math.sqrt(dx * dx + dy * dy)
          if (dist < mouse.radius) {
            ctx.beginPath()
            ctx.moveTo(p.x, p.y)
            ctx.lineTo(mouse.x, mouse.y)
            ctx.strokeStyle = `rgba(${fgRgb},${linkAlpha * (1 - dist / mouse.radius)})`
            ctx.lineWidth = 1
            ctx.stroke()
          }
        }
      }
      rafId = requestAnimationFrame(animate)
    }

    const onMouseMove = (e: MouseEvent) => {
      mouse.x = e.clientX - canvasRect.left
      mouse.y = e.clientY - canvasRect.top
    }
    const onMouseLeave = () => {
      mouse.x = null
      mouse.y = null
    }

    refreshFg()
    resize()
    rafId = requestAnimationFrame(animate)
    window.addEventListener('resize', resize)
    window.addEventListener('mousemove', onMouseMove)
    window.addEventListener('mouseleave', onMouseLeave)
    const themeObserver = new MutationObserver(refreshFg)
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-theme'],
    })

    return () => {
      cancelAnimationFrame(rafId)
      window.removeEventListener('resize', resize)
      window.removeEventListener('mousemove', onMouseMove)
      window.removeEventListener('mouseleave', onMouseLeave)
      themeObserver.disconnect()
    }
  }, [])

  return <canvas ref={canvasRef} className="particle-field" aria-hidden />
}
