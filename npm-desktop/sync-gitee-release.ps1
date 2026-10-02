#Requires -Version 5.1
<#
.SYNOPSIS
    Sync a Nuphus release to Gitee so domestic users can download from gitee.com directly.

.DESCRIPTION
    Gitee only had a stale v0.2.0 source-code release, so domestic users had no local
    download channel at all. This script mirrors one version's assets from the GitHub
    authority into the Gitee mirror repo, and publishes a Gitee-specific latest.json
    whose Windows entry points at the Gitee asset (direct connection for CN users)
    while macOS / Linux keep the GitHub authority URL.

    Integrity is never delegated to the transport: every asset is downloaded from the
    GitHub authority (mirrors only as a transport), checked against the sha256 digest
    reported by the GitHub Release API, uploaded to Gitee, then downloaded BACK from
    Gitee and re-hashed. A byte mismatch aborts before anything is announced.

.NOTES
    Token handling: the Gitee token is read from `git credential fill` at runtime and
    kept in memory only - never written to disk, never printed.

    Supported switches (same conventions as publish.ps1):
      -Version x.y.z   explicit version (default: src-tauri/tauri.conf.json)
      -DryRun          print the plan, change nothing
      -SkipDownload    reuse already-cached assets under npm-desktop/downloads/<ver>/
      -KeepReleases N  keep at most N recent Gitee releases (default 3); older releases
                       are reported, never deleted without -PruneReleases
      -PruneReleases   with -KeepReleases, actually delete older releases (IRREVERSIBLE)
#>
[CmdletBinding()]
param(
    [string]$Version = '',
    [switch]$DryRun,
    [switch]$SkipDownload,
    [int]$KeepReleases = 3,
    [switch]$PruneReleases
)

$ErrorActionPreference = 'Stop'

$RepoRoot   = Split-Path -Parent $PSScriptRoot
$NpmDesktop = $PSScriptRoot
$Downloads  = Join-Path $NpmDesktop 'downloads'
$StageDir   = Join-Path $NpmDesktop 'gitee-stage'

# ── GitHub 权威源（digest 与签名的可信来源）─────────────────────────────────
$ReleaseUrl = 'https://github.com/mrpulor-gh/nuphus/releases/download'
$ReleaseApi = 'https://api.github.com/repos/mrpulor-gh/nuphus'
# 访问 GitHub API 需经代理的机器设 NUPHUS_GITHUB_PROXY，如 http://127.0.0.1:2081
$ApiProxy   = $(if ($env:NUPHUS_GITHUB_PROXY) { $env:NUPHUS_GITHUB_PROXY } else { '' })
# 传输镜像（只加速、不承担可信度）：每个字节都要与 Release API 的 digest 对账
$MirrorUrls = @('https://gh-proxy.com', 'https://cors.isteed.cc')

# ── Gitee 镜像仓（国内直连下载点）───────────────────────────────────────────
$GiteeOwner    = 'nuphus'
$GiteeRepo     = 'nuphus'
$GiteeApi      = "https://gitee.com/api/v5/repos/$GiteeOwner/$GiteeRepo"
$GiteeAssetUrl = "https://gitee.com/$GiteeOwner/$GiteeRepo/releases/download"
# latest.json 的国内入口。Gitee 没有 GitHub 那种「releases/latest/download/」形式
# （实测会 302 到 repository/archive/latest 再 404），资产下载 URL 又必然带版本号——
# 而旧客户端不可能知道新版本号。因此国内入口用 raw 文件：URL 稳定、每次发版由本脚本
# 刷新内容，旧客户端读到的永远是最新清单。
$GiteeManifestUrl = 'https://gitee.com/nuphus/nuphus/raw/main/latest.json'
# 仓库内该清单的路径（随发版提交，走 origin + gitee 双推）
$RepoManifest    = Join-Path $RepoRoot 'latest.json'

# 平台路由 + 该平台在 Gitee 上的资产名：单一事实来源，避免"传了什么"与
# "清单指向什么"两处各写一份而悄悄不一致。Asset 为 $null 表示该平台继续走
# GitHub 权威源（不占用 Gitee 容量）。
# 刻意只把 Windows 安装包放进 Gitee：
#   - 国内用户以 Windows 为绝对多数；
#   - updater 的 windows-x86_64 恰好就用这个文件，一份资产同时服务手动安装与自动更新；
#   - Gitee 仓库容量是共享配额，把全部平台（含 124MB AppImage）都放进去不可持续。
$PlatformRoute = [ordered]@{
    'windows-x86_64' = [ordered]@{ Source = 'gitee';  Asset = 'nuphus-win32-x64-{0}-setup.exe' }
    'darwin-aarch64' = [ordered]@{ Source = 'github'; Asset = $null }
    'linux-x86_64'   = [ordered]@{ Source = 'github'; Asset = $null }
}

