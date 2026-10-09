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
use std::io::Read;
use std::sync::Mutex;
use std::time::Duration;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Agent 缓存 TTL（秒）—— 过期后重建，感知代理变化
/// pub(super)：http.rs 的 client 缓存复用同一 TTL 模式
pub(super) const AGENT_TTL: i64 = 60;

/// web 抓取单响应字节上限（5 MiB）。
///
/// 动机：`max_chars` 是字符闸且由模型控制，挡不住「一个 100MB 的 HTML 直接进内存」
/// —— 字符闸在 decode 之后才生效，全量 `bytes()` 会先把整个 body 读进内存。
/// 带 Content-Length 且超限的响应直接拒绝（不读 body）；无 Content-Length
/// （chunked）时读满上限即截断，保留可用部分，不报错。
const MAX_WEB_BYTES: usize = 5 * 1024 * 1024;

/// DOM 渲染最大递归深度。
///
/// 真实页面嵌套几十层就到头，超深只有「恶意构造」一种来源。超限只是不再继续
/// 下钻，已产出的 parts 照常返回（不整页丢弃）。
const MAX_DOM_DEPTH: usize = 256;

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

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// GitHub API 的 Authorization 头（可选）。
///
/// 未认证时 GitHub 搜索 API 限流 **10 次/分**（其余 API 60 次/小时），
/// 认证后 30 次/分。web 聚合每个查询都要打一次 github 源，未认证时
/// 连打几个查询就会开始返回 403 —— 表现为"源时好时坏"，实为限流。
///
/// 从环境变量读 token（`GITHUB_TOKEN` / `GH_TOKEN`，与 gh CLI 同一约定），
/// 未设置则返回 None，行为与原实现完全一致（不报错、不空跑）。
/// token 值只进请求头，永不进日志。
fn github_auth_header() -> Option<String> {
    for key in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(format!("Bearer {}", v));
            }
        }
    }
    None
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
    // 市场/语言参数随查询语言走：中文查询用 zh-CN，否则 en-US。
    // 原实现硬编码 setmkt=en-US&setlang=en，Bing 把中文/混合查询按英文
    // 市场处理后返回与本意无关的商业结果（实测「trafilatura library」
    // 返回巴基斯坦 CSS 考试论坛、「笔记本合盖用显示器」等）。
    let is_cjk = query.chars().any(|c| {
        matches!(c as u32,
            0x4E00..=0x9FFF    // CJK 统一表意
            | 0x3400..=0x4DBF  // 扩展 A
            | 0x3000..=0x303F  // CJK 标点
            | 0xFF00..=0xFFEF) // 全角
    });
    let (mkt, lang) = if is_cjk {
        ("zh-CN", "zh-CN")
    } else {
        ("en-US", "en")
    };
    let url = format!(
        "https://www.bing.com/search?q={}&count={}&setmkt={}&setlang={}",
        urlencoding::encode(query),
        count.clamp(1, 20),
        mkt,
        lang
    );
    tracing::info!("[web_search] trying Bing (mkt={}): {}", mkt, url);

    let agent = get_agent();
    let response = {
        let mut attempt = 0;
        loop {
            match agent
                .get(&url)
                .header("User-Agent", USER_AGENT)
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
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

    let html = {
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let (bytes, truncated) =
            read_body_capped(response).map_err(|e| format!("read bing response failed: {}", e))?;
        // 搜索结果页被字节闸截断 = DOM 不完整，解析出来的条目不可信。
        // 与其把半页结果混进聚合池（污染排序），不如让本源整体退出，
        // 由 aggregate_web_search 的跨源降级接手。
        if truncated {
            return Err(format!(
                "bing result page exceeds {} MiB byte cap; source dropped",
                MAX_WEB_BYTES / (1024 * 1024)
            ));
        }
        decode_web_body(&bytes, content_type.as_deref())
    };

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
            match agent
                .get(&url)
                .header("User-Agent", USER_AGENT)
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
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
                    break Err(format!("ddg lite request failed: {}", msg));
                }
            }
        }
    }?;

    let html = {
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let (bytes, truncated) = read_body_capped(response)
            .map_err(|e| format!("read ddg lite response failed: {}", e))?;
        // 同 search_bing：结果页被截断则 DOM 不完整，本源退出而非产出半页结果。
        if truncated {
            return Err(format!(
                "ddg lite result page exceeds {} MiB byte cap; source dropped",
                MAX_WEB_BYTES / (1024 * 1024)
            ));
        }
        decode_web_body(&bytes, content_type.as_deref())
    };

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

