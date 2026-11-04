#Requires -Version 5.1
<#
.SYNOPSIS
    Patrols a Nuphus working copy for the junk and ignore mistakes that pile up
    between commits, and reports what a human (or an agent) has to deal with.

.DESCRIPTION
    Four checks, in the order an agent should run them before a "commit the
    local changes" style request:

      A. Git hygiene   - branch / HEAD / uncommitted changes. Informational:
                         a dirty tree is normal mid-task, never a finding.
      B. Session junk  - screenshots, BMP debug crops, scratch folders, stale
                         logs. Every rule mirrors a pattern in .gitignore, so
                         "ignored on purpose" and "nobody cleaned up" are told
                         apart. Only removed with -Clean.
      C. Ignore audit  - tracked files that an ignore rule swallows (a broken
                         rule) and ignored files that look like source (a rule
                         that may be too broad). Report only, never touched.
      D. Secret scan   - credential-shaped strings in uncommitted and untracked
                         text. Example / template / fixture paths are skipped.

    Exit code is a gate, not a mood:
      0 = no WARN finding (dirty tree and junk stay INFO, they are normal)
      1 = at least one WARN finding (ignore mistake, or a secret-shaped string)

.PARAMETER Path
    Repository root. Defaults to the parent folder of this script.

.PARAMETER Clean
    Delete the session junk reported by check B. Nothing else is ever removed:
    ignore-audit and secret findings are only reported.

.PARAMETER IncludeTempDir
    Also treat the contents of temp/ as junk. Off by default: that folder is a
    scratch pad where rollback artifacts (registry exports, lockfile backups)
    legitimately live, so a patrol must not sweep it silently.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\workspace-patrol.ps1

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\workspace-patrol.ps1 -Clean -IncludeTempDir

.NOTES
    Reference implementation for agents: run this first, then act on the
    report instead of re-deriving git status / temp scan / ignore audit by
    hand. Read-only unless -Clean is passed.
#>
param(
    [string]$Path = (Split-Path -Parent $PSScriptRoot),
    [switch]$Clean,
    [switch]$IncludeTempDir
)

$ErrorActionPreference = 'Stop'

# ────────────────────────────── 0. repo root ──────────────────────────────

$script:Root = (Resolve-Path -LiteralPath $Path).Path
if (-not (Test-Path -LiteralPath (Join-Path $script:Root '.git'))) {
    throw "Not a git repository: $Path"
}

function Invoke-Git {
    param([string[]]$GitArgs)
    $out = & git @GitArgs 2>&1
    if ($LASTEXITCODE -ne 0) { throw "git $($GitArgs -join ' ') failed: $out" }
    return $out
}

# Directories never walked. Build output, dependencies and runtime user data
# dwarf the source tree and hold nothing a human should clean by hand:
# workflows/ and npm-desktop/ are data dirs (workflow instances, extracted
# release packages), not source.
$script:SkipDirs = @('node_modules', 'target', '.git', 'dist', 'projects', 'downloads', 'apps', 'workflows', 'npm-desktop')

function Test-SkipDir {
    param([string]$Name)
    return ($script:SkipDirs -contains $Name) -or $Name -like 'target-*' -or $Name -like '.nuphus*'
}

function Get-ScannableFiles {
    # Depth-first walk with pruning. Get-ChildItem -Recurse cannot prune, and
    # walking node_modules costs minutes to learn nothing.
    $stack = New-Object System.Collections.Stack
    $stack.Push($script:Root)
    while ($stack.Count -gt 0) {
        $dir = Get-Item -LiteralPath $stack.Pop()
        foreach ($child in $dir.EnumerateDirectories()) {
            if (-not (Test-SkipDir $child.Name)) { $stack.Push($child.FullName) }
        }
        foreach ($file in $dir.EnumerateFiles()) { $file }
    }
}

function Format-Size {
    param([double]$Bytes)
    if ($Bytes -ge 1MB) { return ('{0:N1} MB' -f ($Bytes / 1MB)) }
    if ($Bytes -ge 1KB) { return ('{0:N0} KB' -f ($Bytes / 1KB)) }
    return ('{0} B' -f $Bytes)
}

