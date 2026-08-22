# Nuphus Desktop

Nuphus - 协同共生桌面助手（AI 智慧协作伙伴）。

`npm` 一键安装：自动选择对应平台的二进制（win32-x64 / macOS arm64 / linux-x64），
无需手动下载安装包。

## 安装

```bash
# 全局安装（提供 nuphus 命令）
npm install -g @nuphus/nuphus-desktop

# 或免安装体验
npx @nuphus/nuphus-desktop
```

## 启动

```bash
nuphus
```

## 支持平台

| 平台 | 架构 | 二进制子包 |
|------|------|-----------|
| Windows | x64 | `@nuphus/nuphus-desktop-win32-x64` |
| macOS | arm64 (Apple Silicon) | `@nuphus/nuphus-desktop-osx-arm64` |
| Linux | x64 | `@nuphus/nuphus-desktop-linux-x64` |

## 说明

- 首次安装体积较大（桌面应用含本地模型），请耐心等待。
- 需要完整安装包（msi / dmg / deb）请前往 [GitHub Releases](https://github.com/mrpulor-gh/nuphus/releases)。

## License

Apache-2.0
