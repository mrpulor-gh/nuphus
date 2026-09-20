//! 网络请求诊断（第一级：只观测，不改连接行为）。
//!
//! 目标：用户再反馈「只走 IPv6 / 网络不通」时，报错文本与日志本身带证据，
//! 可立刻二分「DNS 只有 AAAA」「IPv6 半通（建连成功但无数据）」「代理/环境问题」。
//!
//! 两个产出：
//! 1. [`DiagResolver`]：挂在 `reqwest::ClientBuilder::dns_resolver` 上，记录**最近一次**
//!    解析事实（族分布 / 样例 IP / 解析耗时 / 解析错误）；
//! 2. [`format_diag`]：把解析事实 + 失败分类 + 错误链 + 本轮耗时与重试序号压成
//!    **单行有界摘要**（≤ 300 字符），既写 tracing，也附在返回给上层的错误文本尾部。
//!
//! # 关键不变量（勿改）
//! - 解析走 `std::net::ToSocketAddrs`（与 reqwest 内置 `GaiResolver` 同源 getaddrinfo：
//!   `spawn_blocking` + `(host, 0)`），**返回全部地址且保持系统返回顺序** ——
//!   hyper-util 用「首个地址的族」决定 happy-eyeballs 首选族（见其
//!   `SocketAddrs::split_by_preference`），顺序一变连接语义就变；
//! - 解析失败时记录错误文本后，把错误**原样**交给上层（不吞、不改写错误类型与文本）；
//! - 记录的是**候选地址集**，不是实际使用的地址：hyper-util 未公开导出 `ConnectError`
//!   的 `addr` 字段，失败时拿不到胜者地址 —— 禁止把候选当成事实陈述；
//! - 只记录 host 与 IP，**绝不**记录 api key / 请求头 / 请求体。

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// 与 `reqwest::Error` 内部 `BoxError` 同构（`Box<dyn Error + Send + Sync>`），
/// 即 [`Resolving`] 的错误类型。
type BoxedError = Box<dyn std::error::Error + Send + Sync>;

/// 单次 DNS 解析的事实记录（**候选地址集**，非实际使用地址）。
#[derive(Debug, Clone, Default)]
pub struct ResolutionRecord {
    /// 被解析的主机名
    pub host: String,
    /// 解析耗时（毫秒）
    pub elapsed_ms: u128,
    /// A 记录得到的 IPv4 地址（系统返回顺序）
    pub v4: Vec<IpAddr>,
    /// AAAA 记录得到的 IPv6 地址（系统返回顺序）
    pub v6: Vec<IpAddr>,
    /// 解析失败时的错误文本（成功为 `None`）
    pub error: Option<String>,
}

/// 记录最近一次 DNS 解析事实的 resolver（`reqwest::dns::Resolve` 实现）。
///
/// 每个请求轮次创建一个（[`DiagResolver::new_arc`]），客户端与该记录共享同一份状态：
/// 请求失败后由调用方 [`DiagResolver::last`] 取回，拼进错误文本与日志。
/// 一轮内若发生多次解析（重定向 / 代理），只保留最后一次。
#[derive(Debug)]
pub struct DiagResolver {
    last: Arc<Mutex<Option<ResolutionRecord>>>,
}

impl DiagResolver {
    /// 创建可挂到 `ClientBuilder::dns_resolver` 的共享 resolver。
    pub fn new_arc() -> Arc<DiagResolver> {
        Arc::new(DiagResolver {
            last: Arc::new(Mutex::new(None)),
        })
    }

