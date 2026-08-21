//! web — Web 搜索与抓取工具
//!
//! - `web::search` — 多源降级搜索 (Bing → DDG Lite)
//! - `web::extract` — 从 URL 抓取并清洗为纯文本，支持 CDP 浏览器渲染兜底
//!
//! 搜索源降级链:
//!   1. Bing (中国大陆通常可达，结果质量较好)
//!   2. DuckDuckGo Lite (极简 HTML，兜底)
//!
//! 提取降级链:
//!   1. 直接 HTTP GET + DOM 正文提取
//!   2. CDP 浏览器渲染提取 (SPA/JS-heavy)
//!   3. Jina AI 渲染镜像

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;
use scraper::{CaseSensitivity, Html, Selector};
use std::sync::Mutex;
use std::time::Duration;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Agent 缓存 TTL（秒）—— 过期后重建，感知代理变化
/// pub(super)：http.rs 的 client 缓存复用同一 TTL 模式
pub(super) const AGENT_TTL: i64 = 60;

/// reqwest 出口的 cookie 域白名单（视频/风控域，命中才从 vault 取 cookie
/// 拼 Cookie header；未命中域名的请求行为完全不变）。
const COOKIE_HOST_WHITELIST: &[&str] = &["bilibili.com", "douyin.com"];

/// URL host 命中白名单时，从 cookie vault 取该 host 适用的 cookie 拼
/// `Cookie` header 值；未命中或无可用 cookie 返回 `None`。
/// 安全约束：返回值只进请求头，永不进日志。
/// pub(super)：http_request 工具的 use_cookies 参数复用同一白名单出口
pub(super) fn cookie_header_for(url: &str) -> Option<String> {
    let host = reqwest::Url::parse(url).ok()?.host_str()?.to_string();
    let whitelisted = COOKIE_HOST_WHITELIST
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{}", d)));
    if !whitelisted {
        return None;
    }
    let cookies = crate::cookies::vault().cookies_for_host(&host);
    crate::cookies::to_header(&cookies)
}

pub(super) struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// 全局 HTTP Agent 缓存（TTL 过期重建，感知代理变化）
static AGENT_CACHE: Mutex<Option<(reqwest::blocking::Client, i64)>> = Mutex::new(None);

fn get_agent() -> reqwest::blocking::Client {
    let now = unix_now();
    if let Ok(cache) = AGENT_CACHE.lock() {
        if let Some((ref agent, ts)) = *cache {
            if now - ts < AGENT_TTL {
                return agent.clone();
            }
        }
    }
    let builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60));

    let agent = builder
        .build()
        .expect("Failed to build reqwest blocking Client");
    if let Ok(mut cache) = AGENT_CACHE.lock() {
        *cache = Some((agent.clone(), now));
    }
    agent
}

pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// 解析 Bing 重定向 URL
fn extract_bing_url(href: &str) -> String {
    if href.contains("bing.com/redirect") || href.contains("bing.com/url?") {
        if let Some(idx) = href.find("url=") {
            let encoded = &href[idx + 4..];
            let end = encoded.find('&').unwrap_or(encoded.len());
            return urlencoding::decode(&encoded[..end])
                .map(|s| s.into_owned())
                .unwrap_or(href.to_string());
        }
    }
    href.to_string()
}

// ── 搜索引擎多源实现 ──

/// 源1: Bing 搜索
fn search_bing(query: &str, count: usize) -> Result<Vec<SearchResult>, String> {
    let url = format!(
        "https://www.bing.com/search?q={}&count={}&setmkt=en-US&setlang=en",
        urlencoding::encode(query),
        count.clamp(1, 20)
    );
    tracing::info!("[web_search] trying Bing: {}", url);

    let agent = get_agent();
    let response = {
        let mut attempt = 0;
        loop {
            match agent
                .get(&url)
                .header("User-Agent", USER_AGENT)
                .header("Accept-Language", "en-US,en;q=0.9")
                .send()
            {
                Ok(r) => break Ok(r),
                Err(e) => {
                    let msg = format!("{}", e);
                    if attempt == 0 {
                        std::thread::sleep(Duration::from_millis(500));
                        attempt += 1;
                        continue;
                    }
                    break Err(format!("bing request failed: {}", msg));
                }
            }
        }
    }?;

    let html = response
        .text()
        .map_err(|e| format!("read bing response failed: {}", e))?;

    let document = Html::parse_document(&html);
    let result_sel =
        Selector::parse("li.b_algo").map_err(|e| format!("bing selector parse failed: {:?}", e))?;

    let h2_a_sel = Selector::parse("h2 a").unwrap();
    let caption_p_sel = Selector::parse(".b_caption p").unwrap();
    let caption_sel = Selector::parse(".b_caption").unwrap();

    let mut results = Vec::new();
    for element in document.select(&result_sel).take(count) {
        let title = element
            .select(&h2_a_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let url = element
            .select(&h2_a_sel)
            .next()
            .and_then(|e| e.value().attr("href"))
            .map(extract_bing_url)
            .unwrap_or_default();

        let snippet = element
            .select(&caption_p_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                element
                    .select(&caption_sel)
                    .next()
                    .map(|e| e.text().collect::<String>().trim().to_string())
            })
            .unwrap_or_default();

        if !title.is_empty() && !url.is_empty() && url.starts_with("http") {
            results.push(SearchResult {
                title,
                url,
                snippet,
            });
        }
    }

    tracing::info!("[web_search] Bing returned {} results", results.len());
    if results.is_empty() {
        return Err("bing returned no parseable results".to_string());
    }
    Ok(results)
}