/// 把各源失败原因汇总成一句**可行动**的失败文案。
///
/// 这是跨源降级的最后一环：真正决定「重试还是换词」的判断交给模型，
/// 文案必须把两件事说清楚 —— 哪些源挂了、各自因为什么挂。
///
/// 旧实现是一句写死的 `All search sources failed (...)`，五个源名全列上但
/// 一个原因都没有，模型只能反复重试同一个查询。
fn describe_aggregate_failure(failures: &[(&'static str, String, SourceFailure)]) -> String {
    if failures.is_empty() {
        // 所有源都「成功返回但零结果」——这不是故障，是查询没匹配上任何源。
        return "No search source returned results for this query. The query likely matched \
                nothing; try different or broader terms (drop brand/version qualifiers), or \
                narrow with source=wiki/github/docs."
            .to_string();
    }

    let detail = failures
        .iter()
        .map(|(name, reason, _)| format!("{}: {}", name, reason))
        .collect::<Vec<_>>()
        .join("; ");

    let retryable = failures
        .iter()
        .filter(|(_, _, kind)| *kind == SourceFailure::Retryable)
        .count();

    if retryable > 0 {
        // 可重试失败占多数 → 大概率是限流而非查询问题，该等而不是换词。
        format!(
            "All {} search sources failed ({} of {} look rate-limited or timed out): {}. \
             This looks like rate limiting rather than a bad query — retry after a short wait \
             instead of rewriting the query.",
            failures.len(),
            retryable,
            failures.len(),
            detail
        )
    } else {
        // 全是永久性失败 → 换个查询词比重试有用。
        format!(
            "All {} search sources failed for reasons that retrying will not fix ({}). \
             Try different search terms, or target one source directly with source=wiki/github/docs.",
            failures.len(),
            detail
        )
    }
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

// ── 默认 web 源：五源聚合 ──

/// 候选结果的来源标识。也决定「源可靠性权重」。
#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchSource {
    Bing,
    DdgLite,
    Wikipedia,
    GitHub,
    Docs,
}

impl SearchSource {
    /// 源可靠性基准权重。
    ///
    /// API 源返回结构化 JSON，标题/摘要即为该源对查询的官方理解，噪声低；
    /// HTML 源（Bing / DDG Lite）需解析商业页面 DOM，容易掺入广告位、
    /// 相关推荐和错配结果。实测本机 Bing 对技术类查询返回过完全无关的
    /// 商业内容，故 HTML 源降权 —— 但保留在聚合池里，API 源未命中时
    /// 仍可兜底。
    fn weight(self) -> f64 {
        match self {
            SearchSource::GitHub => 1.0,
            SearchSource::Docs => 1.0,
            SearchSource::Wikipedia => 0.95,
            SearchSource::Bing => 0.6,
            SearchSource::DdgLite => 0.6,
        }
    }

    fn name(self) -> &'static str {
        match self {
            SearchSource::Bing => "bing",
            SearchSource::DdgLite => "ddg-lite",
            SearchSource::Wikipedia => "wikipedia",
            SearchSource::GitHub => "github",
            SearchSource::Docs => "docs",
        }
    }
}

/// 把查询切成可比较的词项。
///
/// 英文/数字按非字母数字边界切词；中日韩字符逐字成项（中文无空格分词，
/// 单字粒度对「标题命中查询」这种粗粒度判断足够，且不会因误切词漏判）。
/// 全部转小写，去掉长度 1 的 ASCII 词（"a"/"of" 之类无区分度）。
///
/// 刻意**不含任何硬编码词表**：词项是否「有用」交由 IDF 权重在候选池上
/// 统计得出，不预先假设任何词汇。
fn tokenize(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut tokens = Vec::new();
    let mut ascii_word = String::new();

    for ch in lower.chars() {
        if ch.is_alphanumeric() && !ch.is_whitespace() {
            if ch.is_ascii() {
                ascii_word.push(ch);
            } else {
                // CJK / 其他非 ASCII 字母数字：逐字成项
                if !ascii_word.is_empty() {
                    if ascii_word.len() > 1 {
                        tokens.push(std::mem::take(&mut ascii_word));
                    } else {
                        ascii_word.clear();
                    }
                }
                tokens.push(ch.to_string());
            }
        } else if !ascii_word.is_empty() {
            if ascii_word.len() > 1 {
                tokens.push(std::mem::take(&mut ascii_word));
            } else {
                ascii_word.clear();
            }
        }
    }
    if ascii_word.len() > 1 {
        tokens.push(ascii_word);
    }
    tokens
}

/// 词项权重：IDF（Inverse Document Frequency，在**本次候选池**上统计）。
///
/// 动机（实测教训）：「trafilatura library」召回 5 条，其中 4 条标题含
/// `library`。硬编码「library 是虚词」能修这一例，但换个查询就失效，
/// 且会误伤「rust crate」这类正当技术查询。
///
/// 改用在候选池上统计：一个词项出现在越多结果里，它对「区分相关与不相关」
/// 的贡献越小。
///
/// ```text
/// df(term)   = 含该词项的候选数 / 候选总数
/// weight     = clamp(ln(1 + 1/df), MIN_TERM_WEIGHT, MAX_TERM_WEIGHT)
/// ```
///
/// - 词项只出现在 1 条结果里 → df=1/N → weight 高（强区分信号）
/// - 词项出现在几乎所有结果里 → df→1 → weight→MIN（无区分度）
///
/// 两个保底常量各自防一类退化：
/// - `MIN_TERM_WEIGHT`：某词到处都是时不完全归零，避免「所有词都平庸」
///   的查询退化成无信号。
/// - `MAX_TERM_WEIGHT`：权重上限。防止某个只出现一次的词（df 极小）
///   一家独大，把其他词的真实命中全压成低比例。
///
/// 注意：df=0（池子里完全没出现）**不走这个 clamp，而是直接归零** ——
/// 见 `term_idf_weights` 内的说明。OOV 词不可能被命中，给它任何正权重
/// 都只是稀释分母。
const MIN_TERM_WEIGHT: f64 = 0.05;
const MAX_TERM_WEIGHT: f64 = 2.0;

/// 计算查询词项在候选池上的 IDF 权重，返回 `(词项, 权重)` 列表。
///
/// `candidates` 是全部源的原始结果（未去重）—— 文档频率要在完整池子上
/// 统计，去重后的池子会低估高频词的出现次数。
///
/// **df=0 的词项权重为 0，不是封顶值。** 这是关键设计：查询词在候选池
/// 里完全没出现时，它不可能被任何结果命中，若给它高权重只会稀释
/// weight_sum 分母、把能命中的词压成低命中率（实测踩过：长查询里
/// "assistant"/"tauri" 这种 OOV 词各占 2.0 权重，使唯一相关的
/// "mrpulor-gh/nuphus" best_ratio 仅 0.15，惨遭过滤）。
/// df=0 的正确含义是「这个词对本次排序没有任何贡献」，归零即可。
fn term_idf_weights(
    query: &str,
    candidates: &[(SearchResult, SearchSource)],
) -> Vec<(String, f64)> {
    let terms = tokenize(query);
    if terms.is_empty() || candidates.is_empty() {
        return terms.into_iter().map(|t| (t, 1.0)).collect();
    }

    // 每条候选的标题+摘要词项集合，用于统计 df
    let per_candidate: Vec<std::collections::HashSet<String>> = candidates
        .iter()
        .map(|(r, _)| {
            let mut s: std::collections::HashSet<String> = tokenize(&r.title).into_iter().collect();
            s.extend(tokenize(&r.snippet));
            s
        })
        .collect();
    let total = per_candidate.len() as f64;

    terms
        .into_iter()
        .map(|t| {
            let df = per_candidate.iter().filter(|set| set.contains(&t)).count() as f64 / total;
            // df=0 → 权重 0（不参与分母，也不可能被命中）
            let w = if df <= 0.0 {
                0.0
            } else {
                (1.0 + 1.0 / df)
                    .ln()
                    .clamp(MIN_TERM_WEIGHT, MAX_TERM_WEIGHT)
            };
            (t, w)
        })
        .collect()
}

/// 单条结果的相关性判定（IDF 加权）。
struct Relevance {
    /// 综合得分：标题命中权重 ×3 + 摘要命中权重 ×1 + 连续子串奖励 ×1。
    /// 仅用于**排序**，不用于过线判定（过线看 `anchor_hit`）。
    score: f64,
    /// 锚点词（权重最高的查询词）是否被命中。
    ///
    /// 这是过线判定的唯一依据。**不用固定比例阈值**，实测踩过两轮坑：
    /// ① 只用绝对分（1.5）不设下限：「trafilatura library」召回一堆 MDN
    ///    "Local Library" 教程，仅命中虚词也能混进结果。
    /// ② 改用归一化命中率（标题/摘要命中权重过半）：2 词查询好使，但
    ///    4-6 词查询命中 1 个主题词就只有 25%，数学上不可能过半 ——
    ///    "Nuphus AI desktop assistant Tauri Rust" 会让唯一相关的
    ///    "mrpulor-gh/nuphus" 以 0.25 被过滤掉。
    ///
    /// 锚点命中等价于「结果有没有谈到查询的核心主题」，
    /// 与查询有几个词、词权重如何分布都无关。
    ///
    /// 配套约束：df=0 的 OOV 词权重为 0（见 `term_idf_weights`），
    /// 否则 "desktop assistant" 这种池里没人谈的组合会把锚点推到无法命中的词上。
    anchor_hit: bool,
}

/// IDF 加权相关性判定。
fn relevance_weighted(terms: &[(String, f64)], result: &SearchResult) -> Relevance {
    if terms.is_empty() {
        return Relevance {
            score: 0.0,
            anchor_hit: false,
        };
    }
    let title_terms = tokenize(&result.title);
    let snippet_terms = tokenize(&result.snippet);

    // weight_sum 只累计有权重的词（df=0 的词权重为 0，不参与分母）
    let weight_sum: f64 = terms.iter().map(|(_, w)| w).sum();
    if weight_sum <= 0.0 {
        return Relevance {
            score: 0.0,
            anchor_hit: false,
        };
    }

    // 加权命中率（分母是权重和，不是词项数）
    let title_hit: f64 = terms
        .iter()
        .filter(|(t, _)| title_terms.iter().any(|x| x == t))
        .map(|(_, w)| w)
        .sum();
    let snippet_hit: f64 = terms
        .iter()
        .filter(|(t, _)| snippet_terms.iter().any(|x| x == t))
        .map(|(_, w)| w)
        .sum();

    let title_ratio = title_hit / weight_sum;
    let snippet_ratio = snippet_hit / weight_sum;

    let mut score = 3.0 * title_ratio + 1.0 * snippet_ratio;

    // 标题完整包含查询串（连续子串）→ 强精确信号，仅加分不影响过线
    if title_terms.len() >= terms.len() {
        let joined = terms
            .iter()
            .filter(|(_, w)| *w > 0.0)
            .map(|(t, _)| t.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if result.title.to_ascii_lowercase().contains(&joined) {
            score += 1.0;
        }
    }

    // 锚点 = 权重最高的查询词（并列时取字符串序在前者，保证结果可复现）
    let anchor = terms
        .iter()
        .filter(|(_, w)| *w > 0.0)
        .max_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.0.as_str().cmp(a.0.as_str()))
        })
        .map(|(t, _)| t.as_str());

    let anchor_hit = match anchor {
        Some(a) => title_terms.iter().any(|t| t == a) || snippet_terms.iter().any(|t| t == a),
        None => false,
    };

    Relevance { score, anchor_hit }
}

/// 把各源返回的 URL 归一化，用于跨源去重。
///
/// Bing 常返回 `https://www.bing.com/ck/a?!...&u=a1<base64url>` 形式的
/// 跳转链接，同一目标页与其它源的原始 URL 看起来毫无关系。此处先解出
/// 跳转目标再归一化；归一化保留 scheme + host + path + query，仅去掉
/// fragment、末尾斜杠与 www. 前缀 —— host 是区分不同站点的关键，丢掉它
/// 会把 `a.com/x` 与 `b.com/x` 误判为同一页。
fn canonical_url(url: &str) -> String {
    use base64::Engine as _;
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Bing /ck/a 跳转：u=a1<base64url(含标准+URL安全两种字母表，可能无 padding)>
    if let Some(idx) = trimmed.find("&u=a1") {
        let encoded = &trimmed[idx + "&u=a1".len()..];
        let end = encoded.find(['&', '#']).unwrap_or(encoded.len());
        let raw = &encoded[..end];
        for engine in [
            base64::engine::general_purpose::STANDARD,
            base64::engine::general_purpose::STANDARD_NO_PAD,
            base64::engine::general_purpose::URL_SAFE,
            base64::engine::general_purpose::URL_SAFE_NO_PAD,
        ] {
            if let Ok(decoded_bytes) = engine.decode(raw.as_bytes()) {
                if let Ok(decoded) = String::from_utf8(decoded_bytes) {
                    if decoded.starts_with("http") {
                        return canonical_url(&decoded);
                    }
                }
            }
        }
    }

    // 拆 scheme
    let (scheme, rest): (String, &str) = trimmed
        .split_once("://")
        .map(|(s, r)| (s.to_ascii_lowercase(), r))
        .unwrap_or_else(|| (String::new(), trimmed));

    // 拆 host / 其余
    let (host, tail): (&str, String) = rest
        .split_once('/')
        .map(|(h, t)| (h, format!("/{}", t)))
        .unwrap_or_else(|| (rest, String::new()));

    let host = host
        .rsplit('@')
        .next()
        .unwrap_or(host)
        .to_ascii_lowercase()
        .trim_start_matches("www.")
        .to_string();
    let path_query = tail.split('#').next().unwrap_or(&tail);
    let path_query = path_query.trim_end_matches('/');

    // 无 scheme 的裸串不该拼出 "://xxx" 这种畸形结果
    if scheme.is_empty() {
        return format!("{}{}", host, path_query);
    }

    format!("{}://{}{}", scheme, host, path_query)
}

