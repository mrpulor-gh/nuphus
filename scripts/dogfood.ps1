# dogfood.ps1 — Nuphus 双通道（dogfooding）可复现验证脚本
#
# 1. 构建 nuphus-mcp 二进制（独立仓库 release/nuphus-mcp，不在主 workspace）
# 2. 运行主 crate 的 E2E 测试：Nuphus 通过 MCP client 连接 nuphus-mcp，
#    调用 desktop_screen_size 返回真实屏幕分辨率
# 3. 运行 demo 展示独立 MCP client 接入
#
# 用法（PowerShell，workspace 根目录）：
#   .\scripts\dogfood.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# nuphus-mcp 已从主 workspace 拆出为独立仓库（release/nuphus-mcp）。
# 构建/运行需显式指定其 manifest，产物输出到独立仓库自己的 target/。
$mcpManifest = Join-Path $root "release\nuphus-mcp\Cargo.toml"
if (-not (Test-Path $mcpManifest)) {
    throw "nuphus-mcp manifest not found: $mcpManifest (先运行 scripts\package-nuphus-mcp.ps1 组装独立仓库)"
}

Write-Host "== [1/3] Build nuphus-mcp (standalone repo) ==" -ForegroundColor Cyan
cargo build --manifest-path $mcpManifest -p nuphus-mcp
if ($LASTEXITCODE -ne 0) { throw "cargo build nuphus-mcp failed" }

Write-Host "`n== [2/3] Dual-channel E2E (Nuphus -> nuphus-mcp via MCP client) ==" -ForegroundColor Cyan
cargo test -p nuphus --lib mcp::dual::tests::e2e_dogfood_screen_size -- --ignored --nocapture
if ($LASTEXITCODE -ne 0) { throw "dogfood E2E failed" }

Write-Host "`n== [3/3] Standalone demo (any MCP client style) ==" -ForegroundColor Cyan
cargo run --manifest-path $mcpManifest -p nuphus-mcp --example demo
if ($LASTEXITCODE -ne 0) { throw "demo failed" }

Write-Host "`n== dogfood OK: Nuphus MCP 双通道验证通过 ==" -ForegroundColor Green
