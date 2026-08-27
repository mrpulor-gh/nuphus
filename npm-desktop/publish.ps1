# npm-desktop/publish.ps1
# Nuphus Desktop npm release pipeline:
#   download platform assets from GitHub Releases
#   -> assemble 4 packages (meta + 3 platform) -> publish -> verify install
#
# Usage (run from repo root via npm script):
#   npm run publish:npm                      # release current version (reads src-tauri/tauri.conf.json)
#   npm run publish:npm -- -Version 0.1.3    # explicit version（显式版本号用 PS 风格）
#   npm run publish:npm -- --dry-run         # preflight: version/auth/existing-version/asset checks only
#   npm run publish:npm -- --skip-download   # reuse assets already under downloads/<version>/
#   npm run publish:npm -- --skip-verify     # skip post-publish install verification
# 开关同时接受 npm 风格（--dry-run 等，脚本内防呆归位）与 PS 风格（-DryRun）。
# 注意：npm→-Command 路径下多余参数以位置参数进入脚本——开关靠防呆块识别；
# 显式版本号必须写 -Version x.y.z（--version 形式无法携带值，会明确报错）。
#
# Requirements:
#   - Windows 10+ (uses built-in tar.exe for zip and tar.gz extraction)
#   - npm authenticated with publish rights on the @nuphus scope (npm whoami)
#   - GitHub Release tag v<version> already published with assets (2026-08-24 命名格式：
#     nuphus-<platform>-<version>，由 .github/workflows/release.yml 打包上传):
#       nuphus-win32-x64-<version>.zip / nuphus-osx-arm64-<version>.zip / nuphus-linux-x64-<version>.tar.gz

[CmdletBinding()]
param(
    [string]$Version = "",
    [switch]$SkipDownload,
    [switch]$SkipVerify,
    [switch]$DryRun,
    # npm 风格开关经位置传入时的兜底收口（--skip-download --dry-run 等多标记）
    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$ExtraFlags
)

# 防呆（2026-08-24 实测事故）：npm 风格的 '--skip-download' 无法绑定 PS 开关
# （参数名匹配不含连字符），会按位置落进 $Version，被当成版本号拼出
# v--skip-download 的下载 URL 空转 10 分钟后 404。此处统一识别归位：
$flagTokens = @($Version, $ExtraFlags) | Where-Object { $_ -match '^-' }
if ($flagTokens.Count -gt 0) {
    foreach ($t in $flagTokens) {
        if     ($t -match 'skip.download') { $SkipDownload = $true }
        elseif ($t -match 'skip.verify')   { $SkipVerify   = $true }
        elseif ($t -match 'dry')           { $DryRun       = $true }
        else { throw "无法识别的参数 '$t'——开关用 -SkipDownload/-SkipVerify/-DryRun/--skip-download 等；显式版本号用 -Version x.y.z" }
    }
    if ($Version -match '^-') { $Version = "" }
}

$ErrorActionPreference = 'Stop'

$RepoRoot   = Split-Path -Parent $PSScriptRoot
$NpmDesktop = $PSScriptRoot
$Downloads  = Join-Path $NpmDesktop 'downloads'
$Packages   = Join-Path $NpmDesktop 'packages'
$ReleaseUrl = 'https://github.com/mrpulor-gh/nuphus/releases/download'
$Registry   = 'https://registry.npmjs.org'
$MetaName   = 'nuphus-desktop'