/// 源2: DuckDuckGo Lite 搜索
fn search_ddg_lite(query: &str, count: usize) -> Result<Vec<SearchResult>, String> {
    let url = format!(
        "https://lite.duckduckgo.com/lite/?q={}",
        urlencoding::encode(query)
    );
    tracing::info!("[web_search] trying DDG Lite: {}", url);

    let agent = get_agent();
    let response = {
        let mut attempt = 0;
        loop {
            match agent.get(&url).header("User-Agent", USER_AGENT).send() {
                Ok(r) => break Ok(r),
                Err(e) => {
                    let msg = format!("{}", e);
                    if attempt == 0 {
                        std::thread::sleep(Duration::from_millis(500));
                        attempt += 1;
                        continue;
                    }
                    break Err(format!("ddg lite request failed: {}", msg));
                }
            }
        }
    }?;

    let html = response
        .text()
        .map_err(|e| format!("read ddg lite response failed: {}", e))?;

    let document = Html::parse_document(&html);
    let row_sel =
        Selector::parse("table tr").map_err(|e| format!("ddg selector parse failed: {:?}", e))?;
    let a_sel = Selector::parse("a").unwrap();

    let mut results = Vec::new();
    for element in document.select(&row_sel).take(count) {
        let td = element.select(&Selector::parse("td").unwrap()).nth(1);
        let Some(td) = td else { continue };

        let a = td.select(&a_sel).next();
        let Some(a) = a else { continue };

        let title = a.text().collect::<String>().trim().to_string();
        let url = a.value().attr("href").unwrap_or("").to_string();
        if title.is_empty() || url.is_empty() {
            continue;
        }

        let inner_html = td.inner_html();
        let snippet = inner_html
            .split_once("</a>")
            .map(|x| x.1)
            .and_then(|after_link| {
                let parts: Vec<&str> = after_link.split("<br>").collect();
                parts.get(1).map(|s| strip_html_tags(s).trim().to_string())
            })
            .unwrap_or_default();

        results.push(SearchResult {
            title,
            url,
            snippet,
        });
    }

    tracing::info!("[web_search] DDG Lite returned {} results", results.len());
    if results.is_empty() {
        return Err("ddg lite returned no parseable results".to_string());
    }
    Ok(results)
}

fn build_tool_result(query: &str, results: Vec<SearchResult>) -> Result<ToolResult, String> {
    let json_results: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "title": r.title,
                "url": r.url,
                "snippet": r.snippet,
            })
        })
        .collect();

    let payload = serde_json::json!({
        "query": query,
        "count": results.len(),
        "results": json_results,
    });

    Ok(ToolResult::success(
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    ))
}

// ── Task 2: 领域搜索源 (Wikipedia / GitHub / Docs) ──

/// 源3: Wikipedia API 搜索
fn search_wikipedia(query: &str, count: usize) -> Result<Vec<SearchResult>, String> {
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit={}&origin=*",
        urlencoding::encode(query),
        count.clamp(1, 20)
    );
    tracing::info!("[web_search] trying Wikipedia: {}", url);

    let agent = get_agent();
    let response = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| format!("wikipedia request failed: {}", e))?;

    let body = response
        .text()
        .map_err(|e| format!("read wikipedia response failed: {}", e))?;

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("parse wikipedia json failed: {}", e))?;

    let results = json["query"]["search"]
        .as_array()
        .ok_or_else(|| "wikipedia: no search results in response".to_string())?;

    let mut out = Vec::new();
    for item in results.iter().take(count) {
        let title = item["title"].as_str().unwrap_or("").to_string();
        let snippet = item["snippet"].as_str().unwrap_or("").to_string();
        let snippet = strip_html_tags(&snippet);
        let _page_id = item["pageid"].as_i64().unwrap_or(0);
        let url = format!(
            "https://en.wikipedia.org/wiki/{}",
            urlencoding::encode(&title.replace(' ', "_"))
        );

        if !title.is_empty() {
            out.push(SearchResult {
                title,
                url,
                snippet,
            });
        }
    }

    tracing::info!("[web_search] Wikipedia returned {} results", out.len());
    if out.is_empty() {
        return Err("wikipedia returned no results".to_string());
    }
    Ok(out)
}