    /// 最近一次解析记录（本轮尚未解析或尚无失败前为 `None`）。
    pub fn last(&self) -> Option<ResolutionRecord> {
        // 锁中毒不让请求受影响：诊断是旁路，取回内层已写入的值即可。
        match self.last.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Resolve for DiagResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let slot = Arc::clone(&self.last);

        let fut: Resolving = Box::pin(async move {
            let started = Instant::now();
            let lookup_host = host.clone();
            // 与内置 GaiResolver 同源：`spawn_blocking` + getaddrinfo。
            // 端口传 0（GaiResolver 同款），实际端口由 hyper-util 按 URL 覆写。
            let joined = tokio::task::spawn_blocking(move || {
                (lookup_host.as_str(), 0u16)
                    .to_socket_addrs()
                    .map(|iter| iter.collect::<Vec<SocketAddr>>())
            })
            .await;

            match joined {
                Ok(Ok(addrs)) => {
                    let (v4, v6) = split_families(&addrs);
                    store_latest(
                        &slot,
                        ResolutionRecord {
                            host,
                            elapsed_ms: started.elapsed().as_millis(),
                            v4,
                            v6,
                            error: None,
                        },
                    );
                    // **保持系统返回顺序**：hyper-util 以首个地址的族决定
                    // happy-eyeballs 首选族，重排等于改连接行为。
                    Ok(Box::new(addrs.into_iter()) as Addrs)
                }
                Ok(Err(e)) => {
                    let error = e.to_string();
                    store_latest(
                        &slot,
                        ResolutionRecord {
                            host,
                            elapsed_ms: started.elapsed().as_millis(),
                            v4: Vec::new(),
                            v6: Vec::new(),
                            error: Some(error),
                        },
                    );
                    // 原样把 I/O 错误交给上层：不吞、不改写
                    Err(Box::new(e) as BoxedError)
                }
                Err(join_err) => {
                    let err = join_error_to_io(join_err);
                    let error = err.to_string();
                    store_latest(
                        &slot,
                        ResolutionRecord {
                            host,
                            elapsed_ms: started.elapsed().as_millis(),
                            v4: Vec::new(),
                            v6: Vec::new(),
                            error: Some(error),
                        },
                    );
                    Err(Box::new(err) as BoxedError)
                }
            }
        });
        fut
    }
}

/// 写入最近一次解析记录（锁中毒时恢复内层数据继续写，绝不让观测影响请求结果）。
fn store_latest(slot: &Mutex<Option<ResolutionRecord>>, rec: ResolutionRecord) {
    match slot.lock() {
        Ok(mut guard) => *guard = Some(rec),
        Err(poisoned) => *poisoned.into_inner() = Some(rec),
    }
}

/// `spawn_blocking` 任务异常 → I/O 错误。
///
/// hyper-util 的 GaiResolver 对「任务被取消」映射为 `Interrupted`、对其余 `JoinError`
/// 直接 panic；诊断 resolver 不 panic（panic 会丢掉诊断且污染请求路径），
/// 取消仍按 `Interrupted` 对齐，其余统一映射为 I/O 错误向上返回。
fn join_error_to_io(join_err: tokio::task::JoinError) -> std::io::Error {
    if join_err.is_cancelled() {
        std::io::Error::new(std::io::ErrorKind::Interrupted, join_err)
    } else {
        std::io::Error::other(join_err)
    }
}

/// 按地址族分组，**组内保持系统返回顺序**（供诊断展示，不参与连接决策）。
fn split_families(addrs: &[SocketAddr]) -> (Vec<IpAddr>, Vec<IpAddr>) {
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for addr in addrs {
        match addr.ip() {
            ip @ IpAddr::V4(_) => v4.push(ip),
            ip @ IpAddr::V6(_) => v6.push(ip),
        }
    }
    (v4, v6)
}

/// reqwest 错误分类。
///
/// 连接层判据与 `chat_completions::transport::is_connection_error` 保持一致
/// （connect | timeout | request），其后才是 body / decode。
pub fn classify(e: &reqwest::Error) -> &'static str {
    if e.is_connect() {
        "connect"
    } else if e.is_timeout() {
        "timeout"
    } else if e.is_request() {
        "request"
    } else if e.is_body() {
        "body"
    } else if e.is_decode() {
        "decode"
    } else {
        "other"
    }
}

/// 逐级拼接错误链（`Display` 以 `": "` 连接），**最多 4 层、总长 ≤ 240 字符**。
///
/// 超长以 `...` 结尾；多行错误（如 TLS/网关回传的正文）压成单行，
/// 保证诊断行不破坏日志的行式解析。注意：链里不会出现「实际使用的地址」——
/// hyper-util 的 `ConnectError` 不公开 `addr`，其 `Display` 只有
/// `"tcp connect error"` / `"dns error"`，原因文本在 `source()` 里。
pub fn error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    const MAX_LEVELS: usize = 4;
    const MAX_CHARS: usize = 240;

    let mut parts: Vec<String> = Vec::new();
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = cur {
        if parts.len() == MAX_LEVELS {
            break;
        }
        let msg = single_line(&e.to_string());
        if !msg.is_empty() {
            parts.push(msg);
        }
        cur = e.source();
    }
    truncate_chars(&parts.join(": "), MAX_CHARS)
}

