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
    # 续跑：前一轮已发布部分包（npm 上传成功但脚本被终止）时，
    # 对「已发布」的包跳过而不是 throw，只发剩余包。
    [switch]$Resume,
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
        elseif ($t -match 'resume')        { $Resume       = $true }
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
# GitHub Release API（digest 权威来源，本机直连可达；schannel 握手失败的机器设 NUPHUS_GITHUB_PROXY）
$ReleaseApi = 'https://api.github.com/repos/mrpulor-gh/nuphus'
# 国内镜像前缀（2026-09-28 本机实测：gh-proxy ~6.4 MB/s >> cors.isteed ~548 KB/s > ghfast.top ~295 KB/s；
# 而 GitHub 资产直连实测 TLS 被重置或仅 19 KB/s）。镜像只加速传输、不承担可信度：每个资产都要与
# Release API 的 sha256 digest 对账，对不上即删除换源；取不到 digest 时只允许 GitHub 权威源，
# 未校验的镜像产物不得进入 npm 包。
$MirrorUrls = @('https://gh-proxy.com', 'https://cors.isteed.cc')
# 访问 GitHub API 需经代理的机器设此环境变量，如 http://127.0.0.1:2081
$ApiProxy = $(if ($env:NUPHUS_GITHUB_PROXY) { $env:NUPHUS_GITHUB_PROXY } else { '' })
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
        $who = & npm.cmd whoami --registry $Registry 2>$null
        if ($LASTEXITCODE -ne 0 -or -not $who) {
            Write-WarnMsg 'npm whoami failed (dry-run: continue, but real publish will need auth)'
        } else {
            Write-Ok "authenticated as: $who"
        }
        return
    }
    $who = & npm.cmd whoami --registry $Registry 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $who) {
        throw 'npm not authenticated. Run `npm login` or set NPM_TOKEN first (must have publish rights on @nuphus scope).'
    }
    Write-Ok "authenticated as: $who"
}

function Get-PublishedVersion($pkgName) {
    $v = & npm.cmd view $pkgName version --registry $Registry 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return $v
}

function Test-NotPublished($pkgName, $version) {
    $published = Get-PublishedVersion $pkgName
    if ($published -eq $version) {
        # registry 不可变：同版本重发必失败。默认仍 throw（防误重发），
        # -Resume 下跳过已发完成的包，只发剩余包。
        if ($Resume) {
            Write-Ok "resume: $pkgName@$version already published, skipping (registry is immutable, republish would fail anyway)"
            return $false
        }
        throw "SKIP: $pkgName@$version already published. Bump the version or unpublish first (npm unpublish is discouraged)."
    }
    if ($published) {
        Write-WarnMsg "$pkgName already has version $published (releasing $version). Publishing a NEW version only."
    } else {
        Write-Ok "$pkgName not on registry yet (first publish)"
    }
    # 需要发布才返回 $true；-Resume 下已发布返回 $false（供主流程跳过）
    return $true
}