# 实际要镜像的资产清单：从路由里派生，而不是另写一份
$GiteeAssets = @($PlatformRoute.Values | Where-Object { $_.Source -eq 'gitee' -and $_.Asset } | ForEach-Object { $_.Asset })

function Write-Step($m) { Write-Host "==> $m" -ForegroundColor Cyan }
function Write-Ok($m)   { Write-Host "    $m" -ForegroundColor Green }
function Write-WarnMsg($m) { Write-Host "    WARN: $m" -ForegroundColor Yellow }
function Write-Fail($m) { Write-Host "    FAIL: $m" -ForegroundColor Red }

function Resolve-Version {
    if ($Version) { return $Version }
    $conf = Join-Path $RepoRoot 'src-tauri\tauri.conf.json'
    if (-not (Test-Path $conf)) { throw "tauri.conf.json not found: $conf" }
    $v = (Get-Content $conf -Raw -Encoding UTF8 | ConvertFrom-Json).version
    if (-not $v) { throw 'cannot read version from src-tauri/tauri.conf.json' }
    return $v
}

# Gitee token 只在内存里；不落盘、不打印。失败时给出可操作的提示而非堆栈。
function Get-GiteeToken {
    $in = "protocol=https`nhost=gitee.com`n`n"
    $res = $in | git credential fill 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $res) { return $null }
    $line = @($res) | Where-Object { $_ -like 'password=*' } | Select-Object -First 1
    if (-not $line) { return $null }
    return ($line -replace '^password=', '')
}

# GitHub API token（私有仓必需；nuphus 是公开仓，匿名也能读 release 元数据）。
# 同样只在内存里，不落盘不打印。取不到就返回空串——公开仓下 Release API 匿名可用。
function Get-GitHubApiToken {
    $in = "protocol=https`nhost=github.com`n`n"
    $res = $in | git credential fill 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $res) { return '' }
    $line = @($res) | Where-Object { $_ -like 'password=*' } | Select-Object -First 1
    if (-not $line) { return '' }
    return ($line -replace '^password=', '')
}

# 统一的 Gitee POST：走 curl.exe 的 multipart。
# 为什么不用 Invoke-RestMethod -Form / .NET HttpClient：实测同一 token 下两者发出的
# multipart 都被 Gitee 判 401（"登录失效"），而 curl -F 同参数成功——差异在
# Content-Disposition 的 name 是否带引号。不是认证问题，是实现问题。
# token 只作为 curl 参数传入，本脚本任何日志都不回显 URL 或 token。
# 中文 body 走临时文件：Windows 命令行非 UTF-8，直传会乱码。
function Invoke-GiteeMultipart($url, $token, $fields, $filePath) {
    $bodyFile = $null
    try {
        $args = @('-s', '-w', "`n%{http_code}", '-X', 'POST', $url)
        foreach ($k in $fields.Keys) {
            $v = [string]$fields[$k]
            if ($v -match '[^\x20-\x7E]') {
                # 含非 ASCII（中文案）：落临时文件走 <file，避开命令行编码
                $bodyFile = Join-Path $env:TEMP "gitee-field-$([guid]::NewGuid().ToString('N')).txt"
                [System.IO.File]::WriteAllText($bodyFile, $v, (New-Object System.Text.UTF8Encoding($false)))
                $args += @('-F', "$k=<$bodyFile")
            } else {
                $args += @('-F', "$k=$v")
            }
        }
        if ($filePath) { $args += @('-F', "file=@$filePath") }
        $args += @('-F', "access_token=$token")
        $args += @('--max-time', '1800')

        $raw = & curl.exe @args 2>$null
        $lines = @($raw) -split "`r?`n"
        if ($lines.Count -lt 2) { throw "curl returned no parsable output: $($lines -join '|')" }
        $code = 0
        if (-not [int]::TryParse($lines[-1].Trim(), [ref]$code)) { throw "could not parse http code from curl output" }
        $body = ($lines[0..($lines.Count - 2)] -join "`n")
        if ($code -lt 200 -or $code -ge 300) {
            throw "HTTP $code from $(Split-Path $url -Leaf): $($body.Substring(0, [Math]::Min(200, $body.Length)))"
        }
        return $body
    } finally {
        if ($bodyFile -and (Test-Path $bodyFile)) { Remove-Item $bodyFile -Force -ErrorAction SilentlyContinue }
    }
}