/// 源4: GitHub API 搜索（搜索仓库）
fn search_github(query: &str, count: usize) -> Result<Vec<SearchResult>, String> {
    let url = format!(
        "https://api.github.com/search/repositories?q={}&per_page={}&sort=stars&order=desc",
        urlencoding::encode(query),
        count.clamp(1, 20)
    );
    tracing::info!("[web_search] trying GitHub: {}", url);

    let agent = get_agent();
    let response = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github.v3+json")
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| format!("github request failed: {}", e))?;

    let body = response
        .text()
        .map_err(|e| format!("read github response failed: {}", e))?;

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("parse github json failed: {}", e))?;

    let items = json["items"]
        .as_array()
        .ok_or_else(|| "github: no items in response".to_string())?;

    let mut out = Vec::new();
    for item in items.iter().take(count) {
        let full_name = item["full_name"].as_str().unwrap_or("").to_string();
        let description = item["description"].as_str().unwrap_or("").to_string();
        let html_url = item["html_url"].as_str().unwrap_or("").to_string();
        let stars = item["stargazers_count"].as_i64().unwrap_or(0);
        let language = item["language"].as_str().unwrap_or("unknown");

        if !full_name.is_empty() && !html_url.is_empty() {
            let snippet = if description.is_empty() {
                format!("[{}] ⭐{}", language, stars)
            } else {
                format!("{} [{}] ⭐{}", description, language, stars)
            };
            out.push(SearchResult {
                title: full_name,
                url: html_url,
                snippet,
            });
        }
    }

    tracing::info!("[web_search] GitHub returned {} results", out.len());
    if out.is_empty() {
        return Err("github returned no results".to_string());
    }
    Ok(out)
}

/// 源5: MDN (developer.mozilla.org) 技术文档搜索
fn search_docs(query: &str, count: usize) -> Result<Vec<SearchResult>, String> {
    let url = format!(
        "https://developer.mozilla.org/api/v1/search?q={}&locale=en-US&size={}",
        urlencoding::encode(query),
        count.clamp(1, 20)
    );
    tracing::info!("[web_search] trying MDN Docs: {}", url);

    let agent = get_agent();
    let response = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| format!("mdn docs request failed: {}", e))?;

    let body = response
        .text()
        .map_err(|e| format!("read mdn docs response failed: {}", e))?;

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("parse mdn docs json failed: {}", e))?;

    let documents = json["documents"]
        .as_array()
        .ok_or_else(|| "mdn docs: no documents in response".to_string())?;

    let mut out = Vec::new();
    for doc in documents.iter().take(count) {
        let title = doc["title"].as_str().unwrap_or("").to_string();
        let summary = doc["summary"].as_str().unwrap_or("").to_string();
        let mdn_url = doc["mdn_url"].as_str().unwrap_or("").to_string();
        let url = format!("https://developer.mozilla.org{}", mdn_url);

        if !title.is_empty() {
            out.push(SearchResult {
                title,
                url,
                snippet: summary,
            });
        }
    }

    tracing::info!("[web_search] MDN Docs returned {} results", out.len());
    if out.is_empty() {
        return Err("mdn docs returned no results".to_string());
    }
    Ok(out)
}

// ── HTML 解析辅助（web::extract 保留） ──

/// 把 HTML 剥成纯文本:去掉 script/style + 所有标签 + 解码实体 + 合并空白
pub(super) fn html_to_text(html: &str, max_chars: usize) -> String {
    let mut text = html.to_string();
    text = match regex::Regex::new(r#"(?is)<script[^>]*>[\s\S]*?</script>"#) {
        Ok(re) => re.replace_all(&text, "").to_string(),
        Err(_) => text,
    };
    text = match regex::Regex::new(r#"(?is)<style[^>]*>[\s\S]*?</style>"#) {
        Ok(re) => re.replace_all(&text, "").to_string(),
        Err(_) => text,
    };
    text = match regex::Regex::new(r#"<[^>]+>"#) {
        Ok(re) => re.replace_all(&text, " ").to_string(),
        Err(_) => text,
    };
    text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'");
    text = match regex::Regex::new(r#"\s+"#) {
        Ok(re) => re.replace_all(&text, " ").to_string(),
        Err(_) => text,
    };
    text = text.trim().to_string();

    if text.len() > max_chars {
        let trunc: String = text.chars().take(max_chars).collect();
        format!(
            "{}...\n\n[Truncated: {} chars total, {} returned]",
            trunc,
            text.chars().count(),
            max_chars
        )
    } else {
        text
    }
}

pub(super) fn strip_html_tags(html: &str) -> String {
    let stripped = match regex::Regex::new(r#"<[^>]+>"#) {
        Ok(re) => re.replace_all(html, "").to_string(),
        Err(_) => html.to_string(),
    };
    stripped
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}

// ── Task 1: DOM 正文提取（Reader Mode 风格） ──

/// DOM 正文提取算法 — 类似 Firefox Reader Mode / readability
pub(super) fn extract_readable(html: &str, max_chars: usize) -> String {
    let doc = Html::parse_document(html);
    let container = find_content_container(&doc);

    let text = match container {
        Some(root) => {
            let mut parts: Vec<String> = Vec::new();
            render_element(&root, &mut parts, 0);
            let raw = parts.join("\n").trim().to_string();
            collapse_excessive_whitespace(&raw)
        }
        None => html_to_text(html, usize::MAX),
    };

    if text.chars().count() > max_chars {
        let trunc: String = text.chars().take(max_chars).collect();
        format!(
            "{}...\n\n[Truncated: {} chars total, {} returned]",
            trunc,
            text.chars().count(),
            max_chars
        )
    } else {
        text
    }
}

