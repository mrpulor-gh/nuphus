# 前端 · 移动端 Web 应用（frontend/src/mobile/）

> 定位：Nuphus 桌面 Agent 的「手机共生端」——一个零依赖、可在 iOS/Android/微信 WebView 里跑的 PWA，通过 `mobile_server` 的 REST + WS 与桌面端共享同一条对话与同一个执行态。**纯客户端：没有任何业务权威**，执行态、会话存储、提炼锁、切换守卫全在后端。
> 规模：32 文件（28 源码 + 4 测试）约 1.4 万行（其中 mobile.css 4911 行）。

## 结构

| 分组 | 文件 | 职责 |
|---|---|---|
| 根与状态 | `App.tsx`（1314）/ `store.ts`（1093）/ `api.ts`（931）/ `ws.ts`（238） | 根组件（鉴权门 → 通道判定 → WS 生命周期 → 发送/遥控/通道切换全部编排）；纯函数状态机（chatReducer/applyEvent/乐观消息）；REST 契约层（30 端点封装、通道基址解析、超时与鉴权注入）；WS 客户端（就绪帧竞态防护、指数退避、僵尸看门狗、活性 ping） |
| 网络判定 | `connection.ts`（167）/ `auth.ts`（40） | 通道判定（lan/wan/null）、局域网探测、中继缓存与多租户归属解析；token 生命周期（URL → localStorage → 地址栏净化） |
| 适配层 | `i18n.ts`（26）/ `theme.ts`（41）/ `fontsize.ts`（40）/ `imagePath.ts`（119）/ `image-worker.ts`（61）/ `useWakeLock.ts`（62） | 复用桌面 zh/en 字典的纯函数 t()；深/浅主题；三档字号；识别文本中的本机图片绝对路径区间；Worker 解码+压缩图片（MAX_EDGE 1920 / JPEG 0.8）；执行中屏幕常亮 |
| 组件层 | `components/ChatScreen.tsx`（226）/ `NavBar.tsx`（1207）/ `Composer.tsx`（422）/ `MobileMarkdown.tsx`（830）/ `MessageList.tsx` / `MessageBubble.tsx`（340）/ `TraceModal.tsx` / `ConfirmCard.tsx` / `UserInputCard.tsx` / `RefineModal.tsx` / `RatingSheet.tsx` / `LocalImage.tsx`（216）/ `PairingGuide.tsx` / `AddToHomeScreen.tsx` / `WorkflowRunCard.tsx`（219）/ `MobileErrorBoundary.tsx` | 三层骨架（NavBar + MessageList + Composer + 悬浮卡片）；导航条+设置抽屉；底部输入栏；零依赖 Markdown（12 语言正则高亮 + 图片通道接入）；消息流增量窗口；气泡；执行时间线；危险操作确认卡；输入卡；提炼弹窗；点评；本机图片缩略图（懒加载+引用计数缓存+降级重试）；配对引导；A2HS；工作流执行胶囊+遥控 |

## 功能作用

用户在手机上打开 `mobile.html`（局域网 `http://<IP>:18772/mobile.html` 或中继公网入口），输入桌面端设置的配对密码换取 token，之后：

- **看**：拉取与桌面端完全一致的对话历史（GET /history），实时接收 NuphusEvent WS 流（流式文本/工具调用/执行开始结束/提炼提示等）；
- **说**：POST /message 发送消息（空闲=新执行，执行中=追加指令），支持携带最多 9 张压缩后的图片 data URL；
- **控**：危险操作确认卡、request_user_input 输入卡、终止执行、工作流暂停/继续/终止、遥控切换桌面端当前会话与运行模式/模型；
- **看电脑上的图**：Agent 回复里给出的电脑本地图片绝对路径（如 `E:\NUS\shot.png`），手机端无文件系统权限，通过 `GET /file?path=` 拉 Blob 渲染成缩略图。

## 规则与坑

- **三条网络通路是理解本模块的钥匙**（connection.ts:1-10）：`lan`（页面 hostname 属私有网段，局域网直连，免费低延迟）/ `wan`（公网入口页面，经中继服务器）/ `null`（判定中，此时 canChat=false，不建 WS）。
- 前端所有「猜测」都被显式设计成可收敛的乐观状态（乐观气泡、pending、sync_running 对账）——因为纯客户端没有业务权威。