/// 从请求 URL 取主机名，取不到（非法 URL）时返回 `"unknown"`。
pub fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// 单行有界诊断摘要（**总长 ≤ 300 字符**）：
///
/// ```text
/// [net] host=api.example.com dns=12ms v6=1[2406:d440::1] v4=0 class=connect attempt=2/4 elapsed=8402ms chain="..." hint=仅解析到 IPv6（无 A 记录）
/// ```
///
/// `rec = None` 表示本轮没触发解析（错误发生在解析之前），渲染为 `dns=not-resolved`，
/// 此时不给 hint（无解析事实就不下结论）；记录的 host 与 `host` 不一致（代理路径）时渲染为
/// `dns=12ms@127.0.0.1`，避免把代理主机的族分布当成目标主机的。
/// `chain` 由 [`error_chain`] 生成，这里按剩余预算二次收敛。
///
/// 降级顺序（信息优先级：hint > 事实字段 > 链 > 样例 IP）：链放不下预算时先丢样例 IP
/// （样例与计数同源，链是唯一证据），仍放不下才按剩余预算截断链。
pub fn format_diag(
    host: &str,
    rec: Option<&ResolutionRecord>,
    class: &str,
    chain: &str,
    elapsed_ms: u128,
    attempt: usize,
    total: usize,
) -> String {
    const MAX_LINE: usize = 300;
    const MAX_CHAIN: usize = 240;
    const MAX_HOST: usize = 64;

    let dns = match rec {
        Some(r) => {
            let mut s = format!("{}ms", r.elapsed_ms);
            // 走代理时解析对象是代理主机（HTTP CONNECT 不在本地解析目标主机）：
            // 记录 host 与请求 URL host 不一致就显式标出，避免把代理的族分布当成目标的。
            if !r.host.is_empty() && !r.host.eq_ignore_ascii_case(host) {
                s.push('@');
                s.push_str(&truncate_chars(&r.host, MAX_HOST));
            }
            s
        }
        None => "not-resolved".to_string(),
    };
    let no_addrs: [IpAddr; 0] = [];
    let (v6, v4) = match rec {
        Some(r) => (r.v6.as_slice(), r.v4.as_slice()),
        None => (&no_addrs[..], &no_addrs[..]),
    };
    let hint = hint_for(rec)
        .map(|h| format!(" hint={h}"))
        .unwrap_or_default();
    let chain = truncate_chars(&single_line(chain), MAX_CHAIN);

    let head = |with_samples: bool| -> String {
        format!(
            "[net] host={} dns={} {} {} class={} attempt={}/{} elapsed={}ms",
            truncate_chars(host, MAX_HOST),
            dns,
            family_segment("v6", v6, with_samples),
            family_segment("v4", v4, with_samples),
            class,
            attempt,
            total,
            elapsed_ms,
        )
    };

    // 优先保留样例 IP；链必须被截断时改丢样例，把预算让给链。
    let with_samples = head(true);
    let full_chain = format!("{with_samples} chain=\"{chain}\"{hint}");
    if clen(&full_chain) <= MAX_LINE {
        return full_chain;
    }

    let without_samples = head(false);
    let fixed = clen(&without_samples) + clen(&hint) + clen(" chain=\"\"");
    let budget = MAX_LINE.saturating_sub(fixed).min(MAX_CHAIN);
    format!(
        "{without_samples} chain=\"{}\"{hint}",
        truncate_chars(&chain, budget)
    )
}

/// 单族 hint：只解析到一族的地址时给出可立即二分的结论。
fn hint_for(rec: Option<&ResolutionRecord>) -> Option<&'static str> {
    let rec = rec?;
    if rec.error.is_some() {
        return Some("DNS 解析失败");
    }
    match (rec.v4.is_empty(), rec.v6.is_empty()) {
        (true, false) => Some("仅解析到 IPv6（无 A 记录）"),
        (false, true) => Some("仅解析到 IPv4（无 AAAA 记录）"),
        // 双栈（正常）；两族皆空且无错误（异常状态）无证据，均不给 hint
        _ => None,
    }
}

/// 渲染一个地址族的计数（`with_samples` 时附最多 2 个样例 IP）。
fn family_segment(label: &str, addrs: &[IpAddr], with_samples: bool) -> String {
    const MAX_SAMPLES: usize = 2;

    let mut seg = format!("{label}={}", addrs.len());
    if with_samples && !addrs.is_empty() {
        let samples: Vec<String> = addrs
            .iter()
            .take(MAX_SAMPLES)
            .map(|ip| ip.to_string())
            .collect();
        seg.push('[');
        seg.push_str(&samples.join(","));
        seg.push(']');
    }
    seg
}

/// 字符数（诊断长度上限按字符计，hint 含中文）。
fn clen(s: &str) -> usize {
    s.chars().count()
}

/// 多行 / 回车压成空格，保证诊断行始终是单行。
fn single_line(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect()
}