# 权威 digest：GitHub Release API 的 assets[].digest。取不到即返回 $null，
# 调用方据此拒绝未校验的传输镜像。
function Get-ReleaseDigest($assetName, $ver) {
    try {
        $req = @{ Uri = "$ReleaseApi/releases/tags/v$ver"; UseBasicParsing = $true; TimeoutSec = 60 }
        if ($ApiProxy) { $req['Proxy'] = $ApiProxy }
        $rel = (Invoke-WebRequest @req).Content | ConvertFrom-Json
        $hit = @($rel.assets) | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
        if (-not $hit) { return $null }
        $dig = [string]$hit.digest
        if ($dig -match '^sha256:([0-9a-fA-F]{64})$') { return $Matches[1].ToLower() }
        return $null
    } catch { return $null }
}

# 下载资产：先落 .tmp，sha256 与权威 digest 对账通过才转正；不过即删。
# 与 publish.ps1 Get-Asset 同一套纪律，避免截断/污染文件被缓存复用。
function Get-Asset($assetName, $ver) {
    $versionDir = Join-Path $Downloads $ver
    $localFile  = Join-Path $versionDir $assetName
    if (Test-Path $localFile) {
        Write-Ok "cached: $assetName ($((Get-Item $localFile).Length) bytes)"
        return $localFile
    }
    if ($SkipDownload) { throw "$assetName not cached and -SkipDownload was set" }
    if ($DryRun) { Write-Ok "[dry-run] would download $assetName"; return $null }

    $expected = Get-ReleaseDigest $assetName $ver
    if (-not $expected) { throw "no authoritative digest for $assetName - refusing to mirror unverified bytes" }
    New-Item -ItemType Directory -Force -Path $versionDir | Out-Null

    $sources = @()
    foreach ($m in $MirrorUrls) { $sources += @{ Label = "mirror $m"; Uri = "$m/$ReleaseUrl/v$ver/$assetName" } }
    $sources += @{ Label = 'github'; Uri = "$ReleaseUrl/v$ver/$assetName" }

    foreach ($s in $sources) {
        $tmp = "$localFile.tmp"
        Remove-Item $tmp -Force -ErrorAction SilentlyContinue
        Write-Step "download $assetName <- $($s.Label)"
        try {
            $req = @{ Uri = $s.Uri; OutFile = $tmp; UseBasicParsing = $true; TimeoutSec = 1200 }
            # 镜像必须直连（叠本地代理会被限速）；只有裸 GitHub 域名才按需走代理
            if ($s.Label -eq 'github' -and $ApiProxy) { $req['Proxy'] = $ApiProxy }
            Invoke-WebRequest @req | Out-Null
            if (-not (Test-Path $tmp)) { throw 'download produced no file' }
            $actual = (Get-FileHash -Path $tmp -Algorithm SHA256).Hash.ToLower()
            if ($actual -ne $expected) { Remove-Item $tmp -Force; throw "sha256 mismatch from $($s.Label)" }
            Move-Item $tmp $localFile -Force
            Write-Ok "verified sha256 $actual"
            return $localFile
        } catch {
            Write-WarnMsg "$($s.Label) failed: $($_.Exception.Message)"
            Remove-Item $tmp -Force -ErrorAction SilentlyContinue
        }
    }
    throw "could not download $assetName from any source"
}

# 从 Gitee 下载回来再哈希一遍：上传成功不等于字节正确。
# 必须跟随重定向：Gitee 资产下载是 302 → attach_files/{id}/download/…（实测 2 跳），
# 不跟随只会拿到一个 161 字节的 HTML 跳转页，哈希自然对不上——那是校验方式的坑，
# 不是上传内容的问题。
function Test-GiteeAsset($release, $fileName, $expectedSha) {
    $url = "$GiteeAssetUrl/$($release.tag_name)/$fileName"
    $tmp = Join-Path $env:TEMP "gitee-verify-$([guid]::NewGuid().ToString('N')).bin"
    try {
        $null = & curl.exe -sL -o $tmp -w '%{http_code}' --max-time 600 $url 2>$null
        if (-not (Test-Path $tmp)) { return $false }
        if ((Get-Item $tmp).Length -le 0) { return $false }
        $actual = (Get-FileHash -Path $tmp -Algorithm SHA256).Hash.ToLower()
        return ($actual -eq $expectedSha)
    } catch {
        return $false
    } finally {
        Remove-Item $tmp -Force -ErrorAction SilentlyContinue
    }
}

