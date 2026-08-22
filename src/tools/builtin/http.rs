//! http — 通用 HTTP/API 请求工具（http_request）
//!
//! 与 web.rs 的定位区分：web_search/web_extract 面向「搜索与网页正文提取」，
//! http_request 面向「调用 API / 调试接口」，支持自定义 method/headers/body。
//!
//! SSRF 防护（check_ssrf）：
//! - localhost（127.0.0.0/8、::1）默认放行（本地开发服务）
//! - 恒拦截：0.0.0.0、169.254.0.0/16（link-local / 云元数据）
//! - 默认拦截私网段：10.0.0.0/8、172.16.0.0/12、192.168.0.0/16；
//!   allow_private=true 时仅放行这些私网段，169.254.0.0/16 仍拦截
//! - IPv4-mapped IPv6（如 ::ffff:192.168.1.1）归一化后按 v4 判定，防绕过
//! - 已知边界：SSRF 校验的 DNS 解析与 reqwest 连接时的解析相互独立（TOCTOU），
//!   重定向逐跳重新校验可收敛大部分风险
//!
//! 响应处理：
//! - application/json（含 +json）→ 直出（同为文本，走 max_bytes 截断）
//! - text/* 或无 Content-Type → UTF-8 截断到 max_bytes 并标注
//! - 其余（二进制）→ 落盘 %TEMP%/nuphus_http/，返回路径+大小+Content-Type，
//!   body 不进上下文（也不会被读成内存字符串）
//!
//! 输出视为不可信外部内容（security::injection::should_scan_tool 含
//! "http_request"），由调用方统一过 process_external_output 清洗标记。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;
use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

/// 参数默认值
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_BYTES: usize = 524288; // 512 KB
/// timeout_ms 上限：registry 对 web 类工具外层给 120s，留 10s 余量防外层误杀
const MAX_TIMEOUT_MS: u64 = 110_000;
/// 二进制落盘上限（防超大响应打爆磁盘）
const BINARY_CAP_BYTES: u64 = 32 * 1024 * 1024;
/// 二进制落盘目录名（位于系统临时目录下）
const BINARY_DIR_NAME: &str = "nuphus_http";
/// 手动跟随重定向上限（重定向逐跳重新做 SSRF 校验）
const MAX_REDIRECTS: u32 = 5;
/// http_request 工具 UA（与浏览器 UA 区分，避免 API 站点风控误判）
const TOOL_USER_AGENT: &str = "Nuphus-http_request/0.1";

/// http_request 专用 client 缓存（复用 web.rs 的 AGENT_TTL 缓存模式）。
/// 独立 client 的原因：需要 redirect=none，以便重定向逐跳重新做 SSRF 校验。
static HTTP_AGENT_CACHE: Mutex<Option<(reqwest::blocking::Client, i64)>> = Mutex::new(None);

fn get_http_agent() -> reqwest::blocking::Client {
    let now = super::web::unix_now();
    if let Ok(cache) = HTTP_AGENT_CACHE.lock() {
        if let Some((ref agent, ts)) = *cache {
            if now - ts < super::web::AGENT_TTL {
                return agent.clone();
            }
        }
    }
    let agent = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to build reqwest blocking Client");
    if let Ok(mut cache) = HTTP_AGENT_CACHE.lock() {
        *cache = Some((agent.clone(), now));
    }
    agent
}

// ── SSRF 防护 ──

