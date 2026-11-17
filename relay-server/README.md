# Nuphus Relay Server（中继服务）

Nuphus 中继服务（`relay-server`）：为手机端提供**异地访问桌面 Nuphus** 的公网通道。桌面端建立出站长连接接入中继，手机端经中继访问桌面——无需公网 IP、无需端口映射。

> 技术定位：中继**不落盘存储内容**，只做身份校验与转发路由。
>
> 中继是 Nuphus 专用协议通道（承载远程控制数据），**不是通用代理/VPN**。部署方式与用途由部署者自行决定。

---

## 一、架构

```
┌────────────┐  ① 设备 WS（/ws/device，令牌鉴权）  ┌──────────────┐
│  桌面 Nuphus│ ─────────────────────────────────→│              │
│ (relay_client)│                                   │  relay-server │
│  建立隧道   │  ② 隧道 WS（/ws/tunnel）            │  (中继中心)  │
│             │ ─────────────────────────────────→│              │
└────────────┘                                    └──────┬───────┘
                                                         │ ③ 公网隧道端口（默认 18081）
                                                    ┌────▼───────┐
                                                    │  手机 PWA   │
                                                    │ (r.example  │
                                                    │  .com)      │
                                                    └────────────┘
```

三条链路：

1. **设备通道**（`/ws/device`）：桌面端出站长连接，中继按 `device_id` 路由任务。
2. **隧道通道**（`/ws/tunnel`）：桌面端把公网字节流转发到本机 mobile_server（默认 18772）。
3. **公网隧道端口**（默认 `18081`）：手机访问该端口即等于访问桌面 mobile_server。

---

## 二、端口

| 端口 | 环境变量 | 默认 | 作用 |
|------|---------|------|------|
| 主服务 | `RELAY_PORT` | `18080` | `/health` `/task` `/ws/device` `/ws/tunnel` `/admin/rotate-caller-token` |
| 隧道 | `RELAY_TUNNEL_PORT` | `18081` | 手机外网访问隧道（公网入口） |

> `relay-server` 本体是**明文 http/ws**；生产环境 TLS 由反向代理（Nginx / Caddy）终结，对外暴露 443。

---

## 三、配置引导

> **配置原则（务必先读）**：手机异地访问**必须先配置完整的中继线路**，这是能否连上中继的唯一前提。
> 局域网直连地址只是**同一 WiFi 下的兜底**，**不能替代中继配置**——如果只填了局域网地址，
> 手机离开 WiFi 后就永远连不上中继。请按顺序完整配置 3.1 → 3.4，再进入自检。

### 3.1 服务端 token（必配，缺任一拒绝启动）

token 解析优先级：**数据目录文件 > 环境变量**。

- 数据目录文件：`{RELAY_DATA_DIR}/relay_device.token`、`{RELAY_DATA_DIR}/relay_caller.token`
- 环境变量：`RELAY_DEVICE_TOKEN`、`RELAY_CALLER_TOKEN`

| 变量 | 作用 |
|------|------|
| `RELAY_DEVICE_TOKEN` / `relay_device.token` | 桌面端设备 WS 鉴权 |
| `RELAY_CALLER_TOKEN` / `relay_caller.token` | 外部调用方 POST /task 鉴权；经中继下发手机端外网发送用 |
| `RELAY_DATA_DIR` | token 文件目录（可选，缺省当前目录） |

```bash
# 示例：以数据目录方式配置（内容为纯 token，无换行）
export RELAY_DATA_DIR=/etc/nuphus-relay
echo "$RELAY_DEVICE_TOKEN_VALUE" > ${RELAY_DATA_DIR}/relay_device.token
echo "$RELAY_CALLER_TOKEN_VALUE" > ${RELAY_DATA_DIR}/relay_caller.token
```

### 3.2 依赖环境变量（可选，均有默认值）

| 变量 | 默认 | 说明 |
|------|------|------|
| `RELAY_PORT` | `18080` | 主服务监听端口 |
| `RELAY_TUNNEL_PORT` | `18081` | 公网隧道监听端口 |
| `RUST_LOG` | `relay_server=info` | 日志级别 |

### 3.3 桌面端配置（`relay_client.json`）

位置：`config_dir/nuphus/relay_client.json`（如 Windows `%APPDATA%\nuphus\relay_client.json`）

```json
{
  "enabled": true,
  "url": "wss://<你的中继域名>",
  "device_id": "<你的设备唯一标识>",
  "token": "<与服务端 RELAY_DEVICE_TOKEN 一致>",
  "caller_token": "<与服务端 RELAY_CALLER_TOKEN 一致>",
  "public_url": "https://<你的隧道域名>"
}
```