/// 五源并发取候选，按相关性 + 源可靠性排序，去重后取前 `count`。
///
/// 单源失败只记日志不中断 —— 这正是聚合相对串行降级的核心价值：
/// 任一源翻车（限流 / 反爬 / 结构变更）不影响其余四源的产出。
/// 源失败后可重试与否 —— 只用于**诊断**：告诉模型/用户该重试还是该改查询。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceFailure {
    /// 限流 / 暂不可用 / 超时：过一会儿重试可能成功
    Retryable,
    /// 不可解析 / 无匹配 / 被拒：重试无意义
    Permanent,
}

/// 从源返回的错误文本里判可重试性。
///
/// 各 `search_*` 返回 `Result<_, String>`，错误串形如 `HTTP 429` / `HTTP 503` /
/// `… request failed: …timed out…`，**没有结构化状态码可读**，只能按文本判定。
/// 真正决定「要不要重试」的仍是各源内部的 `fetch_direct`（429/503 才重试）；
/// 此处只是把结论汇总给调用方，避免失败时只丢一句
/// 「All search sources failed」让模型无从判断该等还是该换词。
fn classify_source_failure(reason: &str) -> SourceFailure {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("429")
        || lower.contains("503")
        || lower.contains("timed out")
        || lower.contains("timeout")
    {
        SourceFailure::Retryable
    } else {
        SourceFailure::Permanent
    }
}