$Platforms = @(
    @{
        Name     = 'nuphus-desktop-win32-x64'
        Asset    = 'nuphus-win32-x64-{0}.zip'
        Dir      = 'win64'
        Os       = 'win32'
        Cpu      = 'x64'
        Desc     = 'Nuphus desktop binary for win32 x64. Installed automatically by the @nuphus/nuphus-desktop meta package.'
        Keywords = @('nuphus', 'desktop', 'win32')
    },
    @{
        Name     = 'nuphus-desktop-osx-arm64'
        Asset    = 'nuphus-osx-arm64-{0}.zip'
        Dir      = 'macos'
        Os       = 'darwin'
        Cpu      = 'arm64'
        Desc     = 'Nuphus desktop binary for macOS arm64 (Apple Silicon). Installed automatically by the @nuphus/nuphus-desktop meta package.'
        Keywords = @('nuphus', 'desktop', 'macos', 'arm64')
    },
    @{
        Name     = 'nuphus-desktop-linux-x64'
        Asset    = 'nuphus-linux-x64-{0}.tar.gz'
        Dir      = 'linux'
        Os       = 'linux'
        Cpu      = 'x64'
        Desc     = 'Nuphus desktop binary for linux x64. Installed automatically by the @nuphus/nuphus-desktop meta package.'
        Keywords = @('nuphus', 'desktop', 'linux')
    }
)

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-Ok($msg)   { Write-Host "    $msg" -ForegroundColor Green }
function Write-WarnMsg($msg) { Write-Host "    WARN: $msg" -ForegroundColor Yellow }

function Resolve-Version {
    if ($Version) { return $Version }
    $tauriConf = Join-Path $RepoRoot 'src-tauri\tauri.conf.json'
    if (-not (Test-Path $tauriConf)) { throw "tauri.conf.json not found: $tauriConf" }
    $v = (Get-Content $tauriConf -Raw -Encoding UTF8 | ConvertFrom-Json).version
    if (-not $v) { throw 'Cannot read version from src-tauri/tauri.conf.json' }
    return $v
}

function Test-NpmAuth {
    Write-Step 'Checking npm authentication (npm whoami)'
    if ($DryRun) {
        $who = & npm whoami --registry $Registry 2>$null
        if ($LASTEXITCODE -ne 0 -or -not $who) {
            Write-WarnMsg 'npm whoami failed (dry-run: continue, but real publish will need auth)'
        } else {
            Write-Ok "authenticated as: $who"
        }
        return
    }
    $who = & npm whoami --registry $Registry 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $who) {
        throw 'npm not authenticated. Run `npm login` or set NPM_TOKEN first (must have publish rights on @nuphus scope).'
    }
    Write-Ok "authenticated as: $who"
}

function Get-PublishedVersion($pkgName) {
    $v = & npm view $pkgName version --registry $Registry 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return $v
}

function Test-NotPublished($pkgName, $version) {
    $published = Get-PublishedVersion $pkgName
    if ($published -eq $version) {
        throw "SKIP: $pkgName@$version already published. Bump the version or unpublish first (npm unpublish is discouraged)."
    }
    if ($published) {
        Write-WarnMsg "$pkgName already has version $published (releasing $version). Publishing a NEW version only."
    } else {
        Write-Ok "$pkgName not on registry yet (first publish)"
    }
}

function Get-Asset($p, $version) {
    # Asset 为命名模板（{0}=version）：nuphus-<platform>-<version> 格式
    $assetName = $p.Asset -f $version
    $assetUrl = "$ReleaseUrl/v$version/$assetName"
    # 版本隔离缓存：downloads/<version>/<asset>，避免同名资产跨版本误复用（资产名不随版本变）
    $versionDir = "$Downloads\$version"
    $localFile = "$versionDir\$assetName"
    if (Test-Path $localFile) {
        Write-Ok "asset already cached (v$version): $assetName (size $((Get-Item $localFile).Length) bytes)"
        return $localFile
    }
    if ($DryRun) {
        Write-Ok "[dry-run] would download $assetUrl"
        return $null
    }
    Write-Step "Downloading $assetName <- $assetUrl"
    New-Item -ItemType Directory -Force -Path $versionDir | Out-Null
    try {
        Invoke-WebRequest -Uri $assetUrl -OutFile $localFile -UseBasicParsing -TimeoutSec 600
    } catch {
        throw "Failed to download $assetUrl : $($_.Exception.Message)`nCheck that GitHub Release v$version exists and the asset name matches release.yml."
    }
    Write-Ok "downloaded $assetName ($((Get-Item $localFile).Length) bytes)"
    return $localFile
}