> ⚠️ **不要填 `relay.nuphus.com` / `r.nuphus.com`**。当前版本桌面端把这两个官方域名列为拒绝名单
> （`src-tauri/src/relay_client.rs` 的 `is_official_relay_url` → `retire_official_config`）：一旦命中，
> 启动时会清空 `enabled` / `url` / `public_url` / `token` / `caller_token`，表现为
> **「填了地址却一直连不上、配置看起来被重置」**。一律使用你自己的域名。

| 字段 | 必填 | 说明 |
|------|------|------|
| `enabled` | 是 | 中继开关 |
| `url` | 是 | 中继地址（`wss://<域名>` 或 `ws://<域名>:18080`） |
| `device_id` | 是 | 设备标识（中继按此路由，请唯一） |
| `token` | 是 | 设备鉴权（= `RELAY_DEVICE_TOKEN`） |
| `caller_token` | 是 | 调用方鉴权（= `RELAY_CALLER_TOKEN`，经 /relay-hint 下发手机） |
| `public_url` | 否 | 隧道公网入口；为空时自动派生（`ws://host:18080 → http://host:18081`，`wss:// → https://`） |

### 3.4 手机端服务配置（`mobile_server.json`）

位置：`config_dir/nuphus/mobile_server.json`

```json
{
  "enabled": true,
  "port": 18772,
  "token": "<配对后生成的访问令牌>",
  "password_hash": "<salt>:<sha256>"
}
```

---

## 四、部署

```bash
cd relay-server
cargo build --release
# 以数据目录方式启动（token 文件已配置）
RELAY_PORT=18080 RELAY_TUNNEL_PORT=18081 RELAY_DATA_DIR=/etc/nuphus-relay ./target/release/relay-server
```

**TLS 终结**（生产必做）：`relay-server` 为 http/ws，需在反向代理（Nginx / Caddy）终结 TLS 并转发到 `18080` / `18081`。两个端口分工不同，**分流写错会直接导致手机端白屏**：

| 端口 | 承载路径 | 用途 |
|------|---------|------|
| `18080` | `/health` `/task` `/admin/*` `/ws/device` `/ws/tunnel` | 健康检查、任务投递、管理接口、两条 WS 通道 |
| `18081` | 其余全部 | 手机端页面与其静态资源（由桌面的 `mobile_server` 提供） |

**方案 A：单域名按路径分流（推荐，只需一个证书）**

```caddyfile
<你的域名> {
	# 主服务
	@main path /health /task /admin/* /ws/device /ws/tunnel
	handle @main {
		reverse_proxy 127.0.0.1:18080
	}

	# 其余全部交给隧道口：手机端 HTML 与静态资源
	handle {
		reverse_proxy 127.0.0.1:18081
	}

	# Safari 会优先尝试 HTTP/3（QUIC）；跨境 UDP 丢包时会长时间挂起而不回退 TCP，
	# 表现为一直白屏/转圈。关掉 Alt-Svc 通告，强制走 TCP。
	header -Alt-Svc
}
```

**方案 B：双域名各占一个端口**

- `wss://relay.<你的域名>` → 代理到 `127.0.0.1:18080`
- `https://r.<你的域名>` → 代理到 `127.0.0.1:18081`

> 用方案 B 时 `public_url` 填 `https://r.<你的域名>`；用方案 A 时 `url` 与 `public_url` 可以同域，隧道口由路径分流兜住。
> 无论哪种方案，代理都必须透传 WebSocket 升级（Caddy 默认透传；Nginx 需显式写 `proxy_set_header Upgrade` / `Connection` 并 `proxy_http_version 1.1`）。

---

## 五、自检

### 5.1 服务端自检

```bash
# 健康检查：应返回 {"ok":true}
curl https://<你的域名>/health

# 确认监听端口
ss -tlnp | grep -E '18080|18081'
```

### 5.2 桌面端自检

1. `relay_client.json` 五项必填字段完整。
2. 桌面端「手机访问」设置页中继开关打开，状态应显示**已连接**（非 `retrying`）。

### 5.3 手机端自检

**入口地址**用桌面端「设置 → 移动端 → 自定义服务端 → 复制远程链接」给出的那一条，不要手抄局域网
`http://192.168.x.x:18772`——那是同一 WiFi 下的兜底直连，离开 WiFi 即不可达，与中继入口不是同一个东西。