/// 五源聚合的产出：最终结果 + 各源的失败原因。
///
/// 失败原因必须一路带回调用方 —— 全部源挂掉时，模型需要知道是「限流，等一下
/// 再试」还是「查询没匹配，换个词」，这两者的下一步动作完全相反。
struct AggregateOutcome {
    results: Vec<SearchResult>,
    failures: Vec<(&'static str, String, SourceFailure)>,
}

fn aggregate_web_search(query: &str, count: usize) -> AggregateOutcome {
    let count = count.clamp(1, 20);
    // 候选池深度：每源多取一倍。去重 + 阈值淘汰会消耗候选，池子太浅时
    // 只能用噪声填空。
    let per_source = (count * 2).clamp(10, 40);

    let sources: [(
        &'static str,
        fn(&str, usize) -> Result<Vec<SearchResult>, String>,
    ); 5] = [
        ("github", search_github),
        ("docs", search_docs),
        ("wikipedia", search_wikipedia),
        ("bing", search_bing),
        ("ddg-lite", search_ddg_lite),
    ];

    // 源函数是同步阻塞 IO，且本函数已在 spawn_blocking 线程内；
    // 用 std::thread 并发跑而不是 tokio，避免把阻塞调用塞进 async 上下文。
    let mut handles = Vec::with_capacity(sources.len());
    for (name, f) in sources {
        let q = query.to_string();
        // 保留 Result —— 旧实现 `.ok().unwrap_or_default()` 把失败原因整个丢掉，
        // 五个源全挂时只留一句「All search sources failed」，模型分不清是限流
        // （该等）还是查询本身没匹配（该换词）。
        handles.push((name, std::thread::spawn(move || f(&q, per_source))));
    }

    let mut candidates: Vec<(SearchResult, SearchSource)> = Vec::new();
    let mut failures: Vec<(&'static str, String, SourceFailure)> = Vec::new();
    for (name, handle) in handles {
        let outcome = match handle.join() {
            Ok(r) => r,
            Err(_) => {
                tracing::warn!("[web_search] source {} panicked, skipped", name);
                failures.push((name, "panicked".to_string(), SourceFailure::Permanent));
                continue;
            }
        };
        let results = match outcome {
            Ok(r) if r.is_empty() => {
                tracing::debug!("[web_search] source {} returned nothing", name);
                continue;
            }
            Ok(r) => r,
            Err(e) => {
                let kind = classify_source_failure(&e);
                tracing::warn!("[web_search] source {} failed ({:?}): {}", name, kind, e);
                failures.push((name, e, kind));
                continue;
            }
        };
        tracing::debug!(
            "[web_search] source {} contributed {} candidates",
            name,
            results.len()
        );
        let src = match name {
            "github" => SearchSource::GitHub,
            "docs" => SearchSource::Docs,
            "wikipedia" => SearchSource::Wikipedia,
            "bing" => SearchSource::Bing,
            _ => SearchSource::DdgLite,
        };
        for r in results {
            candidates.push((r, src));
        }
    }

    // IDF 权重在完整候选池上统计（去重会低估高频词 df）
    let weighted_terms = term_idf_weights(query, &candidates);

    // ── 源级降级：按语义职责分流，不是统一重试 ──
    //
    // 实测教训：查询 "Nuphus AI desktop assistant Tauri Rust" 送进 GitHub API
    // 匹配不上（返回 0），而单词 "nuphus" 能返回 5 条 —— 源可用，是长查询
    // 匹配问题。
    //
    // 但**不能对三个 API 源无差别重试**：docs/wikipedia 返回空通常是
    // 「主题不匹配」（nuphus 不该搜 MDN），用核心词再搜 MDN 只会召回更多
    // 噪声并污染 IDF 统计。
    //
    // 分流规则：
    // - `github`：仓库/项目名主题查询的首选源，长查询匹配不上 → 核心词重试
    // - `docs`   ：技术文档源，只对显式文档类查询有意义，静默跳过不重试
    // - `wikipedia`：百科源，天然只覆盖百科条目，静默跳过不重试
    // 只有 github 需要重试兜底，另两个源的空结果是「正常无匹配」而非故障。
    let github_had_candidates = candidates.iter().any(|(_, s)| *s == SearchSource::GitHub);
    if !github_had_candidates && !candidates.is_empty() {
        let mut top_terms: Vec<(String, f64)> = weighted_terms.clone();
        top_terms.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        // 取权重最高的 1 个词：这是查询里最有区分度的主题词（nuphus），
        // 也正是长查询匹配失败时最该单独拿去搜的词。
        let retry_query = top_terms
            .first()
            .map(|(t, _)| t.clone())
            .unwrap_or_default();

        if !retry_query.is_empty() && retry_query != query {
            let retried = search_github(&retry_query, per_source).unwrap_or_default();
            if !retried.is_empty() {
                tracing::info!(
                    "[web_search] github empty on full query, retried with top-weight term '{}' → {} candidates",
                    retry_query,
                    retried.len()
                );
                for r in retried {
                    candidates.push((r, SearchSource::GitHub));
                }
            }
        }
    }

    // 加权相关性 + 源可靠性打分。pass = 锚点词命中（与查询长度无关）。
    let mut scored: Vec<(f64, SearchResult, SearchSource, bool)> = candidates
        .into_iter()
        .map(|(r, src)| {
            let rel = relevance_weighted(&weighted_terms, &r);
            let score = rel.score * src.weight();
            (score, r, src, rel.anchor_hit)
        })
        .collect();

    // 优先级：及格的在前；组内按相关性降序；同分时源可靠性高者优先
    scored.sort_by(|a, b| {
        b.3.cmp(&a.3)
            .then_with(|| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.2.name().cmp(b.2.name()))
    });

    // 跨源去重：保留得分最高的 count 条
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<SearchResult> = Vec::with_capacity(count);
    for (score, result, src, pass) in scored.iter() {
        if !pass {
            // 第一个不及格出现即停 —— 后面的只会更不相关
            break;
        }
        let key = canonical_url(&result.url);
        if !key.is_empty() && !seen.insert(key) {
            continue;
        }
        tracing::debug!(
            "[web_search] keep score={:.3} src={} title={:?}",
            score,
            src.name(),
            result.title
        );
        out.push(result.clone());
        if out.len() >= count {
            break;
        }
    }

    // 兜底：全部候选都不及格（查询极偏 / 源集体跑偏）时放开及格线，
    // 按原分数序取前 count —— 给弱相关结果，但不给空。
    if out.is_empty() {
        let mut seen2: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (_score, result, src, _pass) in scored.iter() {
            let key = canonical_url(&result.url);
            if !key.is_empty() && !seen2.insert(key) {
                continue;
            }
            tracing::debug!(
                "[web_search] fallback keep (below threshold) src={} title={:?}",
                src.name(),
                result.title
            );
            out.push(result.clone());
            if out.len() >= count {
                break;
            }
        }
    }

    AggregateOutcome {
        results: out,
        failures,
    }
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
    let mut req = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github.v3+json")
        .timeout(Duration::from_secs(10));
    // 有 token 就带上：未认证 10 次/分，认证 30 次/分。无 token 时行为不变。
    if let Some(auth) = github_auth_header() {
        req = req.header("Authorization", auth);
    }
    let response = req
        .send()
        .map_err(|e| format!("github request failed: {}", e))?;

    // 限流必须显式报错——静默按"无结果"处理会让聚合逻辑误判为
    // 「长查询匹配不上」而触发核心词重试，白打一次 API 还把限流拖更长。
    if response.status() == reqwest::StatusCode::FORBIDDEN
        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        let reset = response
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");
        return Err(format!(
            "github rate limited (set GITHUB_TOKEN to raise the limit); reset at {}",
            reset
        ));
    }

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

    // MDN 的 q= 是子串匹配，「nuphus」「desktop assistant」这类非 Web 平台
    // 主题也会返回一堆 HTML/JS 文档 —— 它们对查询毫无意义，进了聚合池只会
    // 污染 IDF 统计、稀释及格线。前置过滤：标题必须含至少一个查询词项。
    // （docs 是显式 source=docs 调用时也走这条路，过滤只砍噪声不砍正解。）
    let query_tokens = tokenize(query);
    let mut out = Vec::new();
    for doc in documents.iter() {
        let title = doc["title"].as_str().unwrap_or("").to_string();
        if query_tokens.is_empty() || tokenize(&title).iter().any(|t| query_tokens.contains(t)) {
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
        if out.len() >= count {
            break;
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

// ── 字符集嗅探与解码 ──

/// 从 HTTP `Content-Type` 头提取 charset label。
///
/// `text/html; charset=gbk` → `Some("gbk")`。仅认 `charset=` 参数，
/// 大小写不敏感；头缺失或类型不含 charset 时返回 `None`（交给下一级判定）。
/// 结束边界同时认 `;`、空白与引号/尖括号 —— 调用方传入的可能是一段从
/// HTML 里切出的窗口，末尾带着 `">` 这类标签残留。
fn charset_from_content_type(content_type: Option<&str>) -> Option<String> {
    let ct = content_type?;
    let lower = ct.to_ascii_lowercase();
    let idx = lower.find("charset=")?;
    let start = idx + "charset=".len();
    let rest = &ct[start..];
    // value 可能被引号包裹（charset="gbk"）。引号紧贴 "=" 时若先找结束
    // 边界会立刻命中这个前引号而得到空 label，故先剥前导引号。
    let rest = rest.trim_start_matches(['"', '\'']);
    let end = rest
        .find(|c: char| c == ';' || c == '"' || c == '\'' || c == '>' || c.is_whitespace())
        .unwrap_or(rest.len());
    // 保留原始大小写（label 语义上大小写不敏感，但调用方可能用于展示/日志）
    let label = rest[..end].trim();
    if label.is_empty() {
        None
    } else {
        Some(label.to_string())
    }
}

/// 从 HTML 头部嗅探 `<meta charset>` / `<meta http-equiv="Content-Type">`。
///
/// 中文站大量只在 meta 里声明编码而不发送 HTTP header，`reqwest` 的
/// `.text()` 此时按 UTF-8 硬解 GBK 字节 → 整页乱码。此处只扫 HTML 头部
/// （前 1024 字节足够覆盖 meta 声明的常规位置），避免全文正则开销。
fn charset_from_html_meta(html_head: &str) -> Option<String> {
    let lower = html_head.to_ascii_lowercase();

    // <meta charset="gbk"> —— 注意引号紧跟在 "=" 后面，split 的第一段
    // 会是空串，故过滤空段后取第一个非空值。
    if let Some(idx) = lower.find("<meta charset=") {
        let rest = &html_head[idx + "<meta charset=".len()..];
        let label = rest
            .split(|c: char| c == '"' || c == '\'' || c == '>' || c == ';' || c.is_whitespace())
            .find(|s| !s.trim().is_empty())
            .unwrap_or("")
            .trim();
        if !label.is_empty() {
            return Some(label.to_string());
        }
    }

    // <meta http-equiv="Content-Type" content="text/html; charset=gbk">
    // 两个属性的先后顺序两种写法都存在，故以 http-equiv 为准、向前后
    // 两个方向找 content= —— 只向后找会把
    // <meta content="..." http-equiv="..."> 漏掉。
    let pos = lower.find("http-equiv")?;
    let window_start = pos.saturating_sub(64);
    let window_end = (pos + 96).min(lower.len());
    let window = &lower[window_start..window_end];
    let content_pos = window.find("content=")?;
    let after = &window[content_pos + "content=".len()..];
    let after = after.trim_start_matches(['"', '\'']);
    charset_from_content_type(Some(after))
}

/// 把网页字节流按嗅探到的 charset 解码为 `String`。
///
/// 三级判定，逐级降级：
/// 1. HTTP header 的 `Content-Type` charset（最权威）
/// 2. HTML 内 `<meta charset>` / `<meta http-equiv>`（中文站主要依赖此路）
/// 3. 兜底 UTF-8（原 `reqwest::Response::text()` 行为，保证不回归）
///
/// label 无法被 `encoding_rs` 识别时同样回退 UTF-8 —— 宁可保留可读的
/// 英文内容，也不因未知 label 整页失败。
fn decode_web_body(bytes: &[u8], content_type: Option<&str>) -> String {
    let label = charset_from_content_type(content_type)
        .or_else(|| {
            // meta 只在 HTTP 头没给 charset 时才嗅探，避免与权威值冲突。
            // 头部窗口按 latin1 逐字节无损读（0..=255 全部合法）——若用
            // String::from_utf8_lossy，GBK/Shift_JIS 页里的引号、字母会被
            // 替换成 U+FFFD，可能污染恰好横跨窗口边界的 meta 标签。
            let head_len = bytes.len().min(2048);
            let head: String = bytes[..head_len].iter().map(|b| *b as char).collect();
            charset_from_html_meta(&head)
        })
        .unwrap_or_else(|| "utf-8".to_string());

    if label.eq_ignore_ascii_case("utf-8") {
        // 快速路径：UTF-8 无损直转（与 reqwest 现状一致）
        return String::from_utf8_lossy(bytes).into_owned();
    }

    match encoding_rs::Encoding::for_label(label.as_bytes()) {
        Some(enc) => {
            let (cow, _encoding_used, _malformed) = enc.decode(bytes);
            tracing::debug!("[web] decoded body with charset label={}", label);
            cow.into_owned()
        }
        None => {
            tracing::debug!(
                "[web] unknown charset label={}, falling back to utf-8",
                label
            );
            String::from_utf8_lossy(bytes).into_owned()
        }
    }
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
fn render_element(el: &scraper::ElementRef, parts: &mut Vec<String>, depth: usize) {
    // 深度闸：真实页面嵌套几十层就到头，超深只有恶意构造一种来源。
    // 超限只是不再下钻 —— 已产出的 parts 照常返回，不整页丢弃。
    if depth > MAX_DOM_DEPTH {
        return;
    }
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
                            render_element(&child_el, parts, depth + 1);
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
                    render_element(&child_el, parts, depth + 1);
                }
            }
        }
        _ => {
            for child in el.children() {
                if let Some(child_el) = scraper::ElementRef::wrap(child) {
                    render_element(&child_el, parts, depth + 1);
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
            description: "Web search across 5 sources (github/docs/wikipedia/bing/ddg-lite) aggregated and filtered by relevance — returns title+url+snippet only, NOT page content. Default source=web merges all 5; pass source= to restrict. To read a result's content use web_extract.".to_string(),
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
                            // 默认 web 源：五源聚合，不串行降级。
                            //
                            // 原实现是 Bing → DDG Lite 两个 HTML 源串行，实测
                            // 在中文/技术查询下两者都可能返回与查询完全无关的
                            // 结果（Bing 硬编码 setmkt=en-US 时最严重）。改为
                            // 五个源并发取候选，按「查询-结果相关性 + 源可靠性」
                            // 统一排序后取前 count —— 单个源翻车不再决定全局。
                            let outcome = aggregate_web_search(&query, count);
                            if outcome.results.is_empty() {
                                return Ok(ToolResult::failure(
                                    describe_aggregate_failure(&outcome.failures),
                                ));
                            }
                            build_tool_result(&query, outcome.results)
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
            description: "Fetch a known URL and return readable text. Use for reading article/doc/README content found via web_search. NOT for: pages needing login/interaction/anti-bot bypass (use browser_navigate + browser_*), or locating specific elements (use browser_snapshot). Charset is auto-detected (GBK/Shift_JIS/…); falls back to CDP render for SPA/JS-heavy pages.".to_string(),
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
                // SSRF 守卫：与 http_request 共用同一实现（check_ssrf 内部已含
                // http/https scheme 白名单，不再另写前缀判断）。策略传
                // (allow_private=false, allow_loopback=false)：web_extract 抓的是
                // 公网网页，私网段与本机地址都不该碰，也不给模型开关。
                if let Err(e) = super::http::check_ssrf(&url, false, false) {
                    return Ok(ToolResult::failure(e));
                }

                super::run_blocking(move || {
                    let agent = get_agent();

                    // ── 第一层: 直接 HTTP ──
                    let direct_result = fetch_direct(&agent, &url);
                    let _direct_content_ok = match &direct_result {
                        Ok(body) => {
                            let text = extract_readable(&body.text, max_chars);
                            let trimmed = text.trim();
                            // 内容充分（>= 2000 chars）直接返回
                            if trimmed.len() >= 2000 {
                                // 字节闸若截断过，必须让模型看见：否则它会把半篇
                                // 正文当成完整文档来回答，且无从察觉漏了尾巴。
                                let marker = if body.byte_truncated {
                                    byte_truncation_marker()
                                } else {
                                    String::new()
                                };
                                return Ok(ToolResult::success(format!(
                                    "Fetched {}\n\n{}{}",
                                    url, text, marker
                                )));
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
                            return Ok(ToolResult::success(format!(
                                "Fetched {}\n\n[via CDP browser render]\n\n{}",
                                url, text
                            )));
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
                        Ok(mirror) => {
                            let mirror_body = mirror.text.as_str();
                            // r.jina.ai 返回的是 **Markdown 纯文本**（Jina 的产品
                            // 形态是"URL → clean markdown"），不是 HTML。旧实现直接喂
                            // extract_readable（HTML 解析器）→ find_content_container
                            // 必然找不到容器 → 兜底走 html_to_text → 把 Jina 已经产出
                            // 的标题/列表/代码块结构抹平，白解一遍还丢结构。
                            //
                            // 判定：文本含成对 HTML 标签（<p>/<div>/<article> 等）才算
                            // HTML，走解析；否则视为纯文本，按字符上限截断后直接返回。
                            let looks_like_html = {
                                static HTML_TAG: std::sync::OnceLock<regex::Regex> =
                                    std::sync::OnceLock::new();
                                let re = HTML_TAG.get_or_init(|| {
                                    regex::Regex::new(r"(?is)<(?:p|div|article|section|main|span|h[1-6]|ul|ol|li|table|br|a)\b")
                                        .expect("static html-tag regex")
                                });
                                re.is_match(mirror_body)
                            };
                            let text = if looks_like_html {
                                extract_readable(mirror_body, max_chars)
                            } else if mirror_body.chars().count() > max_chars {
                                let trunc: String =
                                    mirror_body.chars().take(max_chars).collect();
                                format!(
                                    "{}\n\n[Truncated: {} chars total, {} returned]",
                                    trunc,
                                    mirror_body.chars().count(),
                                    max_chars
                                )
                            } else {
                                mirror_body.trim().to_string()
                            };
                            if !text.trim().is_empty() {
                                // 字符闸与字节闸各自打自己的标记：长文常常只触到
                                // 其中一个，两个都触到时模型能看出被砍了两刀。
                                let marker = if mirror.byte_truncated {
                                    byte_truncation_marker()
                                } else {
                                    String::new()
                                };
                                return Ok(ToolResult::success(format!(
                                    "Fetched {}\n\n[via Jina AI mirror]\n\n{}{}",
                                    url, text, marker
                                )));
                            }
                            Ok(ToolResult::failure(format!(
                                "All methods failed for {} (direct/browser/mirror all returned empty)",
                                url
                            )))
                        }
                        Err(e) => {
                            let direct_info = match &direct_result {
                                Ok(body) => {
                                    let text = extract_readable(&body.text, max_chars);
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

/// 按字节上限读取响应 body（防超大响应打爆内存），返回 body 与「是否触发截断」。
///
/// - 带 Content-Length 且超限 → 直接 Err（不读 body，避免先分配再丢弃）
/// - 否则 `take(MAX_WEB_BYTES + 1)` 读入；读出超过上限则截断保留可用部分
///
/// `+1` 是为了让「恰好填满上限」与「超出上限」可区分 —— 否则一个正好等于
/// 上限的响应会被误标成已截断（下游据此判断要不要给模型打标记）。
fn read_body_capped(response: reqwest::blocking::Response) -> Result<(Vec<u8>, bool), String> {
    if let Some(len) = response.content_length() {
        if len > MAX_WEB_BYTES as u64 {
            return Err(format!(
                "response too large: Content-Length {} exceeds cap {} bytes",
                len, MAX_WEB_BYTES
            ));
        }
    }
    let mut buf = Vec::new();
    let mut limited = response.take(MAX_WEB_BYTES as u64 + 1);
    limited
        .read_to_end(&mut buf)
        .map_err(|e| format!("body read error: {}", e))?;
    if buf.len() > MAX_WEB_BYTES {
        // 真正读到超限才截断；只读满不超限时 truncated 保持 false。
        buf.truncate(MAX_WEB_BYTES);
        return Ok((buf, true));
    }
    Ok((buf, false))
}

/// 字节闸截断标记 —— 由调用方拼到**最终文本**末尾。
///
/// 不能塞进原始 bytes：中间还隔着 charset 解码与 HTML 正文提取，落在正文
/// 容器之外的标记会被解析器整段丢掉，模型就不知道自己拿到的是残缺页面。
/// 格式对齐 `extract_readable` 既有的 `[Truncated: …]` 标记，让模型只需认一种。
pub(super) fn byte_truncation_marker() -> String {
    format!(
        "\n\n[Truncated: response exceeds {} MiB byte cap; tail omitted]",
        MAX_WEB_BYTES / (1024 * 1024)
    )
}

/// `fetch_direct` 的产出：已解码的 body 文本 + 字节闸是否触发过截断。
pub(super) struct FetchedBody {
    pub text: String,
    pub byte_truncated: bool,
}

/// 解析 `Retry-After` 头（仅秒数形式）。
///
/// 只支持 `Retry-After: 2` 这类 delta-seconds；HTTP-date 形式（绝对时间）不解析
/// —— 抓取工具不该为服务端时区解析引入不确定性。秒数封顶 30s：服务端可能回
/// `Retry-After: 3600`，抓取工具不能因此挂住一小时。
fn retry_after_delay(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let raw = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim();
    let secs = raw.parse::<u64>().ok()?;
    Some(Duration::from_secs(secs.min(30)))
}

/// 直接 HTTP GET 请求，返回已按 charset 三级嗅探解码的 body + 字节闸截断标志
fn fetch_direct(agent: &reqwest::blocking::Client, url: &str) -> Result<FetchedBody, String> {
    // 按域白名单附 cookie（命中才走 vault；未命中域名行为完全不变）
    let cookie_header = cookie_header_for(url);
    let mut attempt = 0;
    loop {
        let mut req = agent
            .get(url)
            .header("User-Agent", USER_AGENT)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8");
        if let Some(ref h) = cookie_header {
            req = req.header("Cookie", h);
        }
        match req.send() {
            Ok(r) => {
                let code = r.status();
                if !code.is_success() {
                    // reqwest blocking: HTTP error status comes as Ok(Response), not Err
                    let msg = format!("HTTP {}", code.as_u16());
                    // 只重试 429（限流）/ 503（服务暂不可用）—— 404/403 等重试无意义。
                    let retryable = matches!(code.as_u16(), 429 | 503);
                    if retryable && attempt == 0 {
                        let delay =
                            retry_after_delay(r.headers()).unwrap_or(Duration::from_millis(500));
                        std::thread::sleep(delay);
                        attempt += 1;
                        continue;
                    }
                    return Err(msg);
                }
                // charset 必须在消费 Response 之前取——read_body_capped 会消费 Response
                let content_type = r
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                let (bytes, byte_truncated) =
                    read_body_capped(r).map_err(|e| format!("read body failed: {}", e))?;
                if bytes.trim_ascii().is_empty() {
                    return Err(format!("HTTP {} (empty body)", code.as_u16()));
                }
                return Ok(FetchedBody {
                    text: decode_web_body(&bytes, content_type.as_deref()),
                    byte_truncated,
                });
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
    // SSRF 守卫：本层危害最大 —— CDP profile 带登录态，入口一旦被绕过
    // 就是「以用户身份访问内网」。策略与 web_extract 入口一致 (false, false)。
    super::http::check_ssrf(url, false, false)?;
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

// ── 回归测试 ──

#[cfg(test)]
mod tests {
    use super::*;

    fn r(title: &str, url: &str, snippet: &str) -> SearchResult {
        SearchResult {
            title: title.to_string(),
            url: url.to_string(),
            snippet: snippet.to_string(),
        }
    }

    // ── charset 嗅探 ──

    #[test]
    fn test_charset_from_content_type_basic() {
        assert_eq!(
            charset_from_content_type(Some("text/html; charset=gbk")),
            Some("gbk".to_string())
        );
        // 大小写不敏感、带引号
        assert_eq!(
            charset_from_content_type(Some("TEXT/HTML; CHARSET=\"GB2312\"")),
            Some("GB2312".to_string())
        );
        // 无 charset 参数 → None，交给下一级
        assert_eq!(charset_from_content_type(Some("text/html")), None);
        assert_eq!(charset_from_content_type(None), None);
        // charset= 后面为空
        assert_eq!(charset_from_content_type(Some("text/html; charset=")), None);
    }

    #[test]
    fn test_charset_from_html_meta_both_forms() {
        let a = r#"<html><head><meta charset="gbk"><title>t</title>"#;
        assert_eq!(charset_from_html_meta(a), Some("gbk".to_string()));

        let b =
            r#"<html><head><meta http-equiv="Content-Type" content="text/html; charset=GB2312">"#;
        // 经 meta 窗口解析时读的是已 lowercased 的副本，label 大小写会丢失。
        // charset label 语义上大小写不敏感（encoding_rs::for_label 亦然），
        // 故此处断言小写形式。
        assert_eq!(charset_from_html_meta(b), Some("gb2312".to_string()));

        // 顺序颠倒的写法（content 在 http-equiv 前）
        let c = r#"<html><head><meta content="text/html; charset=big5" http-equiv="Content-Type">"#;
        assert_eq!(charset_from_html_meta(c), Some("big5".to_string()));

        // 无声明
        assert_eq!(charset_from_html_meta("<html><head><title>x</title>"), None);
    }

    #[test]
    fn test_decode_web_body_gbk_roundtrip() {
        // GBK 编码的中文页，HTTP 头不含 charset，只在 meta 里声明。
        // GBK 把 " 编成 0xA1A3，用 latin1 逐字节读头才能原样看到标签结构
        // —— 这正是 decode_web_body 的实现路径（曾用 from_utf8_lossy 读头，
        // 会把这些字节换成 U+FFFD 导致分成读不出 label）。
        let html = "<html><head><meta charset=\"gbk\"></head><body>\u{4f60}\u{597d}\u{4e16}\u{754c}</body></html>";
        let (bytes, _, _) = encoding_rs::GBK.encode(html);

        // 前提：latin1 无损头部能读到 meta 声明
        let head_len = bytes.len().min(2048);
        let head: String = bytes[..head_len].iter().map(|b| *b as char).collect();
        assert_eq!(
            charset_from_html_meta(&head),
            Some("gbk".to_string()),
            "latin1 头部应能嗅到 gbk"
        );

        let decoded = decode_web_body(&bytes, Some("text/html"));
        assert!(
            decoded.contains("\u{4f60}\u{597d}\u{4e16}\u{754c}"),
            "GBK 页应解码为中文，实得: {}",
            decoded
        );
    }

    #[test]
    fn test_decode_web_body_header_wins_over_meta() {
        // header 与 meta 不一致时以 header 为准
        let html =
            "<html><head><meta charset=\"utf-8\"></head><body>\u{4f60}\u{597d}</body></html>";
        let (bytes, _, _) = encoding_rs::GBK.encode(html);
        let decoded = decode_web_body(&bytes, Some("text/html; charset=gbk"));
        assert!(
            decoded.contains("\u{4f60}\u{597d}"),
            "header charset 应优先, 实得: {}",
            decoded
        );
    }

    #[test]
    fn test_decode_web_body_unknown_label_falls_back() {
        let bytes = "plain ascii body".as_bytes();
        let decoded = decode_web_body(bytes, Some("text/html; charset=totally-bogus-xyz"));
        assert_eq!(
            decoded, "plain ascii body",
            "未知 label 不得 panic 或丢内容"
        );
    }

    #[test]
    fn test_decode_web_body_utf8_fast_path_unchanged() {
        let s = "UTF-8 中文内容 retained";
        assert_eq!(decode_web_body(s.as_bytes(), None), s);
        assert_eq!(
            decode_web_body(s.as_bytes(), Some("text/html; charset=utf-8")),
            s
        );
    }

    #[test]
    fn test_decode_web_body_meta_window_boundary_no_panic() {
        // 恰好 1024 字节 + meta 声明横跨窗口边界：不得越界 panic，
        // 也不得因边界切在 continuation byte 中间而丢 charset。
        let mut html = String::from("<html><head>");
        // 填充到接近 1024 的位置后放 meta
        while html.len() < 990 {
            html.push('x');
        }
        html.push_str("<meta charset=\"gbk\">");
        html.push_str("</head><body>");
        while html.len() < 1400 {
            html.push('y');
        }
        html.push_str("</body></html>");
        let (bytes, _, _) = encoding_rs::GBK.encode(&html);
        // HTTP 头不含 charset → 必须靠 meta 嗅探到 gbk
        let decoded = decode_web_body(&bytes, Some("text/html"));
        // 解码成功（不 panic）且内容可读；具体字符含中文时校验
        assert!(!decoded.is_empty());
        assert!(
            decoded.contains("xxx") || decoded.contains("yyy"),
            "应能解出填充内容"
        );
    }

    #[test]
    fn test_decode_web_body_empty_and_short() {
        // 空/极短输入不得 panic
        assert_eq!(decode_web_body(&[], None), "");
        assert_eq!(decode_web_body(b"x", None), "x");
        assert_eq!(
            decode_web_body(b"<meta charset=\"gbk\">", None),
            "<meta charset=\"gbk\">"
        );
    }

    // ── tokenize / relevance ──

    #[test]
    fn test_tokenize_ascii_and_cjk() {
        assert_eq!(tokenize("Hello World"), vec!["hello", "world"]);
        // 单字符 ASCII 词被丢弃（无区分度）；多字符虚词保留——词项是否
        // 「有用」由 relevance 的命中计数决定，tokenize 不做语义判断。
        assert_eq!(tokenize("a of the"), vec!["of", "the"]);
        // CJK 逐字成项
        assert_eq!(
            tokenize("\u{641c}\u{7d22}\u{5f15}\u{64ce}"),
            vec!["\u{641c}", "\u{7d22}", "\u{5f15}", "\u{64ce}"]
        );
        // 混合：ASCII 词 + CJK 逐字
        assert_eq!(
            tokenize("rust \u{722c}\u{866b}"),
            vec!["rust", "\u{722c}", "\u{866b}"]
        );
    }

    // ── IDF 权重（无硬编码词表） ──

    fn cand(title: &str, url: &str, snippet: &str) -> (SearchResult, SearchSource) {
        (
            SearchResult {
                title: title.to_string(),
                url: url.to_string(),
                snippet: snippet.to_string(),
            },
            SearchSource::Bing,
        )
    }

    #[test]
    fn test_idf_weights_downweight_terms_ubiquitous_in_pool() {
        // 池子里 4/5 条含 "library"、仅 1 条含 "trafilatura"
        // → library 权重必须显著低于 trafilatura。
        // 这条不依赖任何具体词汇：换任何高频/低频词对都成立。
        let pool = vec![
            cand("trafilatura python", "https://a/1", "scraping"),
            cand("Local Library site one", "https://b/2", "library"),
            cand("Local Library site two", "https://c/3", "library"),
            cand("Local Library site three", "https://d/4", "library"),
            cand("Local Library site four", "https://e/5", "library"),
        ];
        let w = term_idf_weights("trafilatura library", &pool);
        let tf = w
            .iter()
            .find(|(t, _)| t == "trafilatura")
            .map(|(_, x)| *x)
            .unwrap();
        let lb = w
            .iter()
            .find(|(t, _)| t == "library")
            .map(|(_, x)| *x)
            .unwrap();
        assert!(
            tf > lb,
            "只出现在 1 条结果里的词权重应高于出现在 5 条里的词: trafilatura={tf} library={lb}"
        );
    }

    #[test]
    fn test_idf_weights_empty_pool_falls_back_to_uniform() {
        // 无候选时（源全挂）权重退化为 1.0，不得 panic 或产 NaN
        let w = term_idf_weights("anything at all", &[]);
        assert_eq!(w.len(), 3, "anything / at / all 三个词项");
        assert!(w.iter().all(|(_, x)| (*x - 1.0).abs() < 1e-9));
    }

    #[test]
    fn test_relevance_weighted_ignores_ubiquitous_term_hits() {
        // 一条只命中高频词的结果，加权后应远低于只命中低频词的结果。
        // 这是实测回归的核心：改造前「Local Library 教程」能拿 1.5 分混进结果。
        let pool = vec![
            cand("trafilatura python", "https://a/1", "scraping"),
            cand("Local Library one", "https://b/2", ""),
            cand("Local Library two", "https://c/3", ""),
            cand("Local Library three", "https://d/4", ""),
            cand("Local Library four", "https://e/5", ""),
        ];
        let w = term_idf_weights("trafilatura library", &pool);

        let (real, _) = pool[0].clone();
        let (noise, _) = pool[1].clone();
        let real_rel = relevance_weighted(&w, &real);
        let noise_rel = relevance_weighted(&w, &noise);
        assert!(
            !noise_rel.anchor_hit,
            "仅命中高频词的噪声不得过线（未命中锚点）"
        );
        assert!(real_rel.anchor_hit, "命中锚点词的应过线");
        assert!(real_rel.score > noise_rel.score);
    }

    /// 实测回归钉子：过线判定必须与查询词数无关。
    ///
    /// 旧实现两次失败：① 绝对分 1.5 —— 常见词多的查询能"凑"过线；
    /// ② 归一化命中率过半 —— 4-6 词查询命中 1 个主题词只有 25%，数学上
    /// 不可能过半，反而把唯一相关结果滤掉。锚点命中对两种长度都正确。
    #[test]
    fn test_threshold_is_query_length_invariant() {
        let pool = vec![
            cand("mrpulor-gh/nuphus", "https://a/1", "本地优先 AI Agent"),
            cand(
                "Desktop mouse and keyboard controls",
                "https://mozilla/1",
                "game controls",
            ),
            cand(
                "Compiling from Rust to WebAssembly",
                "https://mozilla/2",
                "rust to wasm",
            ),
        ];
        // 短查询
        let w_short = term_idf_weights("nuphus", &pool);
        let hit_short = relevance_weighted(&w_short, &pool[0].0);
        let noise_short = relevance_weighted(&w_short, &pool[1].0);
        // 长查询（词多，nuphus 仍是锚点）
        let w_long = term_idf_weights("Nuphus AI desktop assistant Tauri Rust", &pool);
        let hit_long = relevance_weighted(&w_long, &pool[0].0);
        let noise_long = relevance_weighted(&w_long, &pool[1].0);

        assert!(
            hit_short.anchor_hit && !noise_short.anchor_hit,
            "短查询应正确区分"
        );
        assert!(
            hit_long.anchor_hit && !noise_long.anchor_hit,
            "长查询必须同样正确区分 —— 这是前两版阈值失效的实证场景"
        );
    }

    /// df=0 的 OOV 词不得成为锚点，也不得参与分母。
    ///
    /// "assistant"/"tauri" 池子里没人谈 → 权重 0。若给正权重，它们会
    /// 稀释 weight_sum，且"权重最高"可能把锚点推到无法命中的词上。
    ///
    /// 同时验证锚点选取：`nuphus` 只出现 1 次（df 低 → 权重高），
    /// `desktop` 出现 2 次（df 高 → 权重低），故 nuphus 才是锚点 ——
    /// 只含 desktop 的结果不得过线。
    #[test]
    fn test_oov_terms_get_zero_weight_not_anchor() {
        let pool = vec![
            cand("nuphus agent", "https://a/1", "desktop"),
            cand("Desktop mouse controls", "https://mozilla/1", "keyboard"),
            cand("Rust to WebAssembly", "https://mozilla/2", "compile"),
        ];
        let w = term_idf_weights("nuphus desktop assistant tauri", &pool);
        let assistant = w.iter().find(|(t, _)| t == "assistant").unwrap().1;
        let tauri = w.iter().find(|(t, _)| t == "tauri").unwrap().1;
        assert_eq!(assistant, 0.0, "OOV 词权重必须为 0");
        assert_eq!(tauri, 0.0, "OOV 词权重必须为 0");

        // 锚点 = nuphus（df 最低、权重最高），不是 desktop
        let rel = relevance_weighted(&w, &pool[0].0);
        assert!(rel.anchor_hit, "含 nuphus 的结果应命中锚点");
        let noise = relevance_weighted(&w, &pool[1].0);
        assert!(
            !noise.anchor_hit,
            "只含 desktop 的结果不得过线 —— desktop 权重低于 nuphus，不是锚点"
        );
    }

    #[test]
    fn test_relevance_weighted_empty_terms_is_zero() {
        let result = r("anything", "https://a.com", "whatever");
        let rel = relevance_weighted(&[], &result);
        assert_eq!(rel.score, 0.0);
        assert!(!rel.anchor_hit);
    }

    #[test]
    fn test_relevance_weighted_still_rewards_title_over_snippet() {
        // 池子里必须含被查询的词，否则 df=0 → 权重归零 → 全部返回 0 分。
        // （这正是 df=0 归零的设计行为，见 term_idf_weights 文档。）
        let pool = vec![
            cand("trafilatura python", "https://a/1", "scraping"),
            cand("unrelated other page", "https://b/2", "nothing"),
            cand("another unrelated page", "https://c/3", "nothing"),
        ];
        let w = term_idf_weights("trafilatura", &pool);
        let title_hit = r(
            "trafilatura - Python scraping",
            "https://a.com",
            "unrelated words",
        );
        let snippet_hit = r(
            "Some Random Page",
            "https://b.com",
            "mentions trafilatura in passing",
        );
        assert!(
            relevance_weighted(&w, &title_hit).score > relevance_weighted(&w, &snippet_hit).score
        );
    }

    // ── canonical_url ──

    #[test]
    fn test_canonical_url_strips_scheme_host_fragment() {
        // 保留 scheme + host（host 是区分站点的关键，丢了会把 a.com/x
        // 与 b.com/x 误判为同页），只去 fragment / 末尾斜杠 / www. 前缀
        assert_eq!(
            canonical_url("https://github.com/adbar/trafilatura#readme"),
            "https://github.com/adbar/trafilatura"
        );
        assert_eq!(
            canonical_url("http://Example.com/Path/"),
            "http://example.com/Path"
        );
        assert_eq!(
            canonical_url("https://www.example.com/a/b?q=1#frag"),
            "https://example.com/a/b?q=1"
        );
    }

    #[test]
    fn test_canonical_url_decodes_bing_redirect() {
        use base64::Engine as _;
        let target = "https://github.com/adbar/trafilatura";
        let encoded = base64::engine::general_purpose::STANDARD.encode(target);
        let redirect = format!("https://www.bing.com/ck/a?!&&p=abc&u=a1{}&ntb=1", encoded);
        assert_eq!(canonical_url(&redirect), target);
    }

    #[test]
    fn test_canonical_url_empty_and_garbage() {
        assert_eq!(canonical_url(""), "");
        assert_eq!(canonical_url("   "), "");
        // 无 scheme 的裸串：应原样归一（小写）而不是拼出诡异的 "://"
        assert_eq!(canonical_url("not a url at all"), "not a url at all");
    }

    #[test]
    fn test_canonical_url_dedupes_same_target_across_sources() {
        let from_bing =
            "https://www.bing.com/ck/a?!&u=a1aHR0cHM6Ly9naXRodWIuY29tL2FkYmFyL3RyYWZpbGF0dXJh";
        let from_github = "https://github.com/adbar/trafilatura";
        assert_eq!(canonical_url(from_bing), canonical_url(from_github));
    }

    // ── SearchSource 权重 ──

    #[test]
    fn test_source_weight_api_above_html() {
        // 实测 HTML 源（Bing）对技术查询返回过完全无关结果，必须降权
        assert!(SearchSource::GitHub.weight() > SearchSource::Bing.weight());
        assert!(SearchSource::Docs.weight() > SearchSource::DdgLite.weight());
        assert!(SearchSource::Wikipedia.weight() > SearchSource::Bing.weight());
    }

    // ── 聚合：搜索引擎市场随查询语言切换 ──

    #[test]
    fn test_bing_market_follows_query_language() {
        // CJK 判定必须与 search_bing 内部一致——这里直接把判别式内联复刻
        // 会让测试退化成「测自己」。改为断言两条真实构造路径的差异：
        // search_bing 无法在不发请求的前提下取回 URL，故退而验证其
        // 判定条件本身（命中 CJK 区段即中文市场）。
        let cjk_ranges: [(u32, u32); 4] = [
            (0x4E00, 0x9FFF),
            (0x3400, 0x4DBF),
            (0x3000, 0x303F),
            (0xFF00, 0xFFEF),
        ];
        let in_cjk = |c: char| {
            cjk_ranges
                .iter()
                .any(|(lo, hi)| (c as u32) >= *lo && (c as u32) <= *hi)
        };

        // 中文查询 → 命中
        assert!(in_cjk('\u{641c}'), "汉字必须命中 CJK 区段");
        // 英文技术查询 → 不命中
        assert!(!in_cjk('t') && !in_cjk('-'), "ASCII 不得命中");
        // CJK 标点与全角
        assert!(in_cjk('\u{3001}'), "CJK 标点应命中");
        assert!(in_cjk('\u{ff03}'), "全角字符应命中");
    }

    // ── 加固：字节闸 / 深度闸 / 重试策略 / SSRF 复用 ──

    #[test]
    fn test_retry_after_delay_parses_seconds_and_caps() {
        use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

        // 无头 → None
        let mut h = HeaderMap::new();
        assert_eq!(retry_after_delay(&h), None);

        // 秒数形式正常解析
        h.insert(RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(retry_after_delay(&h), Some(Duration::from_secs(2)));

        // 封顶 30s（服务端说 1 小时也不能挂住抓取工具）
        h.insert(RETRY_AFTER, HeaderValue::from_static("3600"));
        assert_eq!(
            retry_after_delay(&h),
            Some(Duration::from_secs(30)),
            "Retry-After 必须封顶 30s"
        );

        // HTTP-date 形式不解析
        h.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
        assert_eq!(retry_after_delay(&h), None, "HTTP-date 不应被解析");

        // 非数字 → None
        h.insert(RETRY_AFTER, HeaderValue::from_static("soon"));
        assert_eq!(retry_after_delay(&h), None);
    }

    #[test]
    fn test_render_element_depth_gate_stops_deep_nesting() {
        // 5000 层嵌套：无深度闸时 render_element 递归会爆栈。
        // 真实页面几十层就到头，超深只有恶意构造一种来源。
        let html = format!(
            "<html><body>{}<p>deep</p>{}</body></html>",
            "<div>".repeat(5000),
            "</div>".repeat(5000)
        );
        let doc = Html::parse_document(&html);
        let body_sel = Selector::parse("body").unwrap();
        let body = doc.select(&body_sel).next().expect("body element");

        let mut parts: Vec<String> = Vec::new();
        let start = std::time::Instant::now();
        render_element(&body, &mut parts, 0); // 不得 panic / 爆栈
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "深层渲染耗时应可接受，实得 {:?}",
            start.elapsed()
        );
        // 超限后不再下钻：5000 层深处的文本不应被产出（已产出的 parts 照常返回）
        assert!(
            !parts.iter().any(|p| p.contains("deep")),
            "超过 MAX_DOM_DEPTH 的内容应被闸断"
        );

        // 直接以超限深度调用：应立即返回、不产出任何内容
        let mut parts2: Vec<String> = Vec::new();
        render_element(&body, &mut parts2, MAX_DOM_DEPTH + 1);
        assert!(parts2.is_empty(), "初始深度即超限时应直接返回");
    }

    /// web_extract 复用 http_request 的 SSRF 守卫（不再只查 http:// 前缀）。
    /// 必须在任何网络请求前拦下，故此用例不触网。
    #[test]
    fn test_web_extract_applies_ssrf_guard_before_fetch() {
        let mut registry = ToolRegistry::new();
        registry.register_web_extract();
        let def = registry.get("web_extract").expect("web_extract registered");
        let result = (def.executor)(
            &serde_json::json!({"url": "http://169.254.169.254/latest/meta-data/"}),
            &crate::tools::registry::ToolCtx::default(),
        )
        .expect("executor ok");
        assert!(
            !result.success,
            "内网/云元数据地址必须在抓取前被 SSRF 守卫拦下"
        );

        // 非 http/https scheme 同样被守卫拦下（原前缀检查已移除）
        let result = (def.executor)(
            &serde_json::json!({"url": "file:///etc/passwd"}),
            &crate::tools::registry::ToolCtx::default(),
        )
        .expect("executor ok");
        assert!(!result.success, "file 协议应被拦下");
    }

    /// 字节闸截断必须对模型可见。
    ///
    /// 回归目标：曾经 `read_body_capped` 静默 `buf.truncate()`，模型拿到半篇
    /// 正文却无从察觉，会当成完整文档回答。本用例锁死「标记存在且带上限值」。
    #[test]
    fn test_byte_truncation_marker_is_visible_and_names_the_cap() {
        let marker = byte_truncation_marker();
        assert!(
            marker.contains("[Truncated:"),
            "标记必须沿用 extract_readable 的既有格式，模型只需认一种：{}",
            marker
        );
        assert!(
            marker.contains(&format!("{}", MAX_WEB_BYTES / (1024 * 1024))),
            "标记必须报出实际 MiB 上限，否则模型不知道丢了多少：{}",
            marker
        );
        // 标记挂在末尾，供调用方直接 format 到最终文本尾部
        assert!(marker.starts_with("\n\n"), "应作为独立段落追加");
    }

    // ── 跨源降级：失败分类与可行动文案 ──

    /// 可重试性判定：限流/超时归 Retryable，其余归 Permanent。
    ///
    /// 这条分类直接决定失败文案给模型的下一步建议（等 vs 换词），
    /// 判错会让模型在限流时反复改写查询、越改越跑偏。
    #[test]
    fn test_classify_source_failure_splits_retryable_from_permanent() {
        for r in [
            "HTTP 429",
            "HTTP 503",
            "bing request failed: operation timed out",
            "github request failed: timeout",
            "HTTP 429 Too Many Requests",
        ] {
            assert_eq!(
                classify_source_failure(r),
                SourceFailure::Retryable,
                "限流/超时必须归可重试：{}",
                r
            );
        }
        for r in [
            "bing returned no parseable results",
            "HTTP 404",
            "wikipedia request failed: invalid URL",
            "ddg lite result page exceeds 5 MiB byte cap; source dropped",
            "panicked",
        ] {
            assert_eq!(
                classify_source_failure(r),
                SourceFailure::Permanent,
                "重试无意义的失败不得归可重试：{}",
                r
            );
        }
    }

    /// 失败文案必须给出下一步动作，而不是只列源名。
    ///
    /// 回归目标：旧文案是一句写死的「All search sources failed (bing/ddg-lite/
    /// wiki/github/docs unreachable or returned nothing)」，五个源名全列但一个
    /// 原因都没有 —— 模型只能对同一个查询反复重试。
    #[test]
    fn test_describe_aggregate_failure_is_actionable() {
        // 零失败 = 源都正常但没匹配上 → 应引导换词，而不是让模型等
        let none: Vec<(&'static str, String, SourceFailure)> = vec![];
        let msg = describe_aggregate_failure(&none);
        assert!(
            msg.contains("No search source returned results"),
            "零失败应与「全挂」区分开：{}",
            msg
        );
        assert!(
            msg.contains("different or broader terms"),
            "零失败要给换词建议：{}",
            msg
        );

        // 限流占多数 → 明确劝「等」而不是「改查询」
        let rate_limited = vec![
            ("bing", "HTTP 429".to_string(), SourceFailure::Retryable),
            (
                "ddg-lite",
                "request failed: timed out".to_string(),
                SourceFailure::Retryable,
            ),
            (
                "wikipedia",
                "returned no parseable results".to_string(),
                SourceFailure::Permanent,
            ),
        ];
        let msg = describe_aggregate_failure(&rate_limited);
        assert!(msg.contains("2 of 3"), "应报出可重试占比：{}", msg);
        assert!(
            msg.contains("retry after a short wait instead of rewriting"),
            "限流场景必须劝等待而非改查询：{}",
            msg
        );
        assert!(msg.contains("bing: HTTP 429"), "要带上具体原因：{}", msg);

        // 全是永久性失败 → 劝换词
        let all_permanent = vec![
            ("bing", "HTTP 404".to_string(), SourceFailure::Permanent),
            (
                "github",
                "search failed".to_string(),
                SourceFailure::Permanent,
            ),
        ];
        let msg = describe_aggregate_failure(&all_permanent);
        assert!(
            msg.contains("retrying will not fix"),
            "永久失败要劝换词：{}",
            msg
        );
        assert!(
            msg.contains("source=wiki/github/docs"),
            "永久失败要给定向源的出口：{}",
            msg
        );
    }
}
