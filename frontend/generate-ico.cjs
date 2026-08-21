const fs = require('fs');

// 生成多尺寸 ICO 文件
// 使用简化的位图渲染（不依赖外部库）

function createBMP(width, height, pixels) {
  const rowSize = Math.ceil((width * 4) / 4) * 4; // 每行对齐到4字节
  const imageSize = rowSize * height;
  const headerSize = 40;
  const fileSize = 14 + headerSize + imageSize;
  
  const buf = Buffer.alloc(fileSize);
  let pos = 0;
  
  // BMP 文件头
  buf.write('BM', pos); pos += 2;
  buf.writeUInt32LE(fileSize, pos); pos += 4;
  buf.writeUInt32LE(0, pos); pos += 4; // 保留
  buf.writeUInt32LE(14 + headerSize, pos); pos += 4; // 数据偏移
  
  // DIB 头 (BITMAPINFOHEADER)
  buf.writeUInt32LE(headerSize, pos); pos += 4;
  buf.writeInt32LE(width, pos); pos += 4;
  buf.writeInt32LE(height, pos); pos += 4; // 正高度 = 自下而上
  buf.writeUInt16LE(1, pos); pos += 2; // 平面数
  buf.writeUInt16LE(32, pos); pos += 2; // 每像素位数
  buf.writeUInt32LE(0, pos); pos += 4; // 压缩方式 (BI_RGB)
  buf.writeUInt32LE(imageSize, pos); pos += 4;
  buf.writeInt32LE(2835, pos); pos += 4; // X ppm
  buf.writeInt32LE(2835, pos); pos += 4; // Y ppm
  buf.writeUInt32LE(0, pos); pos += 4; // 颜色数
  buf.writeUInt32LE(0, pos); pos += 4; // 重要颜色
  
  // 像素数据 (BGRA, 自下而上)
  for (let y = height - 1; y >= 0; y--) {
    for (let x = 0; x < width; x++) {
      const idx = (y * width + x) * 4;
      buf.writeUInt8(pixels[idx + 2], pos++); // B
      buf.writeUInt8(pixels[idx + 1], pos++); // G
      buf.writeUInt8(pixels[idx + 0], pos++); // R
      buf.writeUInt8(pixels[idx + 3], pos++); // A
    }
    // 行对齐
    const padding = rowSize - (width * 4);
    for (let p = 0; p < padding; p++) {
      buf.writeUInt8(0, pos++);
    }
  }
  
  return buf;
}

function createICO(images) {
  const numImages = images.length;
  const headerSize = 6;
  const dirEntrySize = 16;
  const dirSize = headerSize + numImages * dirEntrySize;
  
  let offset = dirSize;
  const entries = [];
  
  for (const img of images) {
    entries.push({
      width: img.width > 255 ? 0 : img.width,
      height: img.height > 255 ? 0 : img.height,
      colors: 0,
      reserved: 0,
      planes: 1,
      bpp: 32,
      size: img.data.length,
      offset: offset,
      data: img.data
    });
    offset += img.data.length;
  }
  
  const buf = Buffer.alloc(offset);
  let pos = 0;
  
  // ICO 头
  buf.writeUInt16LE(0, pos); pos += 2;
  buf.writeUInt16LE(1, pos); pos += 2;
  buf.writeUInt16LE(numImages, pos); pos += 2;
  
  // 目录
  for (const entry of entries) {
    buf.writeUInt8(entry.width, pos); pos += 1;
    buf.writeUInt8(entry.height, pos); pos += 1;
    buf.writeUInt8(entry.colors, pos); pos += 1;
    buf.writeUInt8(entry.reserved, pos); pos += 1;
    buf.writeUInt16LE(entry.planes, pos); pos += 2;
    buf.writeUInt16LE(entry.bpp, pos); pos += 2;
    buf.writeUInt32LE(entry.size, pos); pos += 4;
    buf.writeUInt32LE(entry.offset, pos); pos += 4;
  }
  
  // 数据
  for (const entry of entries) {
    entry.data.copy(buf, pos);
    pos += entry.data.length;
  }
  
  return buf;
}