function Write-Head {
    param([string]$Title)
    Write-Output ''
    Write-Output "[$Title]"
}

# ────────────────────────────── A. git hygiene ────────────────────────────

$branch = (Invoke-Git @('rev-parse', '--abbrev-ref', 'HEAD')) | Select-Object -First 1
$head = (Invoke-Git @('log', '-1', '--format=%h %s')) | Select-Object -First 1

$porcelain = Invoke-Git @('status', '--porcelain=v1', '-uall')
$staged = @($porcelain | Where-Object { $_ -match '^[MADRC]' })
$unstaged = @($porcelain | Where-Object { $_ -match '^.[MD]' })
$untracked = @($porcelain | Where-Object { $_ -match '^\?\?' })

Write-Output '=== Nuphus workspace patrol ==='
Write-Output ("repo   : {0}" -f $script:Root)
Write-Output ("branch : {0}" -f $branch)
Write-Output ("HEAD   : {0}" -f $head)
Write-Output ("dirty  : {0} staged, {1} unstaged, {2} untracked" -f $staged.Count, $unstaged.Count, $untracked.Count)

Write-Head 'A. git hygiene'
Write-Output 'uncommitted work is normal mid-task - listed for context only'
if ($porcelain.Count -gt 0) {
    $porcelain | ForEach-Object { Write-Output ('  ' + $_) }
}

# ────────────────────────────── B. session junk ───────────────────────────

$junk = New-Object System.Collections.Generic.List[object]
$files = @(Get-ScannableFiles)

# A tracked file is never junk - it is in the repository on purpose. Build the
# set once so the report and the -Clean path both refuse to touch it: a vendored
# third_party/*.orig looks exactly like a patch leftover, and deleting it shows
# up as an unstaged deletion of a tracked file.
$trackedPaths = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
foreach ($rel in (Invoke-Git @('ls-files'))) {
    if (-not [string]::IsNullOrWhiteSpace($rel)) {
        [void]$trackedPaths.Add((Join-Path $script:Root $rel))
    }
}
function Test-TrackedFile {
    param([string]$FullName)
    return $trackedPaths.Contains($FullName)
}

# (1) Loose media at the repository root: screenshots taken while driving the
#     UI. No product file ships a png/jpg/bmp at the root, so whatever sits
#     there is a leftover. Same reasoning as .gitignore's *.bmp rule.
foreach ($f in $files | Where-Object { $_.DirectoryName -eq $script:Root -and $_.Extension -in '.png', '.jpg', '.jpeg', '.bmp', '.gif' }) {
    $junk.Add([pscustomobject]@{
        Kind = 'root media'; File = $f; Rule = 'screenshot left in the repo root'
    })
}

# (2) debug_* anywhere in the source tree - .gitignore line 8. The OCR debug
#     writer (src/desktop/paddle_ocr.rs) uses the relative folder "debug_ocr",
#     so copies land next to whatever the process CWD happened to be.
foreach ($f in $files | Where-Object { $_.Name -like 'debug_*' }) {
    $junk.Add([pscustomobject]@{
        Kind = 'debug output'; File = $f; Rule = 'debug_* (gitignore:8)'
    })
}

# (3) Editor / patch droppings and stale logs. *.bak / *.tmp / *~ / *.swp are in
#     .gitignore (lines 25-28, 33, 35); *.orig is not, so a leftover patch file
#     also shows up as plain untracked - flag it anyway.
foreach ($f in $files | Where-Object { $_.Extension -in '.bmp', '.tmp', '.bak', '.orig', '.old' -or $_.Name -like '*~' -or $_.Name -like '*.log.err' }) {
    $junk.Add([pscustomobject]@{
        Kind = 'temp/backup'; File = $f; Rule = '*.bak / *.tmp / *~ (gitignore:25-35)'
    })
}
foreach ($f in $files | Where-Object { ($_.DirectoryName -eq $script:Root -and $_.Extension -eq '.log') -or ($_.DirectoryName -eq $script:Root -and $_.Name -eq 'tsc-out.txt') }) {
    $junk.Add([pscustomobject]@{
        Kind = 'stale log'; File = $f; Rule = '*.log / tsc-out.txt (gitignore:2,169)'
    })
}

