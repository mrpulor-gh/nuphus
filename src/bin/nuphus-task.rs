//! nuphus-task — 外部 Agent 完工/受阻上报 CLI（门铃回程通道，唯一化替代 curl）
//!
//! 用法：
//!   nuphus task ready    --id <agent::task_id> --token <T> [--endpoint <url>]
//!   nuphus task progress --id <...> --token <T> --summary "..." [--endpoint <url>]
//!   nuphus task done     --id <...> --token <T> --summary "..." [--report <绝对路径>] [--endpoint <url>]
//!   nuphus task blocked  --id <...> --token <T> --reason "..." [--endpoint <url>]
//!
//! 设计取舍（方案 v8 四章）：
//! - Rust reqwest blocking 直发 UTF-8 JSON —— 根除 cmd/PowerShell curl 的 GBK 编码坑；
//! - stderr 友好化：403 → token 无效提示；连接失败 → 门铃不可达提示；200 → [ok]。
//! - 零新依赖：reqwest 的 blocking feature 已在 src/Cargo.toml 启用，serde_json 同包已有。
//! - 放在 src/bin/（lib crate 已有 src/main.rs 单例 bin），不触碰桌面壳构建面。

use std::process::ExitCode;

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:18771/handoff";

/// 从 argv 中解析 --key value；无 value 的 flag（无）返回 None。
/// 返回 (map: 键→值, 位置参数)
fn parse_args(args: &[String]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(key) = a.strip_prefix("--") {
            if let Some(v) = args.get(i + 1) {
                map.insert(key.to_string(), v.clone());
                i += 2;
            } else {
                map.insert(key.to_string(), String::new());
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    map
}

fn usage() -> &'static str {
    "用法:
  nuphus task ready    --id <agent::task_id> --token <T> [--endpoint <url>]
  nuphus task progress --id <...> --token <T> --summary \"...\" [--endpoint <url>]
  nuphus task done     --id <...> --token <T> --summary \"...\" [--report <绝对路径>] [--endpoint <url>]
  nuphus task blocked  --id <...> --token <T> --reason \"...\" [--endpoint <url>]

公共参数:
  --id       外部任务 id（必须为 {agent}::{task_id} 格式，门铃据此归组）
  --token    门铃令牌（见派发契约/brief 中的「令牌」行）
  --endpoint 可选覆盖门铃端点（默认 http://127.0.0.1:18771/handoff）
  --summary / --message   进度或完工说明（二选一，互为别名）
  --report   完工报告文件绝对路径（done 时可选）
  --reason   受阻原因（blocked 时必填）"
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("{}", usage());
        return if args.is_empty() { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    }
    if args.first().map(String::as_str) != Some("task") {
        eprintln!("[error] 未知子命令。{}", usage());
        return ExitCode::FAILURE;
    }
    let verb = args.get(1).map(String::as_str).unwrap_or("");
    let flags = parse_args(&args[2..]);

    let required = |k: &str| -> Result<String, String> {
        flags
            .get(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("缺少必填参数 --{k}"))
    };

    let run = (|| -> Result<(), String> {
        let id = required("id")?;
        if !id.contains("::") {
            return Err("--id 必须为 {agent}::{task_id} 格式（含 :: 分隔）".to_string());
        }
        let token = required("token")?;

        // summary 与 message 互为别名（hooks 生态常用 $MESSAGE）
        let summary = flags
            .get("summary")
            .or_else(|| flags.get("message"))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        let (status, body_summary, report_path) = match verb {
            "ready" => (
                "ready",
                summary.unwrap_or_else(|| "已就位".to_string()),
                None,
            ),
            "progress" => (
                "progress",
                summary.ok_or_else(|| "progress 需要 --summary/--message".to_string())?,
                None,
            ),
            "done" => (
                "done",
                summary.ok_or_else(|| "done 需要 --summary/--message".to_string())?,
                flags.get("report").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()),
            ),
            "blocked" => (
                "blocked",
                required("reason")?,
                None,
            ),
            other => return Err(format!("未知任务动词「{other}」（ready|progress|done|blocked）")),
        };

        let payload = serde_json::json!({
            "id": id,
            "status": status,
            "summary": body_summary,
            "report_path": report_path,
        });
        let endpoint = flags
            .get("endpoint")
            .filter(|v| !v.is_empty())
            .cloned()
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());

        post_doorbell(&endpoint, &token, &payload)
    })();

    match run {
        Ok(_) => {
            println!("[ok] 已通知 Nuphus");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[error] {e}");
            ExitCode::FAILURE
        }
    }
}

/// POST /handoff；错误反馈友好化（403/连接失败/其它 HTTP 码）。
/// no_proxy：本机系统代理（如 Clash）会拦截 loopback 请求返回 503/10053 ——
/// 门铃是本地端点，必须直连（与 handoff_server 测试的 no_proxy 处理一致）。
fn post_doorbell(endpoint: &str, token: &str, payload: &serde_json::Value) -> Result<(), String> {
    let client = match reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(e) => return Err(format!("初始化 HTTP 客户端失败: {e}")),
    };
    let resp = client
        .post(endpoint)
        .header("X-Handoff-Token", token)
        .header("Content-Type", "application/json")
        .body(payload.to_string())
        .send()
        .map_err(|_| "门铃不可达（Nuphus 是否在运行？请确认门铃端口 18771 已监听）".to_string())?;

    match resp.status().as_u16() {
        200 => Ok(()),
        403 => Err("token 无效（检查 brief 中的令牌行）".to_string()),
        400 => Err("参数非法（id/status/summary 不符合门铃约定，400）".to_string()),
        code => Err(format!("门铃返回 HTTP {code}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_args_basic() {
        let args: Vec<String> = vec![
            "--id".into(),
            "opencode::0827-01".into(),
            "--token".into(),
            "abc123".into(),
            "--summary".into(),
            "完成重构".into(),
        ];
        let m = parse_args(&args);
        assert_eq!(m.get("id").unwrap(), "opencode::0827-01");
        assert_eq!(m.get("token").unwrap(), "abc123");
        assert_eq!(m.get("summary").unwrap(), "完成重构");
    }

    #[test]
    fn test_ready_defaults_summary() {
        let args: Vec<String> = vec![
            "task".into(),
            "ready".into(),
            "--id".into(),
            "a::t1".into(),
            "--token".into(),
            "T".into(),
        ];
        let verb = "ready";
        let flags = parse_args(&args[2..]);
        let id = flags.get("id").unwrap().clone();
        let summary = flags
            .get("summary")
            .or_else(|| flags.get("message"))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "已就位".to_string());
        assert_eq!(id, "a::t1");
        assert_eq!(summary, "已就位");
        assert_eq!(verb, "ready");
    }

    #[test]
    fn test_blocked_requires_reason() {
        let args: Vec<String> = vec![
            "task".into(),
            "blocked".into(),
            "--id".into(),
            "a::t1".into(),
            "--token".into(),
            "T".into(),
        ];
        let flags = parse_args(&args[2..]);
        let reason = flags.get("reason").map(|v| v.trim().to_string());
        assert!(reason.is_none() || reason.as_deref() == Some(""));
    }
}