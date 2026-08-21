const fs = require('fs');
const path = require('path');

// 简化的 ICO 生成器 - 生成多尺寸 PNG 并打包为 ICO
// ICO 格式: 目录头 + 图标目录项 + 图像数据

function createICO(pngBuffers) {
  const numImages = pngBuffers.length;
  const headerSize = 6;
  const dirEntrySize = 16;
  const dirSize = headerSize + numImages * dirEntrySize;
  
  let offset = dirSize;
  const entries = [];
  
  for (const png of pngBuffers) {
    // 解析 PNG 尺寸
    const width = png[16];  // IHDR width
    const height = png[20]; // IHDR height
    
    entries.push({
      width: width === 0 ? 256 : width,
      height: height === 0 ? 256 : height,
      colors: 0,
      reserved: 0,
      planes: 1,
      bpp: 32,
      size: png.length,
      offset: offset,
      data: png
    });
    
    offset += png.length;
  }
  
  // 构建 ICO
  const buf = Buffer.alloc(offset);
  let pos = 0;
  
  // 头部
  buf.writeUInt16LE(0, pos); pos += 2; // 保留
  buf.writeUInt16LE(1, pos); pos += 2; // 类型: 图标
  buf.writeUInt16LE(numImages, pos); pos += 2; // 数量
  
  // 目录项
  for (const entry of entries) {
    buf.writeUInt8(entry.width === 256 ? 0 : entry.width, pos); pos += 1;
    buf.writeUInt8(entry.height === 256 ? 0 : entry.height, pos); pos += 1;
    buf.writeUInt8(entry.colors, pos); pos += 1;
    buf.writeUInt8(entry.reserved, pos); pos += 1;
    buf.writeUInt16LE(entry.planes, pos); pos += 2;
    buf.writeUInt16LE(entry.bpp, pos); pos += 2;
    buf.writeUInt32LE(entry.size, pos); pos += 4;
    buf.writeUInt32LE(entry.offset, pos); pos += 4;
  }
  
  // 图像数据
  for (const entry of entries) {
    entry.data.copy(buf, pos);
    pos += entry.data.length;
  }
  
  return buf;
}

// 读取 SVG
const svgPath = path.join(__dirname, 'public', 'nuphus-icon-v3.svg');
const svgContent = fs.readFileSync(svgPath, 'utf8');

// 由于无法渲染 SVG，我们创建一个占位 ICO
// 实际应用中需要用 sharp 或 canvas 渲染 SVG 为 PNG
console.log('SVG loaded, size:', svgContent.length);
console.log('Note: Full ICO generation requires sharp or similar library');
console.log('Please install sharp: npm install sharp');
console.log('Then run: node convert-with-sharp.js');