/// 判定单个 IP 是否被 SSRF 策略拦截。localhost 恒放行。
fn ip_blocked(ip: &IpAddr, allow_private: bool) -> bool {
    // IPv4-mapped IPv6（::ffff:a.b.c.d）归一化为 v4 判定，防格式绕过
    let v4 = match ip {
        IpAddr::V4(v4) => Some(*v4),
        IpAddr::V6(v6) => v6.to_ipv4_mapped(),
    };
    if let Some(v4) = v4 {
        if v4.is_loopback() {
            return false; // 127.0.0.0/8 放行
        }
        if v4.is_unspecified() || v4.is_link_local() {
            return true; // 0.0.0.0 / 169.254.0.0/16（云元数据）恒拦截
        }
        if v4.is_private() {
            return !allow_private; // 10/8、172.16/12、192.168/16
        }
        return false;
    }
    // 纯 IPv6：::1 放行，未指定地址（::）恒拦截，其余放行
    match ip {
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return false;
            }
            v6.is_unspecified()
        }
        IpAddr::V4(_) => unreachable!(),
    }
}

/// SSRF 校验：解析 URL host → IP，按策略判定放行/拦截。
/// 域名的全部解析结果必须全部放行才放行（防混合应答绕过）。
fn check_ssrf(url: &str, allow_private: bool) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {}", e))?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("unsupported scheme '{}' (only http/https)", scheme)),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "URL missing host".to_string())?;
    // url crate 对 IPv6 字面量 host_str 返回含方括号形式（如 "[::1]"），
    // 直接 parse::<IpAddr> 会失败并落入 DNS 分支（Linux getaddrinfo 拒绝带括号）。
    // 统一 trim 括号再判定；域名无括号不受影响。
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = parsed.port_or_known_default().unwrap_or(80);

    // IP 字面量直接判定，无需 DNS
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip_blocked(&ip, allow_private) {
            return Err(format!(
                "SSRF blocked: {} is a restricted address (allow_private={})",
                ip, allow_private
            ));
        }
        return Ok(());
    }
    // 域名 → DNS 解析，逐条 A/AAAA 记录判定
    let addrs: Vec<IpAddr> = match (host, port).to_socket_addrs() {
        Ok(iter) => iter.map(|sa| sa.ip()).collect(),
        Err(e) => return Err(format!("DNS resolution failed for '{}': {}", host, e)),
    };
    if addrs.is_empty() {
        return Err(format!(
            "DNS resolution failed for '{}': no addresses",
            host
        ));
    }
    for ip in &addrs {
        if ip_blocked(ip, allow_private) {
            return Err(format!(
                "SSRF blocked: {} resolves to restricted address {} (allow_private={})",
                host, ip, allow_private
            ));
        }
    }
    Ok(())
}

// ── 请求执行 ──

struct HttpArgs {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    timeout_ms: u64,
    max_bytes: usize,
    use_cookies: bool,
    allow_private: bool,
}

/// 传输层错误分层：DNS / 连接 / 超时 / 其他。
/// 注意：reqwest 错误链不含响应 body，可安全入日志；响应原文永不进日志。
fn classify_reqwest_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        format!("HTTP request failed: timeout ({})", e)
    } else if e.is_connect() {
        format!("HTTP request failed: connection error ({})", e)
    } else {
        format!("HTTP request failed: {}", e)
    }
}

