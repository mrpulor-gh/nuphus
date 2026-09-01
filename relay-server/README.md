# Nuphus Relay Server（中继服务）

Nuphus 中继服务（`relay-server`）：为手机端提供**异地访问桌面 Nuphus** 的公网通道。桌面端建立出站长连接接入中继，手机端经中继访问桌面——无需公网 IP、无需端口映射。

> 安全定位：中继**不落盘存储内容**，只做身份校验与转发路由。
>
> 使用边界：中继为 Nuphus 专用协议通道（远程控制数据），**不是通用代理/VPN**。允许/禁止用途见根目录 [`relay-usage-policy.md`](../relay-usage-policy.md)。

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
  "url": "wss://relay.nuphus.com",
  "device_id": "<你的设备唯一标识>",
  "token": "<与服务端 RELAY_DEVICE_TOKEN 一致>",
  "caller_token": "<与服务端 RELAY_CALLER_TOKEN 一致>",
  "public_url": "https://r.nuphus.com"
}
```

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

**TLS 终结**（生产必做）：`relay-server` 为 http/ws，需在反向代理（Nginx / Caddy）终结 TLS 并转发到 `18080` / `18081`：

- `wss://relay.nuphus.com` → 代理到 `127.0.0.1:18080`
- `https://r.nuphus.com` → 代理到 `127.0.0.1:18081`

---

## 五、自检

### 5.1 服务端自检

```bash
# 健康检查：应返回 {"ok":true}
curl https://relay.nuphus.com/health

# 确认监听端口
ss -tlnp | grep -E '18080|18081'
```

### 5.2 桌面端自检

1. `relay_client.json` 五项必填字段完整。
2. 桌面端「手机访问」设置页中继开关打开，状态应显示**已连接**（非 `retrying`）。

### 5.3 手机端自检

1. 手机访问隧道公网入口（如 `https://r.nuphus.com`）——**这是中继线路的关键**，必须先可达。
2. `/relay-hint` 返回 `enabled: true`（仅在桌面端中继配置完整且开启时）。
3. 默认走中继；同一 WiFi 下可经 `lan_url` 直连（仅作同网兜底，**不能替代中继配置**）。

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

---

## 七、常见问题

| 现象 | 排查 |
|------|------|
| 服务端拒绝启动 | `RELAY_DEVICE_TOKEN` / `RELAY_CALLER_TOKEN` 未配置（token 文件或环境变量） |
| 手机能访问但发不了消息 | `relay_client.json` `caller_token` 与服务端不一致，或桌面中继未启用 |
| 局域网正常、异地不通 | 检查桌面端中继是否 `connected`、`public_url` 是否可达 |
| `/health` 不通 | 反向代理未转发 443 → `18080`，或 `relay-server` 未运行 |

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