# (4) frontend/.tmp - scratch written by frontend tooling (gitignore:15).
foreach ($f in $files | Where-Object { $_.FullName -like "$script:Root\frontend\.tmp\*" }) {
    $junk.Add([pscustomobject]@{
        Kind = 'frontend scratch'; File = $f; Rule = 'frontend/.tmp (gitignore:15)'
    })
}

# (5) temp/ contents - only when asked, see -IncludeTempDir.
if ($IncludeTempDir) {
    foreach ($f in $files | Where-Object { $_.FullName -like "$script:Root\temp\*" -and $_.Name -ne 'wps-registry-backup' }) {
        $junk.Add([pscustomobject]@{
            Kind = 'temp scratch'; File = $f; Rule = 'temp/ (gitignore:15)'
        })
    }
}

$junkAll = @($junk | Sort-Object { $_.File.Length } -Descending)
$junk = @($junkAll | Where-Object { -not (Test-TrackedFile $_.File.FullName) })
$trackedJunk = @($junkAll | Where-Object { Test-TrackedFile $_.File.FullName })
$junkBytes = 0
if ($junk.Count -gt 0) { $junkBytes = [double]($junk | ForEach-Object { $_.File.Length } | Measure-Object -Sum).Sum }

Write-Head 'B. session junk'
if ($trackedJunk.Count -gt 0) {
    Write-Output ('note  {0} tracked file(s) match a junk pattern - left alone, a tracked file is never junk:' -f $trackedJunk.Count)
    foreach ($j in $trackedJunk) {
        Write-Output ('      ' + $j.File.FullName.Substring($script:Root.Length + 1))
    }
}
if ($junk.Count -eq 0) {
    Write-Output 'none found'
}
else {
    Write-Output ('{0} file(s), {1} - matched against .gitignore patterns, deleted only with -Clean' -f $junk.Count, (Format-Size $junkBytes))
    foreach ($j in $junk) {
        $age = [int]((New-TimeSpan -Start $j.File.LastWriteTime).TotalDays)
        Write-Output ('  {0,-16} {1,-10} {2,4}d  {3}' -f $j.Kind, (Format-Size $j.File.Length), $age, $j.File.FullName.Substring($script:Root.Length + 1))
    }
    if ($Clean) {
        foreach ($j in $junk) {
            Remove-Item -LiteralPath $j.File.FullName -Force
            Write-Output ('  removed: ' + $j.File.FullName.Substring($script:Root.Length + 1))
        }
        # Empty debug_* folders are droppings too. Pruned walk: -Recurse would
        # drag node_modules and target/ into a directory listing for nothing.
        $dirStack = New-Object System.Collections.Stack
        $dirStack.Push($script:Root)
        while ($dirStack.Count -gt 0) {
            $d = Get-Item -LiteralPath $dirStack.Pop()
            foreach ($c in $d.EnumerateDirectories()) {
                if (-not (Test-SkipDir $c.Name)) { $dirStack.Push($c.FullName) }
            }
            if ($d.Name -like 'debug_*' -and $d.FullName -ne $script:Root) {
                $hasFile = $false
                foreach ($f in $d.EnumerateFiles()) { $hasFile = $true; break }
                if (-not $hasFile) {
                    Remove-Item -LiteralPath $d.FullName -Recurse -Force
                    Write-Output ('  removed dir: ' + $d.FullName.Substring($script:Root.Length + 1))
                }
            }
        }
    }
}

# ────────────────────────────── C. ignore audit ───────────────────────────

$warn = 0
$info = 0

