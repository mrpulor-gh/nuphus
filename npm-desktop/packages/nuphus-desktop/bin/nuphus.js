#!/usr/bin/env node
/**
 * Nuphus 桌面应用启动器（npm 一键安装入口）
 *
 * 通过 optionalDependencies 自动选择对应平台的二进制子包：
 *   win32-x64   -> @nuphus/nuphus-desktop-win32-x64  (nuphus.exe)
 *   darwin-arm64-> @nuphus/nuphus-desktop-osx-arm64  (Nuphus.app)
 *   linux-x64   -> @nuphus/nuphus-desktop-linux-x64  (nuphus)
 */
'use strict';

const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const PLATFORM_MAP = {
  'win32-x64': {
    pkg: 'nuphus-desktop-win32-x64',
    binary: 'nuphus.exe',
  },
  'darwin-arm64': {
    pkg: 'nuphus-desktop-osx-arm64',
    app: 'Nuphus.app',
  },
  'linux-x64': {
    pkg: 'nuphus-desktop-linux-x64',
    binary: 'nuphus',
  },
};

function resolveVendorDir() {
  const key = `${process.platform}-${process.arch}`;
  const spec = PLATFORM_MAP[key];
  if (!spec) {
    console.error(`[nuphus] 暂不支持该平台/架构: ${key}`);
    console.error('        当前支持: win32-x64 / darwin-arm64 / linux-x64');
    process.exit(1);
  }
  // 平台子包可能被 npm 提升到 <node_modules>/@nuphus/ 下（与主包同级），也可能被
  // 嵌套在主包的 node_modules/@nuphus/ 下；用 Node 模块解析定位，两种布局都能命中。
  const pkgName = `@nuphus/${spec.pkg}`;
  let vendor;
  try {
    vendor = path.dirname(require.resolve(`${pkgName}/package.json`));
  } catch (_) {
    vendor = path.join(__dirname, '..', '..', pkgName);
  }
  if (!fs.existsSync(vendor)) {
    console.error(`[nuphus] 未找到平台包: ${spec.pkg}`);
    console.error('        请确认安装完整（npm install -g @nuphus/nuphus-desktop）或重装。');
    process.exit(1);
  }
  return { vendor, spec };
}

function main() {
  const { vendor, spec } = resolveVendorDir();

  let cmd;
  let args = [];
  if (process.platform === 'darwin') {
    // macOS: open -a Nuphus.app（正确的应用启动方式）
    cmd = 'open';
    args = ['-a', path.join(vendor, spec.app)];
  } else {
    cmd = path.join(vendor, spec.binary);
  }

  const child = spawn(cmd, args, {
    stdio: 'ignore',
    detached: process.platform !== 'win32',
  });

  child.on('error', (err) => {
    console.error(`[nuphus] 启动失败: ${err.message}`);
    process.exit(1);
  });

  child.unref();
  console.log(`[nuphus] 正在启动 Nuphus 桌面应用…`);
}

main();
