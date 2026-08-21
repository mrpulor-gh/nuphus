import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  base: './',
  server: {
    port: 5174,
    strictPort: false,
    host: '0.0.0.0',
    proxy: {
      '/api': 'http://127.0.0.1:9090',
      // 移动端开发期代理 → mobile_server（P1，默认端口 18772）
      '/message': 'http://127.0.0.1:18772',
      '/history': 'http://127.0.0.1:18772',
      '/health': 'http://127.0.0.1:18772',
      '/ws': {
        target: 'ws://127.0.0.1:18772',
        ws: true,
      },
    },
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.{ts,tsx}'],
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // 兼容旧移动 WebView（微信 X5 / 老 Safari）：target 降到 es2017 让 esbuild
    // 转译 ??（nullish coalescing）/ 可选链等 es2020+ 语法——实测 P0：老内核解析
    // 失败 → 整个 JS 不执行 → 手机页面白屏（桌面 WebView2 新内核不受影响）。
    target: 'es2017',
    rollupOptions: {
      input: {
        'index.html': 'index.html',
        'mobile.html': 'mobile.html',
        'capture_overlay.html': 'capture_overlay.html',
        'hud.html': 'hud.html',
      },
    },
  },
})