function Build-PlatformPackage($p, $version, $assetFile) {
    $pkgDir = Join-Path $Packages $p.Name
    Write-Step "Assembling $($p.Name)@$version"

    if ($DryRun) {
        Write-Ok "[dry-run] would rebuild $pkgDir from $($p.Asset)"
        return
    }

    # Rebuild the package directory from scratch (assets change every release)
    if (Test-Path $pkgDir) { Remove-Item $pkgDir -Recurse -Force }
    New-Item -ItemType Directory -Path $pkgDir | Out-Null

    # Extract asset into package dir (tar.exe handles both .zip and .tar.gz on Win10+)
    Push-Location $pkgDir
    try {
        & tar -xf $assetFile
        if ($LASTEXITCODE -ne 0) { throw "tar extraction failed for $assetFile" }
    } finally {
        Pop-Location
    }

    # Generate platform package.json (UTF-8 no BOM: npm/Node parse cleanly)
    $pkgJson = @{
        name        = "@nuphus/$($p.Name)"
        version     = $version
        description = $p.Desc
        license     = 'Apache-2.0'
        repository  = @{ type = 'git'; url = 'git+https://github.com/mrpulor-gh/nuphus.git' }
        os          = @($p.Os)
        cpu         = @($p.Cpu)
        keywords    = $p.Keywords
    }
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    $json = ($pkgJson | ConvertTo-Json -Depth 5) -replace '\\/', '/'
    [System.IO.File]::WriteAllText((Join-Path $pkgDir 'package.json'), $json, $utf8NoBom)

    # .npmignore: platform binaries are fully controlled here; ignore nothing extra
    [System.IO.File]::WriteAllText((Join-Path $pkgDir '.npmignore'), '# Platform binary package: publish everything extracted from the release asset.', $utf8NoBom)

    Write-Ok "$($p.Name) assembled ($((Get-ChildItem $pkgDir -Recurse -File | Measure-Object).Count) files)"
}

function Update-MetaPackage($version) {
    $metaDir = Join-Path $Packages $MetaName
    $pkgPath = Join-Path $metaDir 'package.json'
    Write-Step "Updating $MetaName@$version"

    if ($DryRun) {
        Write-Ok "[dry-run] would set meta package version=$version + optionalDependencies=$version"
        return
    }
    if (-not (Test-Path $pkgPath)) { throw "meta package.json not found: $pkgPath" }

    # 只更新 version 与三个 optionalDependencies 版本字段，保留手写格式。
    # 勿用 ConvertTo-Json 往返重写：PS 5.1 输出会破坏格式（冒号后双空格、层级缩进错乱），
    # 2026-08-23 曾因此把被 git 跟踪的 meta package.json 写乱。
    $raw = Get-Content $pkgPath -Raw -Encoding UTF8
    $raw = [regex]::Replace($raw, '"version"\s*:\s*"[^"]*"', "`"version`": `"$version`"")
    foreach ($n in $Platforms | ForEach-Object { "@nuphus/$($_.Name)" }) {
        # PS 字符串内引号必须用反引号转义（\" 是 C# 写法，PowerShell 会提前终止字符串）
        $esc = [regex]::Escape($n)
        $raw = [regex]::Replace($raw, "`"$esc`"\s*:\s*`"[^`"]*`"", "`"$n`": `"$version`"")
    }
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($pkgPath, $raw, $utf8NoBom)
    Write-Ok "$MetaName package.json updated"
}

function Test-VersionConsistency($version) {
    Write-Step 'Version consistency check'
    $all = @($MetaName) + @($Platforms | ForEach-Object { $_.Name })
    foreach ($n in $all) {
        $pkgPath = Join-Path $Packages "$n\package.json"
        if (-not (Test-Path $pkgPath)) { throw "missing package.json: $pkgPath" }
        $v = (Get-Content $pkgPath -Raw -Encoding UTF8 | ConvertFrom-Json).version
        if ($v -ne $version) { throw "$n version mismatch: package.json=$v expected=$version" }
    }
    Write-Ok "all 4 packages at $version"
}

function Publish-Package($pkgName, $version) {
    $dir = Join-Path $Packages $pkgName
    if ($DryRun) {
        Write-Ok "[dry-run] npm publish $dir"
        return
    }
    Write-Step "npm publish $pkgName@$version"
    & npm publish $dir --registry $Registry
    if ($LASTEXITCODE -ne 0) { throw "npm publish failed for $pkgName" }
    Write-Ok "$pkgName@$version published"
}

