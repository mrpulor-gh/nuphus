#Requires -Version 5.1
<#
.SYNOPSIS
    预下载 GitHub Release 资产到 npm-desktop/downloads/<ver>/ 并做 sha256 对账。

.DESCRIPTION
    存在理由：publish.ps1 / sync-gitee-release.ps1 的下载段走 Invoke-WebRequest，
    PS 5.1 下大资产（>40MB）会无征兆停滞在同一字节数，或 .tmp 涨到约 48MB 后
    删档归零循环（校验失败即删的设计叠加 IWR 停滞 = 死循环）。
    本脚本用 curl.exe（Start-Process 调用，镜像直连 / 仅 GitHub 源走本地代理），
    逐资产与 Release API 的 sha256 digest 对账，全部通过才算成功。
    成功后 publish.ps1 / sync-gitee-release.ps1 加 -SkipDownload 即可续跑。

.PARAMETER Version
    目标版本；缺省读 src-tauri/tauri.conf.json 的 version。

.PARAMETER Asset
    资产名（可多个），如 nuphus-win32-x64-0.2.25.zip。
    缺省下载该 Release 的全部资产（含 latest.json / .sig 等小文件）。

.EXAMPLE
    powershell -File npm-desktop/fetch-release-asset.ps1
.EXAMPLE
    powershell -File npm-desktop/fetch-release-asset.ps1 -Version 0.2.25 -Asset nuphus-win32-x64-0.2.25-setup.exe
#>
[CmdletBinding()]
param(
    [string]$Version = '',
    [string[]]$Asset = @()
)

$ErrorActionPreference = 'Stop'

function Write-Ok($m)   { Write-Host "    [ok] $m" -ForegroundColor Green }
function Write-Warn($m) { Write-Host "    [warn] $m" -ForegroundColor Yellow }

$RepoRoot  = Split-Path -Parent $PSScriptRoot
if (-not $Version) {
    $conf = Get-Content (Join-Path $RepoRoot 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
    $Version = $conf.version
}
$Downloads = Join-Path $PSScriptRoot ("downloads\{0}" -f $Version)
New-Item -ItemType Directory -Force -Path $Downloads | Out-Null

# token 只在内存：不落盘、不打印（git-pr-protocol SKILL §4 红线）
$fill = "host=github.com`nprotocol=https`n`n" | git credential fill
$tok = ($fill | Select-String '^password=').Line -replace '^password=',''
if (-not $tok) { throw 'no GitHub token from git credential fill' }
$h = @{ Authorization = "Bearer $tok"; Accept = 'application/vnd.github+json' }
$rel = Invoke-RestMethod -Proxy 'http://127.0.0.1:2081' -Uri "https://api.github.com/repos/mrpulor-gh/nuphus/releases/tags/v$Version" -Headers $h
Remove-Variable tok, fill -ErrorAction SilentlyContinue

$want = if ($Asset.Count -gt 0) { $Asset } else { @($rel.assets).ForEach({ $_.name }) }
$digests = @{}
foreach ($a in $rel.assets) {
    if ($want -contains $a.name -and $a.digest -and $a.digest.StartsWith('sha256:')) {
        $digests[$a.name] = $a.digest.Substring(7)
    }
}

$failed = @()
foreach ($name in $want) {
    $out = Join-Path $Downloads $name
    $expect = $digests[$name]
    if (-not $expect) { Write-Warn "no Release API digest for $name - refusing unverified download"; $failed += $name; continue }
    $ok = $false
    foreach ($src in @("https://gh-proxy.com/https://github.com/mrpulor-gh/nuphus/releases/download/v$Version/$name", "https://github.com/mrpulor-gh/nuphus/releases/download/v$Version/$name")) {
        $isMirror = $src -like '*gh-proxy*'
        # Start-Process 调 curl：stderr 不进 PowerShell error stream，天然免疫
        # ErrorActionPreference=Stop 下的 NativeCommandError（SKILL §7.7 第 8 条）
        $curlArgs = @('-sL', '--fail', '--max-time', '1200', '-o', $out)
        if (-not $isMirror) { $curlArgs += @('-k', '--http1.1', '-x', 'http://127.0.0.1:2081') }
        $curlArgs += $src
        $from = if ($isMirror) { 'mirror' } else { 'github+proxy' }
        $p = Start-Process -FilePath 'curl.exe' -ArgumentList $curlArgs -Wait -PassThru -NoNewWindow
        if ($p.ExitCode -eq 0 -and (Test-Path $out)) {
            $actual = (Get-FileHash -Path $out -Algorithm SHA256).Hash.ToLower()
            if ($actual -ne $expect) {
                Write-Warn "$name sha256 mismatch from $from - removing"
                Remove-Item $out -Force
                continue
            }
            $mb = [math]::Round((Get-Item $out).Length / 1MB, 1)
            Write-Ok "$name OK ($from, ${mb}MB, sha256 verified)"
            $ok = $true
            break
        }
        Write-Warn "$name curl failed from $from (exit $($p.ExitCode)) - trying next source"
    }
    if (-not $ok) { $failed += $name }
}

if ($failed.Count -gt 0) {
    throw ("failed: " + ($failed -join ', '))
}
Write-Ok "all $($want.Count) asset(s) cached under $Downloads - use -SkipDownload on publish.ps1 / sync-gitee-release.ps1"
