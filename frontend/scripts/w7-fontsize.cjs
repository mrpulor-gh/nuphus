// W7 px 字号批量 token 化（仅标准字阶值，非标值如 10.5/12.5 保留）
const fs = require('fs');
const path = 'C:/Users/Administrator/Nuphus/frontend/src/styles/';

const MAPPINGS = [
  [/font-size:\s*11px\b/g, 'font-size: var(--fz-xs)'],
  [/font-size:\s*12px\b/g, 'font-size: var(--fz-sm)'],
  [/font-size:\s*13px\b/g, 'font-size: var(--fz-md)'],
  [/font-size:\s*14px\b/g, 'font-size: var(--fz-lg)'],
  [/font-size:\s*16px\b/g, 'font-size: var(--fz-xl)'],
  [/font-size:\s*20px\b/g, 'font-size: var(--fz-2xl)'],
  [/font-size:\s*28px\b/g, 'font-size: var(--fz-3xl)'],
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
// 统计剩余 font-size px
let remain = 0;
for (const file of FILES) {
  const css = fs.readFileSync(path + file, 'utf8');
  remain += (css.match(/font-size:\s*[\d.]+px/g) || []).length;
}
console.log(`剩余 font-size px（含非标值）: ${remain}`);
