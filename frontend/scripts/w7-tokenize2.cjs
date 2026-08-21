// W7 第二轮：语义明确的残留色批量 token 化
const fs = require('fs');
const path = 'C:/Users/Administrator/Nuphus/frontend/src/styles/';

const MAPPINGS = [
  [/#fff\b/gi, 'var(--on-accent)'],
  [/#ffffff\b/gi, 'var(--on-accent)'],
  [/#60a5fa\b/gi, 'var(--accent-hover)'],
  [/#4ade80\b/gi, 'var(--success)'],
  [/#34d399\b/gi, 'var(--success)'],
  [/#f87171\b/gi, 'var(--error)'],
  [/#fbbf24\b/gi, 'var(--warning)'],
  [/#ea580c\b/gi, 'var(--warning)'],
  [/#6ecbf5\b/gi, 'var(--accent)'],
  [/#ff6060\b/gi, 'var(--error)'],
  [/#e0e0f0\b/gi, 'var(--fg-1)'],
  [/#b8d4e8\b/gi, 'var(--fg-2)'],
  [/#1e1e2e\b/gi, 'var(--surface-1)'],
  [/#0d0f1a\b/gi, 'var(--code-bg)'],
  [/#f7f7f9\b/gi, 'var(--fg-1)'],
  [/#e0e0e0\b/gi, 'var(--fg-1)'],
];

const FILES = fs.readdirSync(path).filter(f => f.endsWith('.css') && f !== 'tokens.css');
let total = 0;
for (const file of FILES) {
  const fp = path + file;
  let css = fs.readFileSync(fp, 'utf8');
  let n0 = 0;
  for (const [re, token] of MAPPINGS) {
    n0 += (css.match(re) || []).length;
    css = css.replace(re, token);
  }
  if (n0 > 0) { fs.writeFileSync(fp, css, 'utf8'); console.log(`${file}: ${n0} 处`); total += n0; }
}
console.log(`共 ${total} 处`);
