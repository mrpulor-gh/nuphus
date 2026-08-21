/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  theme: {
    extend: {
      colors: {
        bg: { primary: '#0d0d0d', secondary: '#141414', tertiary: '#1a1a1a', hover: '#222222', active: '#2a2a2a', input: '#111111' },
        border: { DEFAULT: '#2a2a2a', secondary: '#1f1f1f', focus: '#555555' },
        foreground: '#e8e8e8',
        muted: { DEFAULT: '#444444', foreground: '#a0a0a0' },
        accent: { DEFAULT: '#e5a040', dim: 'rgba(229, 160, 64, 0.15)', glow: 'rgba(229, 160, 64, 0.3)' },
        success: { DEFAULT: '#4caf50', dim: 'rgba(76, 175, 80, 0.15)' },
        error: { DEFAULT: '#ef5350', dim: 'rgba(239, 83, 80, 0.15)' },
        warning: { DEFAULT: '#ff9800', dim: 'rgba(255, 152, 0, 0.15)' },
        info: { DEFAULT: '#42a5f5', dim: 'rgba(66, 165, 245, 0.15)' },
      },
      fontFamily: { ui: ['Inter', 'system-ui', 'sans-serif'], mono: ['JetBrains Mono', 'Fira Code', 'Consolas', 'monospace'] },
      fontSize: { xxs: '10px' },
      spacing: { xs: '4px', sm: '8px', md: '12px', lg: '16px', xl: '24px', '2xl': '32px' },
      borderRadius: { sm: '4px', md: '6px', lg: '8px' },
      animation: { pulse: 'pulse 2s infinite', 'spin-slow': 'spin 3s linear infinite' },
      keyframes: { pulse: { '0%, 100%': { opacity: '1' }, '50%': { opacity: '0.5' } } },
    },
  },
  plugins: [],
}