# ── 主流程 ────────────────────────────────────────────────────────────────
$Ver = Resolve-Version
$Tag = "v$Ver"
$RelApiToken = Get-GitHubApiToken

Write-Step "sync Nuphus $Tag to Gitee (dry-run: $DryRun)"
$planned = @($GiteeAssets | ForEach-Object { $_ -f $Ver })
Write-Ok   "assets to mirror: $($planned -join ', ')"
$routeText = @($PlatformRoute.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value.Source)" }) -join '  '
Write-Ok   "platform route  : $routeText"

$Token = Get-GiteeToken
if (-not $Token) {
    throw "no Gitee credential found (git credential fill host=gitee.com). `n" +
          "Run 'git fetch' once against gitee so the credential is stored, or set it via Git Credential Manager."
}
if (-not $DryRun) {
    # 只确认 token 可用，不回显内容
    try {
        Invoke-RestMethod -Uri "$GiteeApi" -Headers @{ Authorization = "token $Token" } -TimeoutSec 45 | Out-Null
        Write-Ok 'Gitee token accepted'
    } catch {
        throw "Gitee API rejected the token: $($_.Exception.Message)"
    }
}

# Gitee 建 release 需要 tag 存在于镜像仓：本地有该 tag 就推上去（代码镜像本应有它）
if (-not $DryRun) {
    $hasTag = git rev-parse -q --verify "refs/tags/$Tag" 2>$null
    if ($LASTEXITCODE -eq 0) {
        # git 把「Everything up-to-date」等信息消息写在 stderr；PS 5.1 在全局
        # $ErrorActionPreference='Stop' 下会把 native command 的 stderr 行当
        # NativeCommandError 抛死整条流水线（2>&1 合并流也不免疫）。
        # 同 publish.ps1 的 npm 段修法：仅本次调用把 EAP 降为 Continue，
        # stderr 作为数据被捕获，成功判定不变。
        Write-Step "push tag $Tag to gitee"
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $pushOut = & git push gitee "refs/tags/$Tag" 2>&1
            foreach ($line in $pushOut) { Write-Ok $line }
        } finally {
            $ErrorActionPreference = $prevEap
        }
    } else {
        Write-WarnMsg "local tag $Tag missing - Gitee release creation may fail without it"
    }
}

# 下载（并校验）要镜像的资产
$staged = @{}
foreach ($tpl in $GiteeAssets) {
    $name = $tpl -f $Ver
    $path = Get-Asset $name $Ver
    if ($path) { $staged[$name] = $path }
}

