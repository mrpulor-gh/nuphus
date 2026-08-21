//! System proxy detection.
//!
//! Strategy: direct connection by default.
//! Only use proxy when user explicitly sets HTTPS_PROXY / HTTP_PROXY env vars.
//! No automatic Windows Registry detection — prevents stale VPN proxy residue.
//!
//! Users can configure NO_PROXY / no_proxy env var to bypass proxy for specific domains.
//! No hardcoded NO_PROXY — respects the user's network environment.

/// Detect proxy URL from environment variables only.
/// Returns None (direct connection) unless user explicitly configured a proxy.
pub fn detect_proxy_url() -> Option<String> {
    let proxy = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .or_else(|_| std::env::var("http_proxy"))
        .ok()?;

    let url = if proxy.contains("://") {
        proxy
    } else {
        format!("http://{}", proxy)
    };

    tracing::info!("[PROXY] 环境变量代理: {}", url);
    Some(url)
}

/// Returns the list of domains that should bypass the proxy.
/// Reads from NO_PROXY env var only. Always includes localhost/127.0.0.1.
pub fn get_no_proxy_domains() -> Vec<String> {
    let env_no_proxy = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();

    let mut domains: Vec<String> = env_no_proxy
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Always bypass localhost
    for &d in &["localhost", "127.0.0.1"] {
        if !domains.contains(&d.to_string()) {
            domains.push(d.to_string());
        }
    }

    domains
}

/// Check whether a host should bypass the proxy based on NO_PROXY rules.
pub fn should_bypass_proxy(host: &str) -> bool {
    let domains = get_no_proxy_domains();
    let host = host
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    domains
        .iter()
        .any(|d| host == d.as_str() || host.ends_with(&format!(".{}", d)))
}

/// Build a `reqwest::Proxy` with NO_PROXY support (reads env var only).
pub fn build_reqwest_proxy(proxy_url: &str) -> Option<reqwest::Proxy> {
    let mut proxy = reqwest::Proxy::all(proxy_url).ok()?;
    let no_proxy_str = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    if !no_proxy_str.is_empty() {
        if let Some(no_proxy) = reqwest::NoProxy::from_string(&no_proxy_str) {
            proxy = proxy.no_proxy(Some(no_proxy));
        }
    }
    Some(proxy)
}
