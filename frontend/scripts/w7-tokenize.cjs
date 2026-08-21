// W7 硬编码颜色批量 token 化（安全映射，仅替换语义明确的颜色）
const fs = require('fs');
const path = 'C:/Users/Administrator/Nuphus/frontend/src/styles/';

// 安全映射：硬编码 → token（仅当语义 100% 明确时）
const MAPPINGS = [
  // accent 蓝
  [/#3b82f6\b/gi, 'var(--accent)'],
  [/rgba\(59,\s*130,\s*246,\s*([\d.]+)\)/gi, 'rgba(var(--accent-rgb),$1)'],
  // success 绿
  [/#10b981\b/gi, 'var(--success)'],
  [/#22c55e\b/gi, 'var(--success)'],
  [/rgba\(16,\s*185,\s*129,\s*([\d.]+)\)/gi, 'rgba(var(--success-rgb),$1)'],
  [/rgba\(34,\s*197,\s*94,\s*([\d.]+)\)/gi, 'rgba(var(--success-rgb),$1)'],
  // error 红
  [/#ef4444\b/gi, 'var(--error)'],
  [/rgba\(239,\s*68,\s*68,\s*([\d.]+)\)/gi, 'rgba(var(--error-rgb),$1)'],
  // warning 橙
  [/#f59e0b\b/gi, 'var(--warning)'],
  [/rgba\(245,\s*158,\s*11,\s*([\d.]+)\)/gi, 'rgba(var(--warning-rgb),$1)'],
  // 前景色（void/spark 系）
  [/#f5f5fa\b/gi, 'var(--fg-1)'],
  [/#d0d0dd\b/gi, 'var(--fg-2)'],
  [/#a0a0b0\b/gi, 'var(--fg-3)'],
  [/#707080\b/gi, 'var(--fg-4)'],
  [/#505060\b/gi, 'var(--fg-5)'],
  // 表面
  [/#0a0a10\b/gi, 'var(--surface-0)'],
  [/#12121a\b/gi, 'var(--surface-1)'],
  [/#1a1a24\b/gi, 'var(--surface-2)'],
  [/#22222e\b/gi, 'var(--surface-3)'],
  [/#2e2e3c\b/gi, 'var(--surface-hover)'],
  [/#383848\b/gi, 'var(--surface-active)'],
  [/#161620\b/gi, 'var(--surface-inset)'],
];

const FILES = fs.readdirSync(path).filter(f => f.endsWith('.css') && f !== 'tokens.css');
let totalReplaced = 0;
const report = [];

for (const file of FILES) {
  const fp = path + file;
  let css = fs.readFileSync(fp, 'utf8');
  let fileReplaced = 0;
  for (const [re, token] of MAPPINGS) {
    const before = css;
    css = css.replace(re, token);
    const n = (before.match(re) || []).length;
    fileReplaced += n;
  }
  if (fileReplaced > 0) {
    fs.writeFileSync(fp, css, 'utf8');
    report.push(`${file}: ${fileReplaced} 处替换`);
    totalReplaced += fileReplaced;
  }
  // 残留 hex 统计
  const remaining = css.match(/#[0-9a-fA-F]{3,8}\b/g) || [];
  if (remaining.length > 0) {
    const uniq = [...new Set(remaining)];
    report.push(`  ⚠️ ${file} 残留 hex ${remaining.length} 处: ${uniq.slice(0, 12).join(' ')}${uniq.length > 12 ? '...' : ''}`);
  }
}

console.log(`共替换 ${totalReplaced} 处\n`);
console.log(report.join('\n'));