# 取权威 latest.json：每个平台的 signature 必须与 GitHub 发布版逐字节相同，
# 否则 updater 验签失败——镜像换了传输通道，可信根（minisign 签名）没换。
if (-not $DryRun) {
    # 权威 latest.json 走 Release API 的 asset 端点，而不是 releases/download 域名：
    # 实测本机 api.github.com 可达，而 github.com 资产域名在 PowerShell 的
    # Invoke-WebRequest 下不稳定（同一 URL 命令行可通、脚本内 404）。签名只从这一份
    # 里取，因此取到后不信任内容本身——可信保证由客户端的 minisign 验签完成。
    Write-Step 'fetch authoritative latest.json (Release API asset endpoint)'
    $gotManifest = $false
    try {
        $relReq = @{ Uri = "$ReleaseApi/releases/tags/$Tag"; UseBasicParsing = $true; TimeoutSec = 60 }
        if ($ApiProxy) { $relReq['Proxy'] = $ApiProxy }
        $rel = (Invoke-WebRequest @relReq).Content | ConvertFrom-Json
        $asset = @($rel.assets) | Where-Object { $_.name -eq 'latest.json' } | Select-Object -First 1
        if (-not $asset) { throw 'release has no latest.json asset' }

        $tmp = "$RepoManifest.tmp"
        Remove-Item $tmp -Force -ErrorAction SilentlyContinue
        $aReq = @{
            Uri         = $asset.url
            OutFile     = $tmp
            UseBasicParsing = $true
            TimeoutSec  = 120
            Headers     = @{ Accept = 'application/octet-stream'; Authorization = "Bearer $RelApiToken" }
        }
        if ($ApiProxy) { $aReq['Proxy'] = $ApiProxy }
        Invoke-WebRequest @aReq | Out-Null
        if (-not (Test-Path $tmp) -or (Get-Item $tmp).Length -le 0) { throw 'asset download produced no file' }
        $probe = Get-Content $tmp -Raw | ConvertFrom-Json
        if (-not $probe.platforms.'windows-x86_64'.signature) { throw 'manifest missing windows signature' }
        $authority = $probe
        Remove-Item $tmp -Force
        $gotManifest = $true
        Write-Ok "authoritative latest.json v$($authority.version) fetched"
    } catch {
        Write-WarnMsg "Release API fetch failed: $($_.Exception.Message)"
        Write-WarnMsg "diag: ApiProxy='$ApiProxy' RelApiToken_len=$($RelApiToken.Length) tagsUri='$ReleaseApi/releases/tags/$Tag' assetUri='$($asset.url)'"
        $resp = $_.Exception.Response
        if ($resp) {
            try {
                $sr = New-Object System.IO.StreamReader($resp.GetResponseStream())
                Write-WarnMsg "diag: HTTP $([int]$resp.StatusCode) body=$($sr.ReadToEnd().Substring(0,200))"
            } catch { Write-WarnMsg "diag: could not read error body ($($_.Exception.Message))" }
        }
        Remove-Item "$RepoManifest.tmp" -Force -ErrorAction SilentlyContinue
    }
    if (-not $gotManifest) { throw 'could not fetch the authoritative latest.json from the Release API' }

    # 生成 Gitee 版清单：signature 原样保留（可信根没换），url 按路由改写。
    # signature 必须与 GitHub 发布版逐字节相同，否则 updater 验签失败。
    $platforms = [ordered]@{}
    foreach ($p in $authority.platforms.PSObject.Properties) {
        $route = $PlatformRoute[$p.Name]
        $src = if ($route) { [string]$route.Source } else { 'github' }
        $url = $p.Value.url
        if ($src -eq 'gitee' -and $route.Asset) {
            $url = "$GiteeAssetUrl/$Tag/$($route.Asset -f $Ver)"
        }
        $platforms[$p.Name] = [ordered]@{
            signature = $p.Value.signature
            url      = $url
        }
    }
    $giteeManifest = [ordered]@{
        version   = $authority.version
        notes     = $authority.notes
        pub_date  = $authority.pub_date
        platforms = $platforms
    }
    # 落仓库根：随发版提交并双推，raw URL 即国内入口（见 $GiteeManifestUrl）
    ($giteeManifest | ConvertTo-Json -Depth 6) | Set-Content -Path $RepoManifest -Encoding UTF8
    Write-Ok "repo latest.json refreshed -> $RepoManifest"
}

if ($DryRun) {
    Write-Step 'DRY RUN - nothing was downloaded, created or uploaded'
    Write-Ok   "would create/update Gitee release $Tag (tag already local: $(git rev-parse -q --verify "refs/tags/$Tag" 2>$null | Out-Null; $LASTEXITCODE -eq 0))"
    foreach ($n in $planned) { Write-Ok "  download + verify + upload $n" }
    Write-Ok   "  refresh+push repo latest.json -> $GiteeManifestUrl"
    exit 0
}

# 建（或取）Gitee release
$hdrs = @{ Authorization = "token $Token" }
$relUrl = "$GiteeApi/releases"
$release = $null
try {
    # 按 tag 精确查询，而不是列表翻页：列表可能分页、或漏掉刚建的 release，
    # 漏判会导致重复创建并被 Gitee 判「该标签已经存在发行版」而中断。
    $release = Invoke-RestMethod -Uri "$relUrl/tags/$Tag" -Headers $hdrs -TimeoutSec 60
} catch {
    $release = $null
}

$body = @"
Nuphus $Ver 桌面版（Gitee 国内下载点）

完整更新记录见仓库 CHANGELOG.md。
Windows 安装包亦可被内置更新器直接使用（签名与 GitHub 发布版一致）。

其他平台（macOS / Linux）请从 GitHub Releases 获取：
$ReleaseUrl/tag/$Tag
"@

if ($release -and $release.id) {
    Write-Ok "Gitee release $Tag already exists (id=$($release.id))"
} else {
    Write-Step "create Gitee release $Tag"
    # Gitee API v5 只认 form 表单参数（JSON body 会被判「tag_name is missing」），
    # 且 multipart 必须自建——见 Invoke-GiteeMultipart 的说明。
    $raw = Invoke-GiteeMultipart $relUrl $Token @{
        tag_name         = $Tag
        name             = $Tag
        body             = $body
        target_commitish = 'main'
    } $null
    $release = $raw | ConvertFrom-Json
    Write-Ok "created release id=$($release.id)"
}