function Get-ReleaseDigest($assetName, $version) {
    # 权威 digest 取 GitHub Release API（assets[].digest = "sha256:<hex>"，本机直连可达）。
    # 取不到返回 $null：调用方据此刻意拒绝未校验镜像，只走 GitHub 权威源。
    try {
        $req = @{
            Uri             = "$ReleaseApi/releases/tags/v$version"
            UseBasicParsing = $true
            TimeoutSec      = 60
        }
        if ($ApiProxy) { $req['Proxy'] = $ApiProxy }
        $res = Invoke-WebRequest @req
        $rel = $res.Content | ConvertFrom-Json
        $hit = @($rel.assets) | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
        if (-not $hit) { return $null }
        $dig = [string]$hit.digest
        if ($dig -match '^sha256:([0-9a-fA-F]{64})$') { return $Matches[1].ToLower() }
        return $null
    } catch {
        return $null
    }
}
function Get-Asset($p, $version) {
    # Asset 为命名模板（{0}=version）：nuphus-<platform>-<version> 格式
    $assetName = $p.Asset -f $version
    # 版本隔离缓存：downloads/<version>/<asset>，避免同名资产跨版本误复用（资产名不随版本变）
    $versionDir = "$Downloads\$version"
    $localFile = "$versionDir\$assetName"
    if (Test-Path $localFile) {
        Write-Ok "asset already cached (v$version): $assetName (size $((Get-Item $localFile).Length) bytes)"
        return $localFile
    }
    if ($DryRun) {
        Write-Ok "[dry-run] would download $assetName <- $ReleaseUrl/v$version/ (mirrors: $($MirrorUrls -join ' '))"
        return $null
    }
    New-Item -ItemType Directory -Force -Path $versionDir | Out-Null
    # 期望 sha256：取到 digest 才镜像优先；取不到只走 GitHub 权威源（不降级成未校验镜像）
    $expected = Get-ReleaseDigest $assetName $version
    if ($expected) {
        Write-Ok "expected sha256 (Release API): $expected"
    } else {
        Write-WarnMsg "取不到 $assetName 的 Release digest —— 本资产只用 GitHub 权威源，不经镜像"
    }
    # 来源顺序：镜像优先（国内实测快一个数量级），GitHub 权威源兜底。
    # 只有拿到 digest 才把镜像放进候选——未校验的镜像字节不得进入 npm 包。
    $sources = @()
    if ($expected) {
        foreach ($m in $MirrorUrls) {
            $sources += @{ Label = "mirror $m"; Uri = "$m/$ReleaseUrl/v$version/$assetName" }
        }
    }
    $sources += @{ Label = 'github'; Uri = "$ReleaseUrl/v$version/$assetName" }
    $lastErr = ''
    foreach ($s in $sources) {
        # 一律先落 .tmp：校验不过或中途失败即删除，截断/污染文件不得留在缓存里被 -SkipDownload 复用
        $tmp = "$localFile.tmp"
        Remove-Item $tmp -Force -ErrorAction SilentlyContinue
        Write-Step "Downloading $assetName <- $($s.Uri)"
        try {
            $req = @{ Uri = $s.Uri; OutFile = $tmp; UseBasicParsing = $true; TimeoutSec = 1200 }
            # 镜像必须直连（叠本地代理会被限速回 40 KB/s）；只有裸 GitHub 域名才按需走代理
            if ($s.Label -eq 'github' -and $ApiProxy) { $req['Proxy'] = $ApiProxy }
            Invoke-WebRequest @req | Out-Null
            if (-not (Test-Path $tmp)) { throw 'download produced no file' }
            $size = (Get-Item $tmp).Length
            if ($size -le 0) { throw 'zero-byte file' }
            $actual = (Get-FileHash -Path $tmp -Algorithm SHA256).Hash.ToLower()
            if ($expected -and $actual -ne $expected) {
                throw "sha256 mismatch: expected $expected, got $actual"
            }
            Move-Item -Path $tmp -Destination $localFile -Force
            if ($expected) {
                Write-Ok "downloaded $assetName via $($s.Label) ($size bytes, sha256 verified: $actual)"
            } else {
                # 只说事实：没有权威 digest 可比对时这只是记录哈希，不叫「已校验」
                Write-Ok "downloaded $assetName via $($s.Label) ($size bytes, sha256=$actual, 无 Release digest 可比对、未经校验)"
            }
            return $localFile
        } catch {
            $lastErr = $_.Exception.Message
            Write-WarnMsg "$($s.Label) failed: $lastErr"
            Remove-Item $tmp -Force -ErrorAction SilentlyContinue
        }
    }
    throw "Failed to download $assetName from any source (mirrors + github); last error: $lastErr. Check that GitHub Release v$version exists and the asset name matches release.yml."
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
    # 显式走系统 bsdtar：$assetFile 是 C:\... 绝对路径，而裸名 `tar` 会走 PATH——若 PATH 上
    # 恰好有 GNU tar 抢先（从 git-bash 里跑本脚本时 /usr/bin/tar 就在前面），它会把 "C:" 当
    # 远程主机规格，报 "Cannot connect to C: resolve failed" 而整个发布失败。
    # System32\tar.exe 是 Win10+ 自带 bsdtar，认 Windows 绝对路径；缺失时才回退裸名。
    $tarExe = Join-Path $env:SystemRoot 'System32\tar.exe'
    if (-not (Test-Path $tarExe)) { $tarExe = 'tar' }
    Push-Location $pkgDir
    try {
        & $tarExe -xf $assetFile
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
    # 捕获输出：npm 的 `+ <pkg>@<version>` 行 + exit 0 才是「发布成功」的权威信号。
    # registry view 滞后不得推翻它（见下文 Registry verification）。
    # npm 会把 notice / deprecation 打到 stderr；PS 5.1 用 2>&1 合并后这些行变成
    # ErrorRecord，在全局 $ErrorActionPreference='Stop' 下会直接终止整条发布链——
    # v0.2.24 实测在第一个包上就这样死掉（`npm.cmd : npm notice`，四包一个未发）。
    # 因此仅本次调用把 EAP 降为 Continue，让 stderr 作为数据被捕获；
    # 成功判定不变（下方仍要求确认行 + exit 0），不是「永不失败」。
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $out = & npm.cmd publish $dir --registry $Registry 2>&1
    } finally {
        $ErrorActionPreference = $prevEap
    }
    $exit = $LASTEXITCODE
    if ($exit -ne 0) {
        $out | Select-Object -Last 20 | ForEach-Object { Write-Host "    $_" }
        throw "npm publish failed for $pkgName (exit $exit)"
    }
    $confirmed = $false
    foreach ($line in $out) {
        $s = ("$line").Trim()
        if ($s.StartsWith('+') -and $s.Contains("@nuphus/$pkgName") -and $s.Contains($version)) {
            $confirmed = $true
            break
        }
    }
    if (-not $script:PublishResults.ContainsKey($pkgName)) { $script:PublishResults[$pkgName] = $false }
    $script:PublishResults[$pkgName] = $confirmed
    if ($confirmed) {
        Write-Ok "$pkgName@$version published (npm confirmed)"
    } else {
        # 不假装成功：exit 0 但没抓到确认行，按「未确认」处理
        Write-WarnMsg "$pkgName@$version returned exit 0 but no '+ pkg@version' confirmation line matched (treating as UNCONFIRMED)"
        $out | Select-Object -Last 10 | ForEach-Object { Write-Host "    $_" }
    }
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
            & npm.cmd init -y 2>$null | Out-Null
            & npm.cmd install "@nuphus/nuphus-desktop@$version" --registry $Registry 2>$null
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

# 返回值收进哈希，避免落到输出流；-Resume 下已发布的包在发布段整段跳过
# （registry 不可变，重发只会拿到 E403）。不加 -Resume 时 Test-NotPublished 自己 throw。
$needsPublish = @{}
foreach ($p in $Platforms) { $needsPublish[$p.Name] = Test-NotPublished "@nuphus/$($p.Name)" $version }
$needsPublish[$MetaName] = Test-NotPublished "@nuphus/$MetaName" $version

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
# pkgName -> $true when npm itself printed `+ <pkg>@<version>` and exited 0.
# Registry view lag must never override this (see Registry verification below).
$script:PublishResults = @{}
foreach ($p in $Platforms) {
    if ($needsPublish[$p.Name]) { Publish-Package $p.Name $version }
    else { Write-Ok "skipping publish of @nuphus/$($p.Name)@$version (already on registry)" }
}
if ($needsPublish[$MetaName]) { Publish-Package $MetaName $version }
else { Write-Ok "skipping publish of @nuphus/$MetaName@$version (already on registry)" }

# Verify published versions on registry.
#
# npm registry is eventually-consistent: `npm view <pkg> version` reads the `latest`
# dist-tag, which can lag a *successful* publish by minutes (measured: 150s on v0.2.23,
# across all 4 packages). A lagging view must NEVER be reported as a failed publish:
#
#   - npm's own `+ <pkg>@<version>` + exit 0 (captured in $script:PublishResults) is the
#     authoritative signal that the publish happened.
#   - If npm confirmed but the view still lags -> WARN + tell the user how to verify.
#     Throwing here would be a FALSE FAILURE, and the harmful reaction is republishing
#     (npm registry is immutable -> same version can never be republished).
#   - Only throw when npm itself did not confirm the publish.
Write-Step 'Registry verification'
if ($DryRun) {
    Write-Ok '[dry-run] would verify all 4 packages on registry after publish (skipped)'
} else {
    # Global budget, not per-package: propagation is one registry-wide lag, so poll all
    # unresolved packages together each round. Worst case is the budget, not budget x N.
    # (An earlier 20 x 15s per-package version could stall ~20 min across 4 packages.)
    $budgetMin = 5
    if ($env:NPM_VERIFY_BUDGET_MIN) { $budgetMin = [int]$env:NPM_VERIFY_BUDGET_MIN }
    $sleepSecs = 15
    $deadline = (Get-Date).AddMinutes($budgetMin)
    $pending = @($MetaName) + @($Platforms | ForEach-Object { $_.Name })
    $firstRound = $true
    while ($pending.Count -gt 0) {
        if (-not $firstRound) { Start-Sleep -Seconds $sleepSecs }
        $firstRound = $false
        $still = @()
        foreach ($n in $pending) {
            $pkg = "@nuphus/$n"
            $v = Get-PublishedVersion $pkg
            if ($v -eq $version) {
                Write-Ok "$pkg@$v confirmed on registry"
            } else {
                $still += $n
            }
        }
        $pending = $still
        if ($pending.Count -gt 0) {
            if ((Get-Date) -ge $deadline) { break }
            Write-Ok "registry propagation pending ($($pending.Count)): $($pending -join ', ') - retrying in ${sleepSecs}s..."
        }
    }
    foreach ($n in $pending) {
        $pkg = "@nuphus/$n"
        $lastView = Get-PublishedVersion $pkg
        $npmConfirmed = $script:PublishResults.ContainsKey($n) -and $script:PublishResults[$n]
        if ($npmConfirmed) {
            Write-WarnMsg "$pkg PUBLISHED (npm confirmed) but registry view still reports '$lastView' after ${budgetMin}min."
            Write-WarnMsg "  -> This is registry propagation lag, NOT a failed publish."
            Write-WarnMsg "  -> Do NOT republish: the npm registry is immutable, the same version cannot be published again."
            Write-WarnMsg "  -> Confirm manually:  npm view $pkg version --registry $Registry"
            Write-WarnMsg "  -> Or wait a few minutes and re-run that view command."
        } else {
            throw "registry verification failed for ${pkg}: expected $version got '$lastView' (npm did not confirm the publish either)"
        }
    }
}

Verify-Install $version

# ── Gitee 国内下载点同步 ──────────────────────────────────────────────────
# 国内用户没有本地下载渠道（Gitee 此前只有 v0.2.0 源码包），而 updater 的
# endpoint[] 第一顺位就是 Gitee raw 清单：不同步，国内用户会"成功读到旧清单"
# 而不回落到权威源 —— 那不是报错，是静默收不到更新。所以这是发版必做步骤。
# 详细机制与踩坑见 plugin/skills/community/git-pr-protocol/SKILL.md §7.6。
$SyncGitee = Join-Path $PSScriptRoot 'sync-gitee-release.ps1'
if (Test-Path $SyncGitee) {
    Write-Step 'Sync to Gitee (domestic download point)'
    try {
        # dry-run 必须传导给同步脚本：否则 publish 的 dry-run 会真去 Gitee 建
        # release、传资产、推 latest.json，dry-run 名不副实。
        $syncArgs = @('-NoProfile', '-File', $SyncGitee, '-Version', $version)
        if ($DryRun) { $syncArgs += '-DryRun' }
        & pwsh @syncArgs
    } catch {
        # 不同步不阻断 npm 发包（npm 是主渠道），但必须显式失败而不是假装成功
        Write-WarnMsg "Gitee sync FAILED: $($_.Exception.Message)"
        Write-WarnMsg '  Domestic users will NOT receive this version until it is synced.'
        Write-WarnMsg "  Re-run manually:  pwsh -File `"$SyncGitee`" -Version $version"
    }
} else {
    Write-WarnMsg "sync script not found: $SyncGitee (Gitee domestic point NOT updated)"
}

Write-Host ''
Write-Host '============================================================' -ForegroundColor Green
$allConfirmed = ($script:PublishResults.Count -eq 4) -and (($script:PublishResults.Values | Where-Object { $_ -ne $true } | Measure-Object).Count -eq 0)
if ($allConfirmed) {
    Write-Host " DONE: @nuphus/$MetaName@$version + 3 platform packages published (npm confirmed)" -ForegroundColor Green
} else {
    Write-WarnMsg "DONE WITH CAVEAT: @nuphus/$MetaName@$version — some packages were not confirmed by npm."
    Write-WarnMsg "  Registry is immutable; if a package really is missing, bump the version and republish."
}
Write-Host '============================================================' -ForegroundColor Green