# (1) A tracked file that an ignore rule swallows is always a rule mistake:
#     the rule either covers a path that must stay in git, or was written for a
#     directory layout that no longer exists.
$trackedButIgnored = @(Invoke-Git @('ls-files', '-i', '-c', '--exclude-standard'))

# (2) Ignored files that look like source may be intentional (the plugin market
#     tree, user data) or a rule that grew too wide. Report, never touch.
#     Dependency and build trees are pruned first: an ignored node_modules or
#     dist file proves nothing, it just drowns the signal.
$sourceExt = @('.rs', '.ts', '.tsx', '.mjs', '.js', '.py', '.toml', '.css')
$auditSkip = @('node_modules', 'dist', '.cache', 'target', '.git', 'downloads', 'apps', '.nuphus', 'temp', 'projects', 'npm-desktop', 'workflows', 'stt-proto')

function Test-AuditSkip {
    param([string]$RelativePath)
    foreach ($segment in ($RelativePath -split '[\\/]')) {
        if ($auditSkip -contains $segment) { return $true }
        if ($segment -like 'target-*' -or $segment -like '.nuphus*') { return $true }
        # UUID folder = a runtime-created workflow instance (.gitignore:126).
        # It is user data by definition; recursing it only finds noise.
        if ($segment -match '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$') { return $true }
    }
    return $false
}

# Ignored entries only, and WITHOUT -uall: -uall expands every ignored file
# individually (94k lines in this repo) while the collapsed form (~80 entries)
# is exactly what the audit needs.
$ignoredEntries = @(Invoke-Git @('status', '--ignored', '--porcelain=v1') |
    Where-Object { $_ -like '!! *' } |
    ForEach-Object { $_.Substring(3).Trim('"') } |
    Where-Object { -not (Test-AuditSkip $_) })

$ignoredSource = New-Object System.Collections.Generic.List[object]
foreach ($entry in $ignoredEntries) {
    $abs = Join-Path $script:Root $entry
    if (Test-Path -LiteralPath $abs -PathType Container) {
        $dirStack = New-Object System.Collections.Stack
        $dirStack.Push($abs)
        while ($dirStack.Count -gt 0) {
            $d = Get-Item -LiteralPath $dirStack.Pop()
            foreach ($c in $d.EnumerateDirectories()) {
                $rel = $c.FullName.Substring($script:Root.Length + 1)
                if (-not (Test-SkipDir $c.Name) -and -not (Test-AuditSkip $rel)) { $dirStack.Push($c.FullName) }
            }
            foreach ($f in $d.EnumerateFiles()) { if ($sourceExt -contains $f.Extension) { $ignoredSource.Add($f) } }
        }
    }
    elseif (Test-Path -LiteralPath $abs) {
        $item = Get-Item -LiteralPath $abs
        if ($sourceExt -contains $item.Extension) { $ignoredSource.Add($item) }
    }
}

Write-Head 'C. ignore audit'
if ($trackedButIgnored.Count -gt 0) {
    $warn++
    Write-Output ('WARN  {0} tracked file(s) swallowed by an ignore rule - the rule is wrong, not the file:' -f $trackedButIgnored.Count)
    $trackedButIgnored | ForEach-Object { Write-Output ('      ' + $_) }
}
else {
    Write-Output 'OK    no tracked file is swallowed by an ignore rule (git ls-files -i -c)'
}

if ($ignoredSource.Count -gt 0) {
    $info++
    Write-Output ('INFO  {0} ignored file(s) look like source - confirm each is an intentional exclusion:' -f $ignoredSource.Count)
    $shown = 0
    foreach ($f in ($ignoredSource | Sort-Object FullName)) {
        if ($shown -ge 20) {
            Write-Output ('      ... and {0} more' -f ($ignoredSource.Count - $shown))
            break
        }
        Write-Output ('      ' + $f.FullName.Substring($script:Root.Length + 1))
        $shown++
    }
}
else {
    Write-Output 'OK    no ignored file looks like source'
}

# ────────────────────────────── D. secret scan ────────────────────────────