/// 多层 DOM 内容容器定位
///
/// 策略：先结构后内容，递进筛选。
/// - Phase 1: CSS 选择器快速匹配（常见语义结构）
/// - Phase 2: DOM 结构评分分析（逐层评分、递进钻取）
fn find_content_container(doc: &Html) -> Option<scraper::ElementRef<'_>> {
    // ── Phase 1: CSS selectors 快速路径 ──
    let selectors = [
        "article",
        "main",
        "[role=main]",
        ".post-content",
        ".article-content",
        ".entry-content",
        ".post-body",
        "#content",
        ".content",
        "#article",
        ".post",
        ".article",
        "#main-content",
        ".main-content",
        ".documentation",
        ".markdown-body",
    ];

    for s in &selectors {
        if let Ok(sel) = Selector::parse(s) {
            if let Some(el) = doc.select(&sel).next() {
                let text_len = el.text().collect::<String>().trim().len();
                if text_len > 50 {
                    return Some(el);
                }
            }
        }
    }

    // ── Phase 2: DOM 评分分析 ──
    if let Some(body) = doc.select(&Selector::parse("body").ok()?).next() {
        // 解开单子级布局包装（如 <div id="root"> → <div class="app">）
        let content_root = unwrap_layout_wrappers(&body);
        if let Some(found) = find_content_scope(&content_root) {
            return Some(found);
        }
    }

    // 最终降级：返回 body
    Selector::parse("body")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
}

/// 解开单子级布局包装链（Next.js/React/Vue 常见模式）
fn unwrap_layout_wrappers<'a>(el: &scraper::ElementRef<'a>) -> scraper::ElementRef<'a> {
    let mut current = *el;
    let mut depth = 0;
    loop {
        if depth > 5 {
            break;
        }
        let children: Vec<_> = current
            .children()
            .filter_map(scraper::ElementRef::wrap)
            .filter(|c| !matches!(c.value().name(), "script" | "style" | "noscript" | "link"))
            .collect();

        if children.len() == 1 {
            let tag = children[0].value().name();
            if matches!(tag, "div" | "section" | "article" | "main") {
                current = children[0];
                depth += 1;
                continue;
            }
        }
        break;
    }
    current
}

/// 判断元素是否包含直接文本（非子代元素内的文本）
fn has_direct_text(el: &scraper::ElementRef) -> bool {
    for child in el.children() {
        if let scraper::Node::Text(t) = child.value() {
            if !t.text.trim().is_empty() {
                return true;
            }
        }
    }
    false
}

/// 递进筛：缩小范围排除噪声，找到能容纳所有内容模块的容器
///
/// 核心逻辑：
/// 1. 评分当前节点的所有子节点
/// 2. 过滤噪声（低分块）
/// 3. 如果有多块内容、或当前节点自身有直接文本 → 停（找到范围）
/// 4. 如果只有一个内容块且当前是薄包装 → 缩进一层继续
fn find_content_scope<'a>(el: &scraper::ElementRef<'a>) -> Option<scraper::ElementRef<'a>> {
    let children: Vec<_> = el
        .children()
        .filter_map(scraper::ElementRef::wrap)
        .filter(|c| !matches!(c.value().name(), "script" | "style" | "noscript" | "link"))
        .collect();

    if children.is_empty() {
        // 叶子节点：有文本量就返回
        if compute_element_stats(el).text_len > 80 {
            return Some(*el);
        }
        return None;
    }

    // 评分所有子节点，过滤噪声
    let mut scored: Vec<(scraper::ElementRef, f64)> = children
        .iter()
        .filter_map(|child| {
            let tag = child.value().name();
            if matches!(tag, "script" | "style" | "noscript" | "iframe" | "link") {
                return None;
            }
            let stats = compute_element_stats(child);
            if stats.text_len == 0 && stats.descendant_tag_count <= 1 {
                return None;
            }
            let mut score = content_score(&stats, child);
            if is_noise_element(child) {
                score -= 10.0;
            }
            Some((*child, score))
        })
        .filter(|(_, s)| *s > -5.0)
        .collect();

    if scored.is_empty() {
        return None;
    }

    // 按评分降序
    scored.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let content_count = scored.iter().filter(|(_, s)| *s > 2.0).count();
    let el_has_direct_text = has_direct_text(el);
    let el_stats = compute_element_stats(el);

    // ── 停条件：已有多个内容块，或当前节点自身是内容容器 ──
    if content_count >= 2 || el_has_direct_text || el_stats.text_len > 300 {
        return Some(*el);
    }

    // ── 缩进条件：唯一内容块 + 当前是薄包装 ──
    if content_count == 1 && !el_has_direct_text && el_stats.text_len < 150 {
        let best = &scored[0];
        if best.1 > 3.0 {
            // 检查最佳块是否自身就是内容容器（有文本有段落）
            let best_stats = compute_element_stats(&best.0);
            // 容器特征：标签多但自身无段落 → 继续缩进
            if best_stats.p_count == 0 && best_stats.descendant_tag_count > 5 {
                return find_content_scope(&best.0);
            }
            return Some(best.0);
        }
    }

    // ── 有正分块就停在此层 ──
    if scored.iter().any(|(_, s)| *s > 0.0) {
        return Some(*el);
    }

    None
}

/// 元素统计数据
#[derive(Default)]
struct ElementStats {
    text_len: usize,
    link_text_len: usize,
    p_count: usize,
    descendant_tag_count: usize,
}

