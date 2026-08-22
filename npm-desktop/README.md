# Nuphus Desktop — npm 发布

`@nuphus/nuphus-desktop` 是 Nuphus 桌面应用的 npm 分发包：

- **主包** `@nuphus/nuphus-desktop`：启动器（`nuphus` 命令）+ optionalDependencies 平台选择
- **平台子包**：`@nuphus/nuphus-desktop-win32-x64` / `-osx-arm64` / `-linux-x64`（各自携带平台二进制）

发布流程已固化为脚本，一条命令完成：**下载 GitHub Release 资产 → 组装 4 个包 → 发布 → 验证安装**。

## 一键发布

```bash
# 发布当前版本（版本号自动读取 src-tauri/tauri.conf.json）
npm run publish:npm

# 指定版本
npm run publish:npm -- --version 0.1.3
```

## 前置条件

1. **代码已打 tag 并推送**，GitHub Actions 已完成 Release 构建上传（release.yml）——资产名固定：
   - `nuphus_windows_amd64.zip`
   - `nuphus_macos_arm64.zip`
   - `nuphus_linux_amd64.tar.gz`
2. **npm 已认证**：`npm whoami` 可返回账号（需对 `@nuphus` scope 有发布权限，token 建议放环境变量 `NPM_TOKEN`）
3. Windows 10+（脚本用系统自带 `tar.exe` 解压 zip / tar.gz）

## 常用参数

| 参数 | 说明 |
|------|------|
| `--version 0.1.3` | 指定发布版本（默认读 tauri.conf.json） |
| `--dry-run` | 演练模式：只做版本 / 认证 / 已发布检查，不下载、不发布 |
| `--skip-download` | 复用 `downloads/` 下已缓存的资产 |
| `--skip-verify` | 跳过发布后的安装验证 |

```bash
# 发版前演练（推荐先跑一次）
npm run publish:npm -- --dry-run
```

## 发布流程细节

```
版本解析（tauri.conf.json） → npm whoami 认证检查
→ npm view 查重（已发布同版本则中止，防覆盖）
→ 下载 3 平台资产 → 解压组装平台包（重建 packages/<pkg>/）
→ 主包更新 version + optionalDependencies
→ 版本一致性校验（4 包同版本）
→ 发布顺序：3 平台子包 → 主包
→ registry 复查 4 包版本
→ 临时目录 npm install + 启动器存在性验证 → 清理
```

## 目录结构

```
npm-desktop/
├── publish.ps1            # 发布脚本（勿手改版本号，脚本自动写）
├── downloads/             # GitHub Release 资产缓存（git 忽略；按版本隔离 downloads/<version>/）
└── packages/
    ├── nuphus-desktop/            # 主包：bin/nuphus.js + package.json + README（已入库）
    ├── nuphus-desktop-win32-x64/  # 平台包（git 忽略，发布时重建）
    ├── nuphus-desktop-osx-arm64/
    └── nuphus-desktop-linux-x64/
```

## 注意事项

- **禁止用 `npm unpublish` 覆盖已发布版本**：npm registry 不可变，发错版本只能升版本重发。
- 平台包目录每次发布**全量重建**（清空后从资产解压），避免残留旧文件。
- macOS 的 sherpa 语音库已内置于 Release 资产（`Nuphus.app`），无需额外下载。
- 发布产物（`downloads/`、平台包目录、`install-test/`）已在 `.gitignore` 忽略，不会污染仓库。
- 本目录不依赖 `npm install`，直接由仓库根 `npm run publish:npm` 调用。