# What is about to land in a commit: everything uncommitted plus untracked.
$changed = @(Invoke-Git @('diff', 'HEAD', '--name-only'))
$scanTargets = New-Object System.Collections.Generic.List[string]
foreach ($rel in $changed) { $scanTargets.Add((Join-Path $script:Root $rel)) }
foreach ($line in $untracked) { $scanTargets.Add((Join-Path $script:Root $line.Substring(3))) }

$skipPath = '(example|sample|template|fixture|\.test\.|\.spec\.|mock)'
$binaryExt = @('.png', '.jpg', '.jpeg', '.bmp', '.gif', '.ico', '.zip', '.exe', '.dll', '.onnx', '.pt', '.pdf', '.woff', '.woff2', '.ttf', '.mp4', '.wav', '.db', '.sig', '.tsbuildinfo')

$patterns = @(
    @{ Name = 'provider key';      Regex = 'sk-[A-Za-z0-9]{20,}' },
    @{ Name = 'bearer token';      Regex = 'Bearer\s+[A-Za-z0-9._\-]{20,}' },
    @{ Name = 'inline credential'; Regex = '(?i)(api[_-]?key|apikey|token|password|passwd|secret)\s*[:=]\s*["''][^"'']{12,}["'']' }
)

function Protect-Secret {
    param([string]$Text)
    if ($Text.Length -le 6) { return ('*' * $Text.Length) }
    $head = $Text.Substring(0, 2)
    $tail = $Text.Substring($Text.Length - 2)
    $stars = '*' * [Math]::Min(10, $Text.Length - 4)
    return ($head + $stars + $tail)
}

$secretHits = New-Object System.Collections.Generic.List[object]
foreach ($target in ($scanTargets | Select-Object -Unique)) {
    if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { continue }
    $item = Get-Item -LiteralPath $target
    if ($binaryExt -contains $item.Extension) { continue }
    if ($item.Length -gt 1MB) { continue }
    if ($item.FullName -match $skipPath) { continue }
    $n = 0
    foreach ($line in (Get-Content -LiteralPath $target)) {
        $n++
        foreach ($p in $patterns) {
            if ($line -match $p.Regex) {
                $masked = [regex]::Replace($line, $p.Regex, { param($m) Protect-Secret $m.Value })
                if ($masked.Length -gt 110) { $masked = $masked.Substring(0, 110) + '...' }
                $secretHits.Add([pscustomobject]@{
                    Pattern = $p.Name; Line = $n; File = $item.FullName; Text = $masked.Trim()
                })
            }
        }
    }
}

Write-Head 'D. secret scan'
if ($secretHits.Count -gt 0) {
    $warn++
    Write-Output ('WARN  {0} credential-shaped string(s) in uncommitted or untracked text:' -f $secretHits.Count)
    foreach ($hit in $secretHits) {
        Write-Output ('      {0}:{1} [{2}] {3}' -f $hit.File.Substring($script:Root.Length + 1), $hit.Line, $hit.Pattern, $hit.Text)
    }
}
else {
    Write-Output ('OK    no credential-shaped string in {0} changed/untracked file(s)' -f $scanTargets.Count)
}

# ────────────────────────────── summary ───────────────────────────────────

Write-Output ''
Write-Output '=== summary ==='
Write-Output ('dirty files : {0}  (staged {1} / unstaged {2} / untracked {3})' -f $porcelain.Count, $staged.Count, $unstaged.Count, $untracked.Count)
Write-Output ('junk        : {0} file(s), {1}{2}' -f $junk.Count, (Format-Size $junkBytes), $(if ($Clean -and $junk.Count -gt 0) { ' (deleted)' } else { '' }))
Write-Output ('ignore audit: {0} rule mistake(s), {1} to review' -f $trackedButIgnored.Count, $ignoredSource.Count)
Write-Output ('secrets     : {0} hit(s)' -f $secretHits.Count)
Write-Output ('result      : {0}' -f $(if ($warn -gt 0) { 'WARN (exit 1)' } else { 'PASS (exit 0)' }))

if ($warn -gt 0) { exit 1 }
exit 0