fn perform_http_request(args: &HttpArgs) -> Result<String, String> {
    check_ssrf(&args.url, args.allow_private)?;
    let agent = get_http_agent();
    let mut method = reqwest::Method::from_bytes(args.method.as_bytes())
        .map_err(|_| format!("unsupported HTTP method: {}", args.method))?;
    let mut url = args.url.clone();
    let mut redirects = 0;

    loop {
        let mut req = agent
            .request(method.clone(), &url)
            .timeout(Duration::from_millis(args.timeout_ms))
            .header("User-Agent", TOOL_USER_AGENT);
        for (k, v) in &args.headers {
            req = req.header(k, v);
        }
        // cookie 域白名单：use_cookies 且命中白名单域才从 vault 取 cookie
        //（返回值只进请求头，永不进日志 —— 与 web.rs 同约束）
        if args.use_cookies {
            if let Some(h) = super::web::cookie_header_for(&url) {
                req = req.header("Cookie", h);
            }
        }
        let has_body = args.body.is_some()
            && method != reqwest::Method::GET
            && method != reqwest::Method::HEAD;
        if has_body {
            let body = args.body.as_deref().unwrap_or("");
            // body 是合法 JSON 且用户未显式指定 Content-Type 时自动补 application/json
            let has_ct = args
                .headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
            if !has_ct && serde_json::from_str::<serde_json::Value>(body).is_ok() {
                req = req.header("Content-Type", "application/json");
            }
            req = req.body(body.to_string());
        }

        let resp = req.send().map_err(|e| classify_reqwest_error(&e))?;
        let status = resp.status();

        // 手动跟随重定向：每跳重新做 SSRF 校验（防 302 跳转私网绕过）
        if status.is_redirection() {
            if let Some(loc) = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            {
                if redirects >= MAX_REDIRECTS {
                    return Err(format!("too many redirects (>{})", MAX_REDIRECTS));
                }
                let next = reqwest::Url::parse(&url)
                    .and_then(|base| base.join(loc))
                    .map_err(|e| format!("invalid redirect Location '{}': {}", loc, e))?
                    .to_string();
                check_ssrf(&next, args.allow_private)?;
                tracing::info!(
                    "[http_request] redirect {} (status {})",
                    next,
                    status.as_u16()
                );
                // 303 语义：换 GET 并丢弃 body
                if status.as_u16() == 303 {
                    method = reqwest::Method::GET;
                }
                url = next;
                redirects += 1;
                continue;
            }
        }

        return render_response(resp, args.max_bytes);
    }
}

/// 响应渲染：json/text 直出（截断标注），二进制落盘（body 不进上下文）。
fn render_response(resp: reqwest::blocking::Response, max_bytes: usize) -> Result<String, String> {
    let status = resp.status();
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let ct_base = ct
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let head = format!(
        "HTTP {} (Content-Type: {})",
        status.as_u16(),
        if ct.is_empty() {
            "unknown"
        } else {
            ct.as_str()
        }
    );

    let is_text = ct_base == "application/json"
        || ct_base.ends_with("+json")
        || ct_base.starts_with("text/")
        || ct_base.is_empty();

    if is_text {
        // 文本：多读 1 字节探测截断，避免全量读入
        let mut limited = resp.take(max_bytes as u64 + 1);
        let mut buf = Vec::new();
        limited
            .read_to_end(&mut buf)
            .map_err(|e| format!("read response body failed: {}", e))?;
        let truncated = buf.len() > max_bytes;
        if truncated {
            buf.truncate(max_bytes);
        }
        // lossy 转换自动处理截断点落在 UTF-8 序列中间的情况
        let text = String::from_utf8_lossy(&buf);
        if truncated {
            Ok(format!(
                "{}\n\n{}...\n\n[Truncated: exceeded max_bytes {}, {} bytes returned]",
                head,
                text,
                max_bytes,
                text.len()
            ))
        } else {
            Ok(format!("{}\n\n{}", head, text))
        }
    } else {
        // 二进制：流式落盘（take 上限），body 不进上下文、不进日志
        let dir = std::env::temp_dir().join(BINARY_DIR_NAME);
        std::fs::create_dir_all(&dir).map_err(|e| format!("create temp dir failed: {}", e))?;
        let ext = match ct_base.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "application/pdf" => "pdf",
            "application/zip" => "zip",
            _ => "bin",
        };
        let path = dir.join(format!(
            "http_{}_{}.{}",
            super::web::unix_now(),
            std::process::id(),
            ext
        ));
        let mut file =
            std::fs::File::create(&path).map_err(|e| format!("create file failed: {}", e))?;
        let mut limited = resp.take(BINARY_CAP_BYTES);
        let size = std::io::copy(&mut limited, &mut file)
            .map_err(|e| format!("write file failed: {}", e))?;
        Ok(format!(
            "{}\n\nBinary response saved to: {} ({} bytes{})",
            head,
            path.display(),
            size,
            if size >= BINARY_CAP_BYTES {
                ", truncated at cap"
            } else {
                ""
            }
        ))
    }
}