// 渲染 Nuphus 图标到像素
function renderIcon(size) {
  const pixels = Buffer.alloc(size * size * 4);
  const center = size / 2;
  const scale = size / 512;
  
  // 背景 #0a0a0f
  for (let i = 0; i < pixels.length; i += 4) {
    pixels[i] = 10;     // R
    pixels[i + 1] = 10; // G
    pixels[i + 2] = 15; // B
    pixels[i + 3] = 255; // A
  }
  
  // 边框
  const borderWidth = Math.max(2, Math.floor(2 * scale));
  const borderRadius = Math.floor(84 * scale);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const dx = Math.min(x, size - 1 - x);
      const dy = Math.min(y, size - 1 - y);
      
      // 圆角矩形边框
      if ((dx < borderWidth || dy < borderWidth) && 
          !(dx < borderRadius && dy < borderRadius && 
            Math.sqrt(Math.max(0, borderRadius - dx) ** 2 + Math.max(0, borderRadius - dy) ** 2) > borderRadius)) {
        const idx = (y * size + x) * 4;
        pixels[idx] = 0;
        pixels[idx + 1] = 212;
        pixels[idx + 2] = 255;
        pixels[idx + 3] = 30;
      }
    }
  }
  
  // 左臂
  const armWidth = Math.max(4, Math.floor(16 * scale));
  const armPoints = [
    {x: -140, y: -120}, {x: -60, y: -120}, {x: -60, y: -60},
    {x: -120, y: 0}, {x: -60, y: 60}, {x: -60, y: 120},
    {x: -140, y: 120}, {x: -140, y: 60}, {x: -200, y: 0}, {x: -140, y: -60}
  ];
  
  drawPolygon(pixels, size, center, scale, armPoints, armWidth, 0, 212, 255);
  
  // 右臂
  const rightArmPoints = armPoints.map(p => ({x: -p.x, y: p.y}));
  drawPolygon(pixels, size, center, scale, rightArmPoints, armWidth, 0, 212, 255);
  
  // 中心圆
  const coreRadius = Math.floor(36 * scale);
  const coreX = Math.floor(center);
  const coreY = Math.floor(center);
  
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const dist = Math.sqrt((x - coreX) ** 2 + (y - coreY) ** 2);
      if (dist <= coreRadius) {
        const idx = (y * size + x) * 4;
        const intensity = 1 - (dist / coreRadius);
        pixels[idx] = Math.floor(0 + intensity * 255);
        pixels[idx + 1] = Math.floor(212 + intensity * 43);
        pixels[idx + 2] = 255;
        pixels[idx + 3] = 255;
      }
    }
  }
  
  // 中心白点
  const whiteRadius = Math.floor(16 * scale);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const dist = Math.sqrt((x - coreX) ** 2 + (y - coreY) ** 2);
      if (dist <= whiteRadius) {
        const idx = (y * size + x) * 4;
        pixels[idx] = 255;
        pixels[idx + 1] = 255;
        pixels[idx + 2] = 255;
        pixels[idx + 3] = 255;
      }
    }
  }
  
  // 连接线
  const lineWidth = Math.max(2, Math.floor(7 * scale));
  drawLine(pixels, size, center, scale, -60, 0, -16, 0, lineWidth, 0, 212, 255);
  drawLine(pixels, size, center, scale, 16, 0, 60, 0, lineWidth, 0, 212, 255);
  
  return pixels;
}

function drawPolygon(pixels, size, center, scale, points, width, r, g, b) {
  for (let i = 0; i < points.length; i++) {
    const p1 = points[i];
    const p2 = points[(i + 1) % points.length];
    drawLine(pixels, size, center, scale, p1.x, p1.y, p2.x, p2.y, width, r, g, b);
  }
}

function drawLine(pixels, size, center, scale, x1, y1, x2, y2, width, r, g, b) {
  const sx1 = Math.floor(center + x1 * scale);
  const sy1 = Math.floor(center + y1 * scale);
  const sx2 = Math.floor(center + x2 * scale);
  const sy2 = Math.floor(center + y2 * scale);
  
  const dx = Math.abs(sx2 - sx1);
  const dy = Math.abs(sy2 - sy1);
  const steps = Math.max(dx, dy);
  
  for (let i = 0; i <= steps; i++) {
    const t = steps === 0 ? 0 : i / steps;
    const x = Math.floor(sx1 + (sx2 - sx1) * t);
    const y = Math.floor(sy1 + (sy2 - sy1) * t);
    
    for (let wy = -Math.floor(width/2); wy <= Math.floor(width/2); wy++) {
      for (let wx = -Math.floor(width/2); wx <= Math.floor(width/2); wx++) {
        const px = x + wx;
        const py = y + wy;
        if (px >= 0 && px < size && py >= 0 && py < size) {
          const idx = (py * size + px) * 4;
          pixels[idx] = r;
          pixels[idx + 1] = g;
          pixels[idx + 2] = b;
          pixels[idx + 3] = 255;
        }
      }
    }
  }
}

// 生成多尺寸
const sizes = [16, 32, 48, 64, 128, 256];
const images = [];

for (const size of sizes) {
  const pixels = renderIcon(size);
  const bmp = createBMP(size, size, pixels);
  images.push({ width: size, height: size, data: bmp });
  console.log(`Generated ${size}x${size}`);
}

// 创建 ICO
const ico = createICO(images);
fs.writeFileSync('public/nuphus.ico', ico);
console.log('ICO saved to public/nuphus.ico');