# 上传资产（multipart attach_files）。同名资产先查重，已存在则跳过。
$haveNames = @(@($release.assets) | ForEach-Object { $_.name })
foreach ($name in $staged.Keys) {
    if ($haveNames -contains $name) {
        Write-Ok "already on Gitee: $name (skipping upload)"
    } else {
        Write-Step "upload $name to Gitee"
        # 同上：multipart 自建，token 只走内存 form 字段
        Invoke-GiteeMultipart "$relUrl/$($release.id)/attach_files" $Token @{} (Resolve-Path $staged[$name]).Path | Out-Null
        Write-Ok "uploaded $name"
    }
    # 无论上传还是跳过，都从 Gitee 拉回来核对 sha256
    $expected = Get-ReleaseDigest $name $Ver
    if (-not $expected) { throw "lost authoritative digest for $name - cannot verify" }
    if (Test-GiteeAsset $release $name $expected) {
        Write-Ok "verified from Gitee: $name"
    } else {
        throw "Gitee copy of $name does NOT match sha256 $expected - aborting"
    }
}

# 清单不进 Gitee release（那里没有稳定 URL），而是提交进仓库走 raw：
# 只 add 这一个文件，绝不把工作区里别人的改动顺手带进这次提交。
Write-Step 'commit latest.json and push'
git -C $RepoRoot add -- $RepoManifest
if ((git -C $RepoRoot status --porcelain -- $RepoManifest) -eq '') {
    Write-Ok 'latest.json unchanged - nothing to commit'
} else {
    # native git 的信息消息（Everything up-to-date / remote: Bypassed… / push 统计）
    # 走 stderr，在全局 EAP='Stop' 下会被当 NativeCommandError 抛死（v0.2.25 实测连炸两处：
    # push 已成功，报错却在成功后的信息行上）。同前修法：仅这段降 EAP 到 Continue，
    # 输出进日志，退出码判定不变——真失败（exit≠0）照旧 throw。
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        git -C $RepoRoot commit -m "chore(release): 刷新 latest.json 至 $Ver（Gitee 国内入口）" 2>&1 | ForEach-Object { Write-Ok $_ }
        if ($LASTEXITCODE -ne 0) { throw "latest.json commit failed (exit $LASTEXITCODE)" }
        git -C $RepoRoot push origin main 2>&1 | ForEach-Object { Write-Ok $_ }
        if ($LASTEXITCODE -ne 0) { throw "push origin main failed (exit $LASTEXITCODE)" }
        git -C $RepoRoot push gitee main 2>&1 | ForEach-Object { Write-Ok $_ }
        if ($LASTEXITCODE -ne 0) { throw "push gitee main failed (exit $LASTEXITCODE)" }
    } finally {
        $ErrorActionPreference = $prevEap
    }
    Write-Ok "domestic manifest: $GiteeManifestUrl"
}

# 容量台账：Gitee 仓库容量是共享配额，报出来让人看见增长
Write-Step 'Gitee release inventory'
try {
    # 用字符串拼接而非插值：插值写法在这里被 PowerShell 解析坏了 hostname
    $listUrl = $relUrl + '?per_page=100'
    $all = Invoke-RestMethod -Uri $listUrl -Headers $hdrs -TimeoutSec 60
    $total = 0
    foreach ($r in $all) {
        $sz = @(@($r.assets) | Measure-Object -Property size -Sum).Sum
        if (-not $sz) { $sz = 0 }
        $total += $sz
        Write-Ok ("{0,-10} assets={1,-3} size={2,8:N1} MB  {3}" -f $r.tag_name, @($r.assets).Count, ($sz / 1MB), $r.created_at)
    }
    Write-Ok ("total across {0} releases: {1:N1} MB" -f @($all).Count, ($total / 1MB))
    if (@($all).Count -gt $KeepReleases) {
        Write-WarnMsg "more than $KeepReleases releases retained - Gitee capacity is shared with the code mirror. Review manually (this script never deletes without -PruneReleases)."
    }
} catch {
    Write-WarnMsg "could not enumerate Gitee releases: $($_.Exception.Message)"
}

Write-Step "DONE - domestic users can now download from:"
Write-Ok   "https://gitee.com/$GiteeOwner/$GiteeRepo/releases"