// ── 工具注册 ──

impl ToolRegistry {
    pub(crate) fn register_http_request(&mut self) {
        self.register(ToolDef {
            name: "http_request".to_string(),
            description: "Make an HTTP/API request with custom method, headers and body. Use for calling REST APIs, webhooks, or debugging endpoints. JSON body auto-sets Content-Type: application/json. Text/JSON responses are returned inline (truncated at max_bytes); binary responses are saved to a temp file (only path+size returned, body not inlined). SSRF guard: localhost allowed; private ranges (10/8, 172.16/12, 192.168/16) and link-local/cloud-metadata (169.254/16) blocked unless allow_private=true (metadata stays blocked). Output is treated as untrusted external content.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "method": { "type": "string", "enum": ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"], "default": "GET", "description": "HTTP method" },
                    "url": { "type": "string", "description": "Full URL including http:// or https://" },
                    "headers": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Custom request headers (name -> value)" },
                    "body": { "type": "string", "description": "Request body. If it is valid JSON, Content-Type: application/json is set automatically (unless already provided in headers)" },
                    "timeout_ms": { "type": "integer", "minimum": 1000, "default": 30000, "description": "Request timeout in milliseconds (max 110000)" },
                    "max_bytes": { "type": "integer", "minimum": 100, "default": 524288, "description": "Max bytes of a text response returned inline; longer text is truncated with a marker" },
                    "use_cookies": { "type": "boolean", "default": false, "description": "Attach cookies from the cookie vault when the URL host hits the domain whitelist" },
                    "allow_private": { "type": "boolean", "default": false, "description": "Allow requests to private IP ranges (10/8, 172.16/12, 192.168/16). Link-local/cloud-metadata 169.254/16 stays blocked" }
                },
                "required": ["url"]
            }),
            category: ToolCategory::WebSearch,
            executor: |params, _ctx| {
                let url = params.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                if url.is_empty() {
                    return Ok(ToolResult::failure("url cannot be empty"));
                }
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Ok(ToolResult::failure("url must start with http:// or https://"));
                }
                let method = params
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("GET")
                    .to_ascii_uppercase();
                let headers: Vec<(String, String)> = params
                    .get("headers")
                    .and_then(|v| v.as_object())
                    .map(|m| {
                        m.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();
                let body = params
                    .get("body")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let timeout_ms = params
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_TIMEOUT_MS)
                    .clamp(1000, MAX_TIMEOUT_MS);
                let max_bytes = params
                    .get("max_bytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_MAX_BYTES as u64) as usize;
                let use_cookies = params
                    .get("use_cookies")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let allow_private = params
                    .get("allow_private")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let args = HttpArgs {
                    method,
                    url,
                    headers,
                    body,
                    timeout_ms,
                    max_bytes,
                    use_cookies,
                    allow_private,
                };
                super::run_blocking(move || match perform_http_request(&args) {
                    Ok(text) => Ok(ToolResult::success(text)),
                    Err(e) => Ok(ToolResult::failure(e)),
                })
            },
            depends_on: vec![],
        });
    }
}