/// 计算元素统计数据
fn compute_element_stats(el: &scraper::ElementRef) -> ElementStats {
    let mut text_len = 0;
    let mut link_text_len = 0;
    let mut p_count = 0;
    let mut descendant_tag_count = 1;

    for node in el.descendants() {
        match node.value() {
            scraper::Node::Text(t) => {
                let trimmed = t.text.trim();
                if !trimmed.is_empty() {
                    text_len += trimmed.len();
                }
            }
            scraper::Node::Element(e) => {
                descendant_tag_count += 1;
                match e.name() {
                    "a" => {
                        for child_node in node.children() {
                            if let scraper::Node::Text(t) = child_node.value() {
                                let trimmed = t.text.trim();
                                if !trimmed.is_empty() {
                                    link_text_len += trimmed.len();
                                }
                            }
                        }
                    }
                    "p" => p_count += 1,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    ElementStats {
        text_len,
        link_text_len,
        p_count,
        descendant_tag_count,
    }
}

/// 基于统计数据为内容块评分
fn content_score(stats: &ElementStats, el: &scraper::ElementRef) -> f64 {
    let tag = el.value().name();

    // 标签类型基础分
    let base = match tag {
        "main" | "article" => 8.0,
        "section" => 4.0,
        "div" => 0.0,
        "p" | "td" | "th" => -1.0,
        "header" | "footer" | "nav" | "aside" => -3.0,
        "ul" | "ol" | "table" => 1.0,
        _ => -1.0,
    };

    let link_density = if stats.text_len > 0 {
        stats.link_text_len as f64 / stats.text_len as f64
    } else {
        1.0
    };

    let mut score = base;

    // 文本量加分
    score += (stats.text_len as f64 / 300.0).min(8.0);

    // 高链接密度 = 导航/菜单，扣分
    if link_density > 0.6 {
        score -= 8.0;
    } else if link_density > 0.3 {
        score -= 2.0;
    }

    // 段落加分（文章类内容特征）
    if stats.p_count >= 2 {
        score += (stats.p_count as f64).min(5.0);
    }

    // 标签多但文本少 = 布局包装
    if stats.text_len < 100 && stats.descendant_tag_count > 30 {
        score -= 5.0;
    }

    // class/id 暗示的内容加分
    let id = el.attr("id").unwrap_or("");
    let class = el.attr("class").unwrap_or("");
    let combined = format!("{} {}", id, class).to_lowercase();

    if combined.contains("content") || combined.contains("article") || combined.contains("post") {
        score += 3.0;
    }
    if combined.contains("main") {
        score += 2.0;
    }

    score
}

/// 检查是否为噪声元素（导航/广告/侧栏等）
fn is_noise_element(el: &scraper::ElementRef) -> bool {
    if matches!(el.value().name(), "nav" | "footer" | "aside" | "header") {
        return true;
    }
    let id = el.attr("id").unwrap_or("");
    let class = el.attr("class").unwrap_or("");
    let combined = format!("{} {}", id, class).to_lowercase();
    let noise = [
        "sidebar",
        "advertisement",
        "ad-",
        "ad ",
        "-ad",
        "nav",
        "menu",
        "footer",
        "widget",
        "comment",
        "comments",
        "social",
        "share",
        "related",
        "recommend",
        "sponsor",
        "promoted",
        "banner",
    ];
    noise.iter().any(|p| combined.contains(p))
}

/// 质量自查：检查正文容器外是否有明显遗漏的内容模块
///
/// 策略：扫描 body 下所有文本密集型元素，递归查找兄弟姐妹节点，
/// 与已提取文本比较词重叠度。重叠度低（<70%）说明是遗漏的独立内容模块。
/// 检查是否为 UI 导航链接（pagination/breadcrumb 等），渲染时跳过
fn is_nav_link(el: &scraper::ElementRef) -> bool {
    // rel 属性明确标记了导航
    if let Some(rel) = el.attr("rel") {
        if matches!(rel, "prev" | "next" | "previous") {
            return true;
        }
    }
    let text = collect_inline_text(el).to_lowercase();
    if text.is_empty() {
        return false;
    }
    // 纯导航关键词（首词匹配，应对 "Next useActionState" 这类拼接）
    let first_word = text.split_whitespace().next().unwrap_or("");
    matches!(first_word, "previous" | "next" | "prev" | "back" | "home")
        || matches!(
            text.as_str(),
            "← previous" | "next →" | "previous page" | "next page"
        )
}

/// 递归渲染 DOM 元素为结构化文本
fn render_element(el: &scraper::ElementRef, parts: &mut Vec<String>, _depth: usize) {
    let tag = el.value().name();

    if matches!(
        tag,
        "script" | "style" | "noscript" | "iframe" | "nav" | "aside" | "footer"
    ) {
        return;
    }

    if is_skip_class(el) {
        return;
    }

    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = tag[1..].parse::<usize>().unwrap_or(1);
            let prefix = "#".repeat(level);
            let text = collect_inline_text(el);
            if !text.is_empty() {
                ensure_trailing_newline(parts);
                parts.push(format!("{} {}", prefix, text));
                parts.push(String::new());
            }
        }
        "p" => {
            let text = collect_inline_text(el);
            if !text.is_empty() {
                parts.push(text);
                parts.push(String::new());
            }
        }
        "a" => {
            let url = el.attr("href").unwrap_or("");
            let text = el.text().collect::<String>().trim().to_string();
            if !text.is_empty() && !is_nav_link(el) {
                if !url.is_empty() && !url.starts_with('#') && !url.starts_with("javascript:") {
                    parts.push(format!("[{}]({})", text, url));
                } else {
                    parts.push(text);
                }
            }
        }
        "ul" | "ol" => render_list(el, parts),
        "table" => {
            ensure_trailing_newline(parts);
            for child in el.children() {
                if let Some(child_el) = scraper::ElementRef::wrap(child) {
                    match child_el.value().name() {
                        "thead" | "tbody" | "tfoot" | "tr" | "caption" | "colgroup" | "col" => {
                            render_element(&child_el, parts, _depth + 1);
                        }
                        _ => {}
                    }
                }
            }
            parts.push(String::new());
        }
        "tr" => {
            let cells: Vec<String> = el
                .children()
                .filter_map(scraper::ElementRef::wrap)
                .filter(|c| matches!(c.value().name(), "td" | "th"))
                .map(|c| collect_inline_text(&c))
                .filter(|t| !t.is_empty())
                .collect();
            if !cells.is_empty() {
                ensure_trailing_newline(parts);
                parts.push(format!("| {} |", cells.join(" | ")));
            }
        }
        "caption" => {
            let text = collect_inline_text(el);
            if !text.is_empty() {
                ensure_trailing_newline(parts);
                parts.push(format!("[{}]", text));
            }
        }
        "dl" => {
            ensure_trailing_newline(parts);
            for child in el.children() {
                if let Some(child_el) = scraper::ElementRef::wrap(child) {
                    match child_el.value().name() {
                        "dt" => {
                            let text = collect_inline_text(&child_el);
                            if !text.is_empty() {
                                parts.push(format!("  {}", text));
                            }
                        }
                        "dd" => {
                            let text = collect_inline_text(&child_el);
                            if !text.is_empty() {
                                parts.push(format!("    → {}", text));
                            }
                        }
                        _ => {}
                    }
                }
            }
            parts.push(String::new());
        }
        "blockquote" => {
            let text = collect_inline_text(el);
            if !text.is_empty() {
                parts.push(format!("> {}", text));
                parts.push(String::new());
            }
        }
        "pre" => {
            let text = el.text().collect::<String>();
            if !text.trim().is_empty() {
                parts.push(format!("```\n{}\n```", text.trim()));
                parts.push(String::new());
            }
        }
        "code" => {
            let text = el.text().collect::<String>().trim().to_string();
            if !text.is_empty() {
                parts.push(format!("`{}`", text));
            }
        }
        "img" => {
            let alt = el.attr("alt").unwrap_or("");
            if !alt.is_empty() {
                parts.push(format!("[图片: {}]", alt));
            }
        }
        "br" => {
            parts.push("\n".to_string());
        }
        "div" | "section" | "article" | "main" | "header" | "details" | "summary" => {
            for child in el.children() {
                if let Some(child_el) = scraper::ElementRef::wrap(child) {
                    render_element(&child_el, parts, _depth + 1);
                }
            }
        }
        _ => {
            for child in el.children() {
                if let Some(child_el) = scraper::ElementRef::wrap(child) {
                    render_element(&child_el, parts, _depth + 1);
                }
            }
        }
    }
}

/// 渲染列表（带缩进标记）
fn render_list(el: &scraper::ElementRef, parts: &mut Vec<String>) {
    ensure_trailing_newline(parts);
    for child in el.children() {
        if let Some(child_el) = scraper::ElementRef::wrap(child) {
            if child_el.value().name() == "li" {
                let text = collect_inline_text(&child_el);
                if !text.is_empty() {
                    parts.push(format!("- {}", text));
                }
            }
        }
    }
    parts.push(String::new());
}

/// 收集内联文本（忽略块级分隔和空白）
fn collect_inline_text(el: &scraper::ElementRef) -> String {
    let mut parts = Vec::new();
    for child in el.children() {
        match child.value() {
            scraper::Node::Text(t) => {
                let text = t.text.trim();
                if !text.is_empty() {
                    parts.push(text.to_string());
                }
            }
            scraper::Node::Element(_) => {
                if let Some(child_el) = scraper::ElementRef::wrap(child) {
                    let tag = child_el.value().name();
                    if tag == "a" {
                        let url = child_el.attr("href").unwrap_or("");
                        let text = child_el.text().collect::<String>().trim().to_string();
                        if !text.is_empty() {
                            if !url.is_empty()
                                && !url.starts_with('#')
                                && !url.starts_with("javascript:")
                            {
                                parts.push(format!("[{}]({})", text, url));
                            } else {
                                parts.push(text);
                            }
                        }
                    } else if tag == "code" {
                        let text = child_el.text().collect::<String>().trim().to_string();
                        if !text.is_empty() {
                            parts.push(format!("`{}`", text));
                        }
                    } else if tag == "br" {
                        parts.push(" ".to_string());
                    } else if tag == "img" {
                        let alt = child_el.attr("alt").unwrap_or("");
                        if !alt.is_empty() {
                            parts.push(format!("[图片: {}]", alt));
                        }
                    } else if !matches!(tag, "script" | "style" | "noscript") {
                        // 递进收集内联文本，保留子元素的边界（<br>/<span>/<a> 等）
                        let inner = collect_inline_text(&child_el);
                        if !inner.is_empty() {
                            parts.push(inner);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    parts.join(" ").trim().to_string()
}

/// 检查是否有需要跳过的 class 名
fn is_skip_class(el: &scraper::ElementRef) -> bool {
    let skip = [
        "sidebar",
        "advertisement",
        "ads",
        "ad",
        "nav",
        "footer",
        "menu",
        "widget",
        "comment",
        "comments",
        "social",
        "share",
        "related",
        "recommend",
    ];
    for cls in skip {
        if el.value().has_class(cls, CaseSensitivity::CaseSensitive) {
            return true;
        }
    }
    false
}

/// 确保 parts 末尾有空行（段落间距）
fn ensure_trailing_newline(parts: &mut Vec<String>) {
    if let Some(last) = parts.last() {
        if !last.is_empty() {
            parts.push(String::new());
        }
    }
}

/// 合并过多空白（最多一个空行间隔）
fn collapse_excessive_whitespace(s: &str) -> String {
    let re = match regex::Regex::new(r#"\n{3,}"#) {
        Ok(r) => r,
        Err(_) => return s.to_string(),
    };
    let s = re.replace_all(s, "\n\n");
    let re = match regex::Regex::new(r#"[ \t]+\n"#) {
        Ok(r) => r,
        Err(_) => return s.to_string(),
    };
    let s = re.replace_all(&s, "\n");
    s.trim().to_string()
}

// ── 工具注册 ──

impl ToolRegistry {
    pub(crate) fn register_web_search(&mut self) {
        self.register(ToolDef {
            name: "web_search".to_string(),
            description: "Search the web or domain-specific sources (wiki/github/docs)".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "count": { "type": "integer", "minimum": 1, "maximum": 20, "default": 5, "description": "Number of results" },
                    "source": { "type": "string", "enum": ["web", "wiki", "github", "docs"], "default": "web", "description": "Search source" }
                },
                "required": ["query"]
            }),
            category: ToolCategory::WebSearch,
            executor: |params, _ctx| {
                let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let count = params.get("count").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let source = params.get("source").and_then(|v| v.as_str()).unwrap_or("web").to_string();

                if query.trim().is_empty() {
                    return Ok(ToolResult::failure("query cannot be empty"));
                }

                super::run_blocking(move || {
                    match source.as_str() {
                        "wiki" => {
                            if let Ok(results) = search_wikipedia(&query, count) {
                                return build_tool_result(&query, results);
                            }
                            Ok(ToolResult::failure(
                                "Wikipedia search failed (unreachable or blocked)"
                            ))
                        }
                        "github" => {
                            if let Ok(results) = search_github(&query, count) {
                                return build_tool_result(&query, results);
                            }
                            Ok(ToolResult::failure(
                                "GitHub search failed (unreachable or blocked)"
                            ))
                        }
                        "docs" => {
                            if let Ok(results) = search_docs(&query, count) {
                                return build_tool_result(&query, results);
                            }
                            Ok(ToolResult::failure(
                                "MDN Docs search failed (unreachable or blocked)"
                            ))
                        }
                        _ => {
                            if let Ok(results) = search_bing(&query, count) {
                                return build_tool_result(&query, results);
                            }
                            tracing::warn!("[web_search] Bing failed, falling back to DDG Lite");

                            if let Ok(results) = search_ddg_lite(&query, count) {
                                return build_tool_result(&query, results);
                            }

                            Ok(ToolResult::failure(
                                "All search sources failed (Bing + DDG Lite unreachable or blocked)"
                            ))
                        }
                    }
                })
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_web_extract(&mut self) {
        self.register(ToolDef {
            name: "web_extract".to_string(),
            description: "Fetch a URL and return readable text (HTML stripped). Automatically uses CDP browser rendering as fallback if HTTP fetch returns empty content (e.g. SPA/JS-heavy pages).".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Full URL including https://" },
                    "max_chars": { "type": "integer", "minimum": 100, "maximum": 50000, "default": 30000, "description": "Max chars to return" }
                },
                "required": ["url"]
            }),
            category: ToolCategory::WebSearch,
            executor: |params, _ctx| {
                let url = params.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let max_chars = params.get("max_chars").and_then(|v| v.as_u64()).unwrap_or(30000) as usize;

                if url.trim().is_empty() {
                    return Ok(ToolResult::failure("url cannot be empty"));
                }
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Ok(ToolResult::failure(
                        "url must start with http:// or https://"
                    ));
                }

                super::run_blocking(move || {
                    let agent = get_agent();

                    // ── 第一层: 直接 HTTP ──
                    let direct_result = fetch_direct(&agent, &url);
                    let _direct_content_ok = match &direct_result {
                        Ok(body) => {
                            let text = extract_readable(body, max_chars);
                            let trimmed = text.trim();
                            // 内容充分（>= 2000 chars）直接返回
                            if trimmed.len() >= 2000 {
                                return Ok(ToolResult::success(text));
                            }
                            tracing::info!(
                                "[web_extract] direct content too short ({} chars), trying CDP browser fallback",
                                trimmed.len()
                            );
                            false
                        }
                        Err(e) => {
                            tracing::warn!("[web_extract] direct fetch failed: {}", e);
                            false
                        }
                    };

                    // ── 第二层: CDP 浏览器渲染提取 ──
                    tracing::info!("[web_extract] launching CDP browser for: {}", url);
                    match fetch_via_browser(&url, max_chars) {
                        Ok(text) if !text.trim().is_empty() => {
                            return Ok(ToolResult::success(
                                format!("[via CDP browser render]\n\n{}", text)
                            ));
                        }
                        Ok(_) => {
                            tracing::warn!("[web_extract] browser returned empty content");
                        }
                        Err(browser_err) => {
                            tracing::warn!("[web_extract] browser fetch failed: {}", browser_err);
                        }
                    }

                    // ── 第三层: Jina AI 渲染镜像降级 ──
                    tracing::info!("[web_extract] trying Jina AI mirror for: {}", url);
                    let mirror_url = format!(
                        "http://r.jina.ai/http://{}",
                        url.trim_start_matches("http://").trim_start_matches("https://")
                    );
                    match fetch_direct(&agent, &mirror_url) {
                        Ok(mirror_body) => {
                            let text = extract_readable(&mirror_body, max_chars);
                            if !text.trim().is_empty() {
                                return Ok(ToolResult::success(
                                    format!("[via Jina AI mirror]\n\n{}", text)
                                ));
                            }
                            Ok(ToolResult::failure(format!(
                                "All methods failed for {} (direct/browser/mirror all returned empty)",
                                url
                            )))
                        }
                        Err(e) => {
                            let direct_info = match &direct_result {
                                Ok(body) => {
                                    let text = extract_readable(body, max_chars);
                                    format!("HTTP content ({} chars)", text.len())
                                }
                                Err(e) => format!("HTTP error: {}", e),
                            };
                            Ok(ToolResult::failure(format!(
                                "Failed to extract content from {}:\n- Direct: {}\n- Browser: failed\n- Jina mirror: {}",
                                url, direct_info, e
                            )))
                        }
                    }
                })
            },
            depends_on: vec![],
        });
    }
}

/// 直接 HTTP GET 请求，返回 body 字符串
fn fetch_direct(agent: &reqwest::blocking::Client, url: &str) -> Result<String, String> {
    // 按域白名单附 cookie（命中才走 vault；未命中域名行为完全不变）
    let cookie_header = cookie_header_for(url);
    let mut attempt = 0;
    loop {
        let mut req = agent.get(url).header("User-Agent", USER_AGENT);
        if let Some(ref h) = cookie_header {
            req = req.header("Cookie", h);
        }
        match req.send() {
            Ok(r) => {
                let code = r.status();
                if !code.is_success() {
                    // reqwest blocking: HTTP error status comes as Ok(Response), not Err
                    let msg = format!("HTTP {}", code.as_u16());
                    if attempt == 0 {
                        std::thread::sleep(Duration::from_millis(500));
                        attempt += 1;
                        continue;
                    }
                    return Err(msg);
                }
                let body = r.text().map_err(|e| format!("read body failed: {}", e))?;
                if body.trim().is_empty() {
                    return Err(format!("HTTP {} (empty body)", code.as_u16()));
                }
                return Ok(body);
            }
            Err(e) => {
                let msg = format!("{}", e);
                if attempt == 0 {
                    std::thread::sleep(Duration::from_millis(500));
                    attempt += 1;
                    continue;
                }
                return Err(format!("request failed: {}", msg));
            }
        }
    }
}

/// 通过 CDP 浏览器渲染页面并提取文本
///
/// 复用进程级共享 BrowserClient（单例）：同一 profile_dir 不允许多实例并发
/// launch。浏览器操作必须在常驻 browser runtime 上执行（临时 runtime drop
/// 会杀死 CDP handler）；实例用毕保持存活供后续复用，不在此 close。
fn fetch_via_browser(url: &str, max_chars: usize) -> Result<String, String> {
    crate::browser::runtime().block_on(async {
        let mut guard = crate::browser::get_or_launch(true) // headless mode
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("Chrome not found") {
                    "CDP browser rendering unavailable: no Chrome/Chromium/Edge installed. Install a Chromium-based browser or use direct HTTP fetch.".to_string()
                } else if msg.contains("Failed to deserialize") || msg.contains("WebSocket") || msg.contains("WS Connection") {
                    "CDP browser rendering unavailable: browser engine version mismatch (the installed Chrome/Edge version is incompatible with the CDP protocol layer). Use direct HTTP fetch as fallback.".to_string()
                } else if msg.contains("Connection refused") || msg.contains("timed out") {
                    format!("CDP browser rendering unavailable: browser process failed to start ({}).", msg)
                } else {
                    msg
                }
            })?;
        let client = guard.as_mut().ok_or("browser client unavailable")?;

        client.navigate(url)
            .await
            .map_err(|e| format!("browser navigate failed: {}", e))?;

        // Extra wait for JS rendering (SPA hydration, lazy loading)
        tokio::time::sleep(Duration::from_millis(2000)).await;

        // Get rendered page text
        let js = format!(
            r#"(function() {{
                const text = document.body.innerText || '';
                return text.substring(0, {});
            }})()"#,
            max_chars
        );

        let value = client.evaluate(&js)
            .await
            .map_err(|e| format!("browser evaluate failed: {}", e))?;

        let text = match &value {
            serde_json::Value::String(s) => s.clone(),
            _ => value.as_str().unwrap_or("").to_string(),
        };

        Ok(text)
    })
}