function Verify-Install($version) {
    if ($DryRun) { Write-Ok '[dry-run] would run install verification'; return }
    if ($SkipVerify) { Write-Ok 'install verification skipped (--skip-verify)'; return }

    Write-Step "Verifying install of @nuphus/nuphus-desktop@$version"
    $testDir = Join-Path $env:TEMP ("nuphus-install-test-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $testDir | Out-Null
    try {
        Push-Location $testDir
        try {
            & npm init -y 2>$null | Out-Null
            & npm install "@nuphus/nuphus-desktop@$version" --registry $Registry 2>$null
            if ($LASTEXITCODE -ne 0) { throw 'npm install verification failed' }
            $bin = Join-Path $testDir 'node_modules\.bin\nuphus.cmd'
            if (-not (Test-Path $bin)) { throw "launcher not found after install: $bin" }
            Write-Ok "install verification passed (launcher: $bin)"
        } finally {
            Pop-Location
        }
    } finally {
        Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------- main
$version = Resolve-Version
Write-Host ''
Write-Host '============================================================' -ForegroundColor Magenta
Write-Host " Nuphus Desktop npm release pipeline" -ForegroundColor Magenta
Write-Host "   version : $version" -ForegroundColor Magenta
Write-Host "   mode    : $(if ($DryRun) { 'DRY-RUN (no download/publish)' } else { 'LIVE' })" -ForegroundColor Magenta
Write-Host '============================================================' -ForegroundColor Magenta
Write-Host ''

Test-NpmAuth

foreach ($p in $Platforms) { Test-NotPublished "@nuphus/$($p.Name)" $version }
Test-NotPublished "@nuphus/$MetaName" $version

$assetFiles = @{}
foreach ($p in $Platforms) {
    if ($SkipDownload) {
        # 只用版本隔离缓存，缺失即报错（不联网）；缓存由不带开关的完整跑一次填充
        $local = "$Downloads\$version\$($p.Asset -f $version)"
        if (-not (Test-Path $local)) {
            throw "--skip-download: 缓存缺失 $local ——先去掉开关完整运行一次完成下载"
        }
        Write-Ok "using cached (skip-download): $(Split-Path -Leaf $local) (size $((Get-Item $local).Length) bytes)"
        $assetFiles[$p.Name] = $local
    } else {
        $f = Get-Asset $p $version
        if ($f) { $assetFiles[$p.Name] = $f }
    }
}

foreach ($p in $Platforms) {
    Build-PlatformPackage $p $version $assetFiles[$p.Name]
}
Update-MetaPackage $version
if ($DryRun) {
    Write-Ok '[dry-run] version consistency check would run after meta update (skipped in dry-run)'
} else {
    Test-VersionConsistency $version
}

Write-Step 'Publishing (platform packages first, then meta)'
foreach ($p in $Platforms) { Publish-Package $p.Name $version }
Publish-Package $MetaName $version

# Verify published versions on registry (with propagation retry:
# npm registry is eventually-consistent; immediate view may return old version)
Write-Step 'Registry verification'
if ($DryRun) {
    Write-Ok '[dry-run] would verify all 4 packages on registry after publish (skipped)'
} else {
    foreach ($n in @($MetaName) + @($Platforms | ForEach-Object { $_.Name })) {
        $v = $null
        for ($attempt = 1; $attempt -le 4; $attempt++) {
            $v = Get-PublishedVersion "@nuphus/$n"
            if ($v -eq $version) { break }
            Write-Ok "[retry $attempt/4] @nuphus/$n not yet $version (got '$v'), waiting for registry propagation..."
            Start-Sleep -Seconds 5
        }
        if ($v -ne $version) { throw "registry verification failed: @nuphus/$n expected $version got $v" }
        Write-Ok "@nuphus/$n@$v confirmed on registry"
    }
}

Verify-Install $version

Write-Host ''
Write-Host '============================================================' -ForegroundColor Green
Write-Host " DONE: @nuphus/$MetaName@$version + 3 platform packages published" -ForegroundColor Green
Write-Host '============================================================' -ForegroundColor Green