// ── 单测（本地 127.0.0.1 TCP HTTP 服务器，参照 transport salvage_tests 模式） ──

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;

    /// 读取一个完整 HTTP 请求（请求行 + 头 + 按 Content-Length 读 body），
    /// 返回原始文本供断言。
    fn read_request(stream: &TcpStream) -> String {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut raw = String::new();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                break;
            }
            let lower = trimmed.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                content_length = v.trim().parse().unwrap_or(0);
            }
            raw.push_str(trimmed);
            raw.push('\n');
        }
        if content_length > 0 {
            let mut body = vec![0u8; content_length];
            let _ = reader.read_exact(&mut body);
            raw.push_str(&String::from_utf8_lossy(&body));
        }
        raw
    }

    /// 启动一次性本地 HTTP 服务器：接收一个请求，把原始请求文本送进 channel，
    /// 然后按 respond 返回的 (Content-Type, body) 回 200。
    fn spawn_server(
        respond: impl FnOnce(&str) -> (&'static str, Vec<u8>) + Send + 'static,
    ) -> (u16, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let raw = read_request(&stream);
                let _ = tx.send(raw.clone());
                let (ct, body) = respond(&raw);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    ct,
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        (port, rx)
    }

    fn args_for(url: String) -> HttpArgs {
        HttpArgs {
            method: "GET".to_string(),
            url,
            headers: vec![],
            body: None,
            timeout_ms: 10_000,
            max_bytes: DEFAULT_MAX_BYTES,
            use_cookies: false,
            allow_private: false,
        }
    }

    #[test]
    fn http_get_returns_status_and_body() {
        let (port, _rx) = spawn_server(|_| ("application/json", br#"{"ok":true}"#.to_vec()));
        let args = args_for(format!("http://127.0.0.1:{}/api", port));
        let out = perform_http_request(&args).expect("GET should succeed");
        assert!(
            out.starts_with("HTTP 200 (Content-Type: application/json)"),
            "got: {}",
            out
        );
        assert!(out.contains(r#"{"ok":true}"#));
    }

    #[test]
    fn http_post_json_body_sets_content_type() {
        let (port, rx) = spawn_server(|_| ("application/json", br#"{"received":true}"#.to_vec()));
        let mut args = args_for(format!("http://127.0.0.1:{}/submit", port));
        args.method = "POST".to_string();
        args.body = Some(r#"{"name":"nuphus"}"#.to_string());
        let out = perform_http_request(&args).expect("POST should succeed");
        assert!(out.contains(r#"{"received":true}"#));
        let raw = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("server should receive request");
        let raw_lower = raw.to_ascii_lowercase();
        assert!(raw.starts_with("POST /submit HTTP/1.1"), "got: {}", raw);
        assert!(
            raw_lower.contains("content-type: application/json"),
            "got: {}",
            raw
        );
        assert!(raw.contains(r#"{"name":"nuphus"}"#), "got: {}", raw);
    }

    #[test]
    fn http_custom_header_passthrough() {
        let (port, rx) = spawn_server(|_| ("text/plain", b"ok".to_vec()));
        let mut args = args_for(format!("http://127.0.0.1:{}/", port));
        args.headers = vec![
            ("X-Test-Marker".to_string(), "nuphus-123".to_string()),
            ("Authorization".to_string(), "Bearer tok".to_string()),
        ];
        let out = perform_http_request(&args).expect("GET should succeed");
        assert!(out.contains("ok"));
        let raw = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("server should receive request");
        let raw_lower = raw.to_ascii_lowercase();
        assert!(
            raw_lower.contains("x-test-marker: nuphus-123"),
            "got: {}",
            raw
        );
        assert!(
            raw_lower.contains("authorization: bearer tok"),
            "got: {}",
            raw
        );
    }

    #[test]
    fn http_text_truncated_at_max_bytes() {
        let (port, _rx) = spawn_server(|_| ("text/plain", "x".repeat(6000).into_bytes()));
        let mut args = args_for(format!("http://127.0.0.1:{}/", port));
        args.max_bytes = 1000;
        let out = perform_http_request(&args).expect("GET should succeed");
        assert!(
            out.contains("[Truncated: exceeded max_bytes 1000"),
            "got tail: {}",
            &out[out.len().saturating_sub(120)..]
        );
        // 截断后正文不超过 max_bytes
        assert!(
            out.len() < 1000 + 300,
            "output should be bounded, len={}",
            out.len()
        );
    }

    #[test]
    fn http_binary_saved_to_file() {
        let payload: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF, 0xFE];
        let (port, _rx) = spawn_server(move |_| ("application/octet-stream", payload.clone()));
        let args = args_for(format!("http://127.0.0.1:{}/file", port));
        let out = perform_http_request(&args).expect("GET should succeed");
        assert!(out.contains("Binary response saved to:"), "got: {}", out);
        assert!(out.contains("(7 bytes)"), "got: {}", out);
        // body 字节不进输出
        assert!(!out.contains('\u{89}'));
        let path_str = out
            .lines()
            .find_map(|l| l.strip_prefix("Binary response saved to: "))
            .and_then(|rest| rest.split(" (").next())
            .expect("path line should exist");
        let saved = std::fs::read(path_str).expect("saved file should exist");
        assert_eq!(saved, vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF, 0xFE]);
        let _ = std::fs::remove_file(path_str);
    }

    #[test]
    fn http_ssrf_blocks_private_and_metadata() {
        // 私网段默认拦截
        assert!(check_ssrf("http://192.168.1.1/", false).is_err());
        assert!(check_ssrf("http://10.0.0.1/", false).is_err());
        assert!(check_ssrf("http://172.16.0.1/", false).is_err());
        // 云元数据恒拦截（allow_private=true 也不放行）
        assert!(check_ssrf("http://169.254.169.254/latest/meta-data", false).is_err());
        assert!(check_ssrf("http://169.254.169.254/latest/meta-data", true).is_err());
        // 0.0.0.0 恒拦截
        assert!(check_ssrf("http://0.0.0.0/", false).is_err());
        assert!(check_ssrf("http://0.0.0.0/", true).is_err());
        // IPv4-mapped IPv6 不能绕过
        assert!(check_ssrf("http://[::ffff:192.168.1.1]/", false).is_err());
        // allow_private 放行私网段
        assert!(check_ssrf("http://192.168.1.1/", true).is_ok());
        assert!(check_ssrf("http://10.0.0.1/", true).is_ok());
        // 非 http/https scheme 拦截
        assert!(check_ssrf("file:///etc/passwd", false).is_err());
    }

    #[test]
    fn http_ssrf_allows_localhost() {
        assert!(check_ssrf("http://127.0.0.1:8080/", false).is_ok());
        assert!(check_ssrf("http://127.0.0.2/", false).is_ok());
        assert!(check_ssrf("http://[::1]:9000/", false).is_ok());
        assert!(check_ssrf("http://localhost/", false).is_ok());
    }

    #[test]
    fn http_executor_end_to_end_via_registry() {
        // 走 ToolRegistry 注册 + executor 闭包全链路（run_blocking 在无
        // tokio 上下文时 fallback 到线程，测试环境可用）
        let (port, _rx) = spawn_server(|_| ("application/json", br#"{"via":"registry"}"#.to_vec()));
        let mut registry = ToolRegistry::new();
        registry.register_http_request();
        let def = registry.get("http_request").expect("tool registered");
        let result = (def.executor)(
            &serde_json::json!({
                "url": format!("http://127.0.0.1:{}/", port),
                "method": "get" // 小写应被归一化
            }),
            &crate::tools::registry::ToolCtx::default(),
        )
        .expect("executor ok");
        assert!(result.success, "expected success, got: {:?}", result.error);
        let out = result.output.unwrap_or_default();
        assert!(out.contains(r#"{"via":"registry"}"#));
    }

    #[test]
    fn http_executor_rejects_bad_url() {
        let mut registry = ToolRegistry::new();
        registry.register_http_request();
        let def = registry.get("http_request").expect("tool registered");
        let result = (def.executor)(
            &serde_json::json!({"url": ""}),
            &crate::tools::registry::ToolCtx::default(),
        )
        .expect("executor ok");
        assert!(!result.success);
        let result = (def.executor)(
            &serde_json::json!({"url": "ftp://x"}),
            &crate::tools::registry::ToolCtx::default(),
        )
        .expect("executor ok");
        assert!(!result.success);
    }
}