按顺序验证，哪一步不通就停在那一步排查：

| # | 检查 | 期望 |
|---|------|------|
| 1 | `curl https://<你的域名>/health` | `{"ok":true}` |
| 2 | `curl -H "X-Mobile-Token: <手机令牌>" https://<你的域名>/identity` | 200 + 身份显示名 JSON（令牌错误则为 401） |
| 3 | 桌面端 `/relay-hint` | `enabled: true`（仅当桌面端中继配置完整且已开启） |
| 4 | 打开入口地址，看 HTML 源码 | 资源路径已被隧道改写成 `/d/<device_id>/…` |
| 5 | 静态资源响应头 | 200，`Content-Type: text/javascript; charset=utf-8` |
| 6 | 中继服务端日志 | 出现 `device online`、`tunnel link online`（101 升级） |

> 第 4、5 步失败（页面白屏、资源 404）先回第四节核对分流规则：手机端静态资源由 `18081` 提供，
> 若整个域名只转到了 `18080`，HTML 能出来但 JS 全 404，表现就是白屏。
> 第 6 步失败说明桌面端没连上中继，查 `relay_client.json` 与桌面端中继状态，不在手机端。

默认走中继；同一 WiFi 下可经 `lan_url` 直连（仅作同网兜底，**不能替代中继配置**）。

### 5.4 端到端验证

```
手机 PWA 打开 → 配对（密码）→ 进入会话 → 发送一条消息 → 桌面端可见并可回复
```

---

## 六、安全说明

- **令牌不进 URL**：设备 WS 鉴权走三通道——`Authorization: Bearer` / `Sec-WebSocket-Protocol: auth.<token>` / query `?token=`，优先 Header，避免令牌落日志/代理/截图。
- **令牌隔离**：`RELAY_DEVICE_TOKEN`（设备通道凭据）永不下发手机端；`RELAY_CALLER_TOKEN` 可经中继下发手机端外网发送。
- **防裸奔**：任一 token 未配置则拒绝启动。
- **速率限制**：HTTP 按 IP 限速；隧道按 IP / 全局并发上限。
- **反向代理日志会泄露手机令牌**：手机端用 `X-Mobile-Token` 头鉴权，Nginx / Caddy 默认访问日志会把它
  原文写进文件（实测 Caddy 2.6 的 `log` 无法按字段剔除单个请求头）。需排查时只记路径与状态码，
  或者干脆对该站点关闭访问日志——日志一旦可读，等于拿到手机端凭据。

---

## 七、常见问题

| 现象 | 排查 |
|------|------|
| 服务端拒绝启动 | `RELAY_DEVICE_TOKEN` / `RELAY_CALLER_TOKEN` 未配置（token 文件或环境变量） |
| 手机能访问但发不了消息 | `relay_client.json` `caller_token` 与服务端不一致，或桌面中继未启用 |
| 局域网正常、异地不通 | 检查桌面端中继是否 `connected`、`public_url` 是否可达 |
| `/health` 不通 | 反向代理未转发 443 → `18080`，或 `relay-server` 未运行 |
| 手机打开白屏 / 一直转圈 | ① 确认用的是「复制远程链接」的地址而非局域网 `192.168.x.x`；② 回第四节核对分流，静态资源必须走 `18081`；③ `curl https://<你的域名>/health` 是否 200 |
| 页面能开、JS 全 404 | 分流缺失：域名只转到了 `18080`，而 HTML 里的 `/d/<device_id>/…` 由 `18081` 提供 |
| Safari 白屏、微信端正常 | 先排除地址差异（两次打开的是否同一入口）；再在反代关掉 HTTP/3 通告（`header -Alt-Svc`）强制 TCP 后复测——Safari 会优先尝试 QUIC，跨境 UDP 丢包时容易挂起而不回退 |
| 填了地址但配置像被重置 | `url` / `public_url` 用了 `relay.nuphus.com` 或 `r.nuphus.com`，被客户端清退（见 3.3 警告） |
| 日志里出现 `X-Mobile-Token` 原文 | 关闭该站点的访问日志（见第六节） |

---

## 八、仓库结构

```
relay-server/
├── Cargo.toml
├── src/
│   └── main.rs      # 路由 / 鉴权 / 隧道 / 限速 / 健康检查
└── README.md        # 本说明

注：vps_ssh.py、_deploy*.py 为本地运维/部署脚本，含基础设施信息，
已移出仓库目录（仅保留在本地 Nuphus-local-tools/relay-deploy-scripts/），
不随仓库分发。
```