/// 按字符截断，超长以 `...` 结尾（结果字符数 ≤ `max`）。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 3 {
        return s.chars().take(max).collect();
    }
    let mut out: String = s.chars().take(max - 3).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

    fn v4(n: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, n))
    }

    fn v6(n: u16) -> IpAddr {
        IpAddr::V6(Ipv6Addr::new(0x2406, 0xd440, 0, 0, 0, 0, 0, n))
    }

    fn rec(v4_addrs: Vec<IpAddr>, v6_addrs: Vec<IpAddr>, error: Option<&str>) -> ResolutionRecord {
        ResolutionRecord {
            host: "api.example.com".to_string(),
            elapsed_ms: 12,
            v4: v4_addrs,
            v6: v6_addrs,
            error: error.map(|e| e.to_string()),
        }
    }

    /// 测试用错误链节点（逐级 `source()`）。
    #[derive(Debug)]
    struct ChainErr {
        msg: String,
        deeper: Option<Box<ChainErr>>,
    }

    impl fmt::Display for ChainErr {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.msg)
        }
    }

    impl std::error::Error for ChainErr {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.deeper
                .as_ref()
                .map(|b| b.as_ref() as &(dyn std::error::Error + 'static))
        }
    }

    fn chain_of(msgs: &[&str]) -> ChainErr {
        let mut node: Option<Box<ChainErr>> = None;
        for msg in msgs.iter().rev() {
            node = Some(Box::new(ChainErr {
                msg: (*msg).to_string(),
                deeper: node,
            }));
        }
        *node.expect("至少一层")
    }

    #[test]
    fn test_split_families_groups_and_preserves_order() {
        let addrs = vec![
            SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::new(0x2406, 1, 0, 0, 0, 0, 0, 1), 0, 0, 0)),
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(1, 1, 1, 1), 0)),
            SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::new(0x2406, 1, 0, 0, 0, 0, 0, 2), 0, 0, 0)),
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(2, 2, 2, 2), 0)),
        ];
        let (v4_addrs, v6_addrs) = split_families(&addrs);
        assert_eq!(v4_addrs.len(), 2);
        assert_eq!(v6_addrs.len(), 2);
        assert_eq!(v4_addrs[0].to_string(), "1.1.1.1");
        assert_eq!(v4_addrs[1].to_string(), "2.2.2.2");
        assert_eq!(v6_addrs[0].to_string(), "2406:1::1");
        assert_eq!(v6_addrs[1].to_string(), "2406:1::2");
    }

    #[test]
    fn test_hint_rules() {
        assert_eq!(
            hint_for(Some(&rec(vec![], vec![v6(1)], None))),
            Some("仅解析到 IPv6（无 A 记录）")
        );
        assert_eq!(
            hint_for(Some(&rec(vec![v4(1)], vec![], None))),
            Some("仅解析到 IPv4（无 AAAA 记录）")
        );
        assert_eq!(hint_for(Some(&rec(vec![v4(1)], vec![v6(1)], None))), None);
        assert_eq!(
            hint_for(Some(&rec(vec![], vec![], Some("dns failure")))),
            Some("DNS 解析失败")
        );
        // 两族皆空且无错误：无事实可下结论
        assert_eq!(hint_for(Some(&rec(vec![], vec![], None))), None);
        // 本轮未触发解析
        assert_eq!(hint_for(None), None);
    }

    #[test]
    fn test_error_chain_limits_levels_and_compacts_to_single_line() {
        let chain = chain_of(&["error sending request", "tcp connect error", "os error 10061", "m4", "m5-cut"]);
        let s = error_chain(&chain);
        assert_eq!(s, "error sending request: tcp connect error: os error 10061: m4");

        let multiline = chain_of(&["first line\nsecond line"]);
        assert_eq!(error_chain(&multiline), "first line second line");
        assert!(!error_chain(&multiline).contains('\n'));
    }

    #[test]
    fn test_error_chain_truncates_to_240_chars() {
        let long = chain_of(&["y".repeat(400).as_str()]);
        let s = error_chain(&long);
        assert_eq!(s.chars().count(), 240);
        assert!(s.ends_with("..."));
    }

    #[test]
    fn test_format_diag_shape_single_family() {
        let r = rec(vec![], vec![IpAddr::V6(Ipv6Addr::new(0x2406, 0xd440, 0, 0, 0, 0, 0, 1))], None);
        let line = format_diag(
            "api.example.com",
            Some(&r),
            "connect",
            "error sending request for url (https://api.example.com/v1): tcp connect error",
            8402,
            2,
            4,
        );
        assert!(
            line.starts_with(
                "[net] host=api.example.com dns=12ms v6=1[2406:d440::1] v4=0 class=connect attempt=2/4 elapsed=8402ms"
            ),
            "line={line}"
        );
        assert!(
            line.contains("chain=\"error sending request for url (https://api.example.com/v1): tcp connect error\""),
            "line={line}"
        );
        assert!(line.ends_with("hint=仅解析到 IPv6（无 A 记录）"), "line={line}");
        assert!(!line.contains('\n'));
    }

    #[test]
    fn test_format_diag_dual_stack_has_no_hint() {
        let r = rec(vec![v4(1), v4(2), v4(3)], vec![v6(1)], None);
        let line = format_diag("api.example.com", Some(&r), "timeout", "timed out", 60_001, 1, 4);
        assert!(line.contains("v6=1["), "line={line}");
        // 最多打印 2 个样例
        assert!(line.contains("v4=3[10.0.0.1,10.0.0.2]"), "line={line}");
        assert!(!line.contains("hint="), "line={line}");
    }

    #[test]
    fn test_format_diag_dns_failure_and_no_resolution() {
        let failed = rec(vec![], vec![], Some("No such host is known. (os error 11001)"));
        let line = format_diag("api.example.com", Some(&failed), "connect", "dns error: No such host", 5, 1, 2);
        assert!(line.contains("dns=12ms v6=0 v4=0"), "line={line}");
        assert!(line.ends_with("hint=DNS 解析失败"), "line={line}");

        let none = format_diag("api.example.com", None, "request", "builder error", 1, 1, 1);
        assert!(none.contains("dns=not-resolved v6=0 v4=0"), "line={none}");
        assert!(!none.contains("hint="), "line={none}");
    }

    #[test]
    fn test_format_diag_respects_300_char_cap() {
        let long_host = "h".repeat(200);
        let long_chain = "x".repeat(500);

        // 双栈 + 超长 host + 超长链
        let dual = rec(vec![v4(1), v4(2)], vec![v6(1), v6(2)], None);
        let line = format_diag(&long_host, Some(&dual), "connect", &long_chain, 12_345_678, 4, 4);
        assert!(clen(&line) <= 300, "len={} line={line}", clen(&line));
        // 链让位后样例 IP 被丢弃，但计数保留
        assert!(line.contains("v6=2 v4=2"), "line={line}");
        assert!(line.contains("chain=\"xxx"), "line={line}");

        // 仅 IPv6 + 超长链：hint 必须保下来
        let only_v6 = rec(vec![], vec![v6(1), v6(2)], None);
        let line = format_diag(&long_host, Some(&only_v6), "connect", &long_chain, 999_999, 4, 4);
        assert!(clen(&line) <= 300, "len={} line={line}", clen(&line));
        assert!(line.ends_with("hint=仅解析到 IPv6（无 A 记录）"), "line={line}");
    }

    #[test]
    fn test_format_diag_truncates_overlong_chain_arg() {
        // 即使调用方传了超过 240 字符的链，输出仍守 300 上限
        let r = rec(vec![v4(1)], vec![], None);
        let line = format_diag("api.example.com", Some(&r), "body", &"z".repeat(1000), 1, 1, 1);
        assert!(clen(&line) <= 300, "len={} line={line}", clen(&line));
        assert!(line.ends_with("hint=仅解析到 IPv4（无 AAAA 记录）"), "line={line}");
    }

    #[test]
    fn test_format_diag_marks_proxy_host_mismatch() {
        // 走代理时解析对象是代理主机：诊断行必须标出来，不能冒充目标主机的族分布
        let mut r = rec(vec![v4(1)], vec![], None);
        r.host = "127.0.0.1".to_string();
        let line = format_diag("api.example.com", Some(&r), "connect", "x", 1, 1, 1);
        assert!(
            line.contains("host=api.example.com dns=12ms@127.0.0.1 "),
            "line={line}"
        );
        assert!(line.ends_with("hint=仅解析到 IPv4（无 AAAA 记录）"), "line={line}");

        // 大小写差异不算不一致（DNS 名大小写不敏感）
        let mut same = rec(vec![], vec![v6(1)], None);
        same.host = "API.Example.COM".to_string();
        let line = format_diag("api.example.com", Some(&same), "connect", "x", 1, 1, 1);
        assert!(line.contains("dns=12ms v6=1"), "line={line}");
    }

    #[test]
    fn test_host_of() {
        assert_eq!(host_of("https://api.example.com/v1/chat"), "api.example.com");
        assert_eq!(host_of("http://127.0.0.1:8000/v1"), "127.0.0.1");
        assert_eq!(host_of("not a url"), "unknown");
    }
}
