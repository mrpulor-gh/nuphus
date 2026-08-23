//! 外部 Agent 登记簿（plugin/team.toml）CRUD —— 阶段 2 配置中心。
//!
//! 文件路径：{plugin_dir}/team.toml（plugin_dir = find_plugin_dir()）。
//!
//! 段 schema（新字段带默认值，旧文件兼容；写回只增删目标段，不动其它段）：
//! ```toml
//! [key]
//! type = "terminal" | "web-ui" | "desktop"   # 交互协议（旧字段，保留）
//! mode = "background" | "embedded" | "standalone" | "web"  # 窗口/运行模式（新字段）
//! display_name = "..."                       # 显示名（缺省 = key）
//! icon = "terminal"                          # lucide 图标名（缺省 = "bot"）
//! open = "..."                               # 启动命令 / exe 路径 / URL
//! args = "..."                               # 启动参数（缺省 = ""）
//! process = "..."                            # 进程名特征（process_list 识别）
//! description = "..."                        # 职责描述（路由提示 / read.md 职责）
//! note = "..."                               # 旧备注（保留）
//! ```
//!
//! mode → type 归并（写回时）：background→terminal、embedded→terminal、
//! standalone→desktop、web→web-ui；读回时 mode 缺省由 type 反推。
//!
//! 写回策略：段级文本增量 —— 拆分为「首注释块 + 各段原文 + 尾部」，仅替换/追加/删除
//! 目标段，保留首注释块、段顺序与其它段的行内注释；写回前用 toml 解析验证现文件
//! 可解析（防损坏源文件）。代价：目标段自身由模板重新生成（行内注释不保留）。
//!
//! 安全约束：key 校验复用 handoff::validate_agent（[a-zA-Z0-9_-]，禁 '.'/':'），
//! 防路径穿越；team.toml 原子写（tmp + rename）。

use std::path::{Path, PathBuf};

/// 某 agent 段 key 校验：复用 handoff 的 validate_agent 语义
/// （允许 '-' 与 claude-code 对齐；禁 '.'/':'，防路径穿越 / id 分隔符歧义）。
fn validate_key(key: &str) -> Result<(), String> {
    crate::commands::config::handoff::validate_agent(key)
}

/// 合法 mode 四值
const MODES: [&str; 4] = ["background", "embedded", "standalone", "web"];

// ────────────────────────────────────────────────────────────────────────────
// 读取
// ────────────────────────────────────────────────────────────────────────────

fn team_toml_path(plugin_dir: &Path) -> PathBuf {
    plugin_dir.join("team.toml")
}

/// 读 team.toml 为 toml::Value；文件不存在 → 空表；损坏 → Err（不吞错）。
fn read_team_toml(plugin_dir: &Path) -> Result<toml::Value, String> {
    let path = team_toml_path(plugin_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(toml::Value::Table(toml::Table::new()))
        }
        Err(e) => return Err(format!("读 team.toml 失败: {e}")),
    };
    toml::from_str(&content).map_err(|e| format!("解析 team.toml 失败: {e}"))
}

fn str_or(table: &toml::Table, key: &str, default: &str) -> String {
    table
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_string()
}

fn display_name_for(key: &str, obj: &toml::Table) -> String {
    let n = str_or(obj, "display_name", "");
    if n.is_empty() {
        key.to_string()
    } else {
        n
    }
}

fn icon_for(obj: &toml::Table) -> String {
    let i = str_or(obj, "icon", "");
    if i.is_empty() {
        "bot".to_string()
    } else {
        i
    }
}

/// mode 显式存在直接用；否则从 type 反推（terminal→embedded、web-ui→web、desktop→standalone）。
fn mode_for(obj: &toml::Table) -> String {
    let m = str_or(obj, "mode", "");
    if !m.is_empty() {
        return m;
    }
    match str_or(obj, "type", "").as_str() {
        "web-ui" => "web".to_string(),
        "desktop" => "standalone".to_string(),
        _ => "embedded".to_string(),
    }
}

/// type 显式存在直接用；否则从 mode 归并（background/embedded→terminal、
/// standalone→desktop、web→web-ui）。
fn type_for(obj: &toml::Table) -> String {
    let t = str_or(obj, "type", "");
    if !t.is_empty() {
        return t;
    }
    match str_or(obj, "mode", "").as_str() {
        "web" => "web-ui".to_string(),
        "standalone" => "desktop".to_string(),
        _ => "terminal".to_string(),
    }
}

/// mode → type 归并（写回时使用）
fn mode_to_type(mode: &str) -> &'static str {
    match mode {
        "web" => "web-ui",
        "standalone" => "desktop",
        _ => "terminal", // background / embedded
    }
}

/// 列出全部外部 Agent（每个段 → 扁平字段，含默认值补全）。按 key 排序，稳定输出。
#[tauri::command]
pub fn list_external_agents() -> Result<Vec<serde_json::Value>, String> {
    list_external_agents_at(&crate::plugin_apps::find_plugin_dir())
}

fn list_external_agents_at(plugin_dir: &Path) -> Result<Vec<serde_json::Value>, String> {
    let value = read_team_toml(plugin_dir)?;
    let table = value.as_table().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for (key, val) in &table {
        let Some(obj) = val.as_table() else {
            continue; // 非表值（少见）跳过，不 panic
        };
        out.push(serde_json::json!({
            "key": key,
            "display_name": display_name_for(key, obj),
            "icon": icon_for(obj),
            "type": type_for(obj),
            "mode": mode_for(obj),
            "open": str_or(obj, "open", ""),
            "args": str_or(obj, "args", ""),
            "process": str_or(obj, "process", ""),
            "description": str_or(obj, "description", ""),
            "note": str_or(obj, "note", ""),
        }));
    }
    out.sort_by(|a, b| {
        a["key"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["key"].as_str().unwrap_or_default())
    });
    Ok(out)
}

// ────────────────────────────────────────────────────────────────────────────
// 段级文本增量（保留注释 / 段顺序 / 其它段原文）
// ────────────────────────────────────────────────────────────────────────────

struct AgentBlock {
    key: String,
    text: String, // 完整段文本（含段头行）
}

struct TeamTomlDoc {
    head: String,           // 首注释块（第一个段头之前，含空行）
    blocks: Vec<AgentBlock>, // 各段原文（保持文件顺序）
    tail: String,           // 最后一个段之后的行
}

/// 识别顶层段头行 `[key]`：key 仅 [a-zA-Z0-9_-] 且不含 '.'（排除嵌套表），
/// 段头后只允许空白或注释。注释行（# 开头）不匹配。
fn top_level_key(line: &str) -> Option<String> {
    let t = line.trim_start();
    if !t.starts_with('[') {
        return None;
    }
    let close = t.find(']')?;
    let inner = &t[1..close];
    if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let rest = t[close + 1..].trim_start();
    if rest.is_empty() || rest.starts_with('#') {
        Some(inner.to_string())
    } else {
        None
    }
}

fn split_doc(content: &str) -> TeamTomlDoc {
    let mut head = String::new();
    let mut blocks = Vec::new();
    let mut tail = String::new();
    let mut current: Option<AgentBlock> = None;
    let mut seen_header = false;

    for line in content.lines() {
        if let Some(key) = top_level_key(line) {
            seen_header = true;
            if let Some(prev) = current.take() {
                blocks.push(prev);
            }
            current = Some(AgentBlock {
                key,
                text: line.to_string(),
            });
        } else if seen_header {
            if let Some(blk) = current.as_mut() {
                blk.text.push('\n');
                blk.text.push_str(line);
            } else {
                tail.push_str(line);
                tail.push('\n');
            }
        } else {
            head.push_str(line);
            head.push('\n');
        }
    }
    if let Some(prev) = current.take() {
        blocks.push(prev);
    }
    TeamTomlDoc {
        head,
        blocks,
        tail,
    }
}

/// TOML 字符串字面量转义（输出合法 TOML literal）
fn toml_lit(s: &str) -> String {
    format!("{}", toml::Value::String(s.to_string()))
}

/// 生成目标段文本（字段顺序固定；空字段省略行）
fn build_agent_block(
    key: &str,
    type_: &str,
    mode: &str,
    display_name: &str,
    icon: &str,
    open: &str,
    args: &str,
    process: &str,
    description: &str,
    note: &str,
) -> String {
    let mut s = format!("[{key}]\n");
    s.push_str(&format!("type = {}\n", toml_lit(type_)));
    s.push_str(&format!("mode = {}\n", toml_lit(mode)));
    s.push_str(&format!("display_name = {}\n", toml_lit(display_name)));
    s.push_str(&format!("icon = {}\n", toml_lit(icon)));
    if !open.is_empty() {
        s.push_str(&format!("open = {}\n", toml_lit(open)));
    }
    if !args.is_empty() {
        s.push_str(&format!("args = {}\n", toml_lit(args)));
    }
    if !process.is_empty() {
        s.push_str(&format!("process = {}\n", toml_lit(process)));
    }
    if !description.is_empty() {
        s.push_str(&format!("description = {}\n", toml_lit(description)));
    }
    if !note.is_empty() {
        s.push_str(&format!("note = {}\n", toml_lit(note)));
    }
    s
}

/// 原子写（tmp + rename），避免半写残留
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("写 team.toml tmp 失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("落盘 team.toml 失败: {e}"))?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// 增删
// ────────────────────────────────────────────────────────────────────────────

/// 从 JSON 提取段字段（默认值补全）；返回 (type_, mode, display_name, icon,
/// open, args, process, description, note)。
fn extract_agent_fields(
    agent: &serde_json::Value,
    key: &str,
    note_fallback: &str,
) -> Result<(String, String, String, String, String, String, String, String, String), String> {
    let mode = match agent.get("mode").and_then(|v| v.as_str()) {
        Some(m) if MODES.contains(&m) => m.to_string(),
        Some(other) => return Err(format!("非法 mode: {other}（允许 background/embedded/standalone/web）")),
        None => "embedded".to_string(),
    };
    let display_name = match agent.get("display_name").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => key.to_string(),
    };
    let icon = match agent.get("icon").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "bot".to_string(),
    };
    let get = |k: &str| -> String { agent.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string() };
    let note = match agent.get("note").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => note_fallback.to_string(), // 更新时保留原 note
    };
    Ok((
        mode_to_type(&mode).to_string(),
        mode,
        display_name,
        icon,
        get("open"),
        get("args"),
        get("process"),
        get("description"),
        note,
    ))
}

/// upsert 一个 agent 段（按 key 新增/更新）。返回是否为新 agent。
/// 写回保留首注释块 / 段顺序 / 其它段原文。
fn upsert_external_agent_at(plugin_dir: &Path, agent: &serde_json::Value) -> Result<bool, String> {
    let key = agent
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    validate_key(key)?;

    let path = team_toml_path(plugin_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("读 team.toml 失败: {e}")),
    };
    // 写回前验证现文件可解析（防损坏源文件）
    let parsed = if content.trim().is_empty() {
        None
    } else {
        Some(
            toml::from_str::<toml::Value>(&content)
                .map_err(|e| format!("现有 team.toml 解析失败（不写回）: {e}"))?,
        )
    };

    let mut doc = split_doc(&content);
    let is_new = !doc.blocks.iter().any(|b| b.key == key);
    let note_fallback = if is_new {
        ""
    } else {
        parsed
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(|v| v.get("note"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
    };
    let (type_, mode, display_name, icon, open, args, process, description, note) =
        extract_agent_fields(agent, key, note_fallback)?;

    let new_text = build_agent_block(
        key, &type_, &mode, &display_name, &icon, &open, &args, &process, &description, &note,
    );
    if is_new {
        doc.blocks.push(AgentBlock {
            key: key.to_string(),
            text: new_text,
        });
    } else if let Some(blk) = doc.blocks.iter_mut().find(|b| b.key == key) {
        blk.text = new_text;
    }

    let mut out = doc.head.clone();
    for b in &doc.blocks {
        out.push_str(&b.text);
        out.push('\n');
    }
    out.push_str(&doc.tail);
    atomic_write(&path, &out)?;
    Ok(is_new)
}

/// 删除一个 agent 段（段不存在 → 无操作成功）。不删除 .nuphus/handoff/{key}/ 目录。
#[tauri::command]
pub fn delete_external_agent(key: String) -> Result<(), String> {
    delete_external_agent_at(&crate::plugin_apps::find_plugin_dir(), &key)
}

fn delete_external_agent_at(plugin_dir: &Path, key: &str) -> Result<(), String> {
    validate_key(key)?;
    let path = team_toml_path(plugin_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()), // 无文件视为已删除
        Err(e) => return Err(format!("读 team.toml 失败: {e}")),
    };
    let mut doc = split_doc(&content);
    let before = doc.blocks.len();
    doc.blocks.retain(|b| b.key != key);
    if doc.blocks.len() == before {
        return Ok(()); // 段不存在，无操作
    }
    let mut out = doc.head.clone();
    for b in &doc.blocks {
        out.push_str(&b.text);
        out.push('\n');
    }
    out.push_str(&doc.tail);
    atomic_write(&path, &out)
}

// ────────────────────────────────────────────────────────────────────────────
// 公共命令（含 agent_init 联动）
// ────────────────────────────────────────────────────────────────────────────

/// upsert 外部 Agent；新 agent 时联动 handoff::init_agent_at 生成
/// .nuphus/handoff/{key}/（read.md 职责 = description）。返回 "created"/"updated"。
#[tauri::command]
pub fn upsert_external_agent(agent: serde_json::Value) -> Result<String, String> {
    let plugin_dir = crate::plugin_apps::find_plugin_dir();
    let is_new = upsert_and_init_at(&plugin_dir, &handoff_root(), &agent)?;
    Ok(if is_new { "created" } else { "updated" }.to_string())
}

/// team.toml 写回 + 新 agent 联动 handoff 目录初始化（root 注入，供单测隔离）。
fn upsert_and_init_at(
    team_root: &Path,
    handoff_root: &Path,
    agent: &serde_json::Value,
) -> Result<bool, String> {
    let is_new = upsert_external_agent_at(team_root, agent)?;
    if is_new {
        let key = agent["key"].as_str().unwrap_or_default();
        let description = agent["description"].as_str().unwrap_or_default();
        crate::commands::config::handoff::init_agent_at(handoff_root, key, description)
            .map_err(|e| format!("初始化 agent 工作目录失败: {e}"))?;
    }
    Ok(is_new)
}

/// handoff 根目录（复用 handoff 模块定义）
fn handoff_root() -> PathBuf {
    crate::commands::config::handoff::handoff_root()
}

// ────────────────────────────────────────────────────────────────────────────
// Agent 图标提取
// ────────────────────────────────────────────────────────────────────────────

/// 提取外部 Agent 应用图标，返回 data URL（前端可直接 <img src>）。
///
/// 支持：
/// - 图片文件（png/jpg/jpeg/webp/gif/bmp）：直接读取编码为 data URL；
/// - exe/dll/ico 等关联图标：Windows 下用 System.Drawing.ExtractAssociatedIcon
///   提取首图标 → PNG → base64（PowerShell 调用，仅在用户触发时执行一次）。
///
/// icon 字段值域（配合前端）：
/// - lucide 预设名（bot/terminal/...）→ 前端 SVG 渲染，不走此命令；
/// - "auto" → 前端取 open/process 路径调用此命令；
/// - 显式文件路径（.png/.ico/.exe...）→ 前端直接调用此命令。
#[tauri::command]
pub fn extract_agent_icon(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("图标文件不存在: {path}"));
    }
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        // 位图类：浏览器原生支持，直接 data URL
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => {
            let bytes = std::fs::read(p).map_err(|e| format!("读取图标失败: {e}"))?;
            let mime = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "gif" => "image/gif",
                "bmp" => "image/bmp",
                _ => "image/png",
            };
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            Ok(format!("data:{mime};base64,{b64}"))
        }
        // 可执行/图标容器：提取关联图标 → PNG
        _ => extract_associated_icon_png(p),
    }
}

/// exe/dll/ico → 关联图标 PNG data URL（Windows：System.Drawing.ExtractAssociatedIcon）。
#[cfg(target_os = "windows")]
fn extract_associated_icon_png(path: &Path) -> Result<String, String> {
    let escaped = path.display().to_string().replace('\'', "''");
    let script = format!(
        "Add-Type -AssemblyName System.Drawing; \
         try {{ $i = [System.Drawing.Icon]::ExtractAssociatedIcon('{escaped}'); \
         if (-not $i) {{ exit 2 }}; \
         $b = $i.ToBitmap(); \
         $ms = New-Object System.IO.MemoryStream; \
         $b.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); \
         [Convert]::ToBase64String($ms.ToArray()) }} catch {{ exit 1 }}"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("调用 PowerShell 提取图标失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "提取关联图标失败（code {}，可能不是有效应用/图标文件）",
            out.status.code().unwrap_or(-1)
        ));
    }
    let b64 = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if b64.is_empty() {
        return Err("提取关联图标为空（文件无可用图标）".to_string());
    }
    Ok(format!("data:image/png;base64,{b64}"))
}

/// 非 Windows：无 ExtractAssociatedIcon，图片类已在上层处理，其余不支持。
#[cfg(not(target_os = "windows"))]
fn extract_associated_icon_png(_path: &Path) -> Result<String, String> {
    Err("当前平台暂不支持提取应用关联图标，请使用 PNG/JPG 图片作为自定义图标".to_string())
}

// ────────────────────────────────────────────────────────────────────────────
// 测试
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join("nuphus-team-test");
        let dir = base.join(format!("{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 含首注释块（含 `# [示例]` 注释段头，验证 top_level_key 跳过注释）
    const SAMPLE: &str = r#"# team.toml — 外部 Agent 登记簿
# 规则：用一次记一段（渐进积累），只记稳定事实。
# 段格式示例（注释，不应被解析）：
# [示例]

[claude-code]
type = "terminal"
open = "终端执行 claude"
process = "claude.exe"
note = "Claude Code v2.1.220"

[opencode]
type = "terminal"
open = "终端执行 opencode"
process = "opencode.exe"
"#;

    #[test]
    fn test_list_external_agents_with_defaults() {
        let root = tmp_root("list");
        std::fs::write(root.join("team.toml"), SAMPLE).unwrap();
        let list = list_external_agents_at(&root).unwrap();
        assert_eq!(list.len(), 2);
        let cc = &list[0]; // 按 key 排序 claude-code < opencode
        assert_eq!(cc["key"], "claude-code");
        assert_eq!(cc["display_name"], "claude-code"); // 无 display_name → key
        assert_eq!(cc["icon"], "bot"); // 无 icon → bot
        assert_eq!(cc["type"], "terminal");
        assert_eq!(cc["mode"], "embedded"); // 无 mode，type=terminal → embedded
        assert_eq!(cc["open"], "终端执行 claude");
        assert_eq!(cc["process"], "claude.exe");
        assert_eq!(cc["note"], "Claude Code v2.1.220");
        assert_eq!(cc["args"], "");
        // 文件不存在 → 空数组
        assert!(list_external_agents_at(&tmp_root("nope")).unwrap().is_empty());
    }

    #[test]
    fn test_upsert_new_and_update() {
        let root = tmp_root("upsert");
        std::fs::write(root.join("team.toml"), SAMPLE).unwrap();

        // 新增
        let new_agent = serde_json::json!({
            "key": "hermes",
            "display_name": "Hermes 虞儿",
            "icon": "bot",
            "mode": "standalone",
            "open": "终端执行 hermes",
            "args": "--headless",
            "process": "hermes.exe",
            "description": "协同 Agent"
        });
        assert!(upsert_external_agent_at(&root, &new_agent).unwrap());
        let list = list_external_agents_at(&root).unwrap();
        assert_eq!(list.len(), 3);
        let hermes = list.iter().find(|a| a["key"] == "hermes").unwrap();
        assert_eq!(hermes["type"], "desktop"); // standalone → desktop 归并
        assert_eq!(hermes["mode"], "standalone");
        assert_eq!(hermes["args"], "--headless");
        assert_eq!(hermes["description"], "协同 Agent");
        // 首注释块 / 既有段保留
        let content = std::fs::read_to_string(root.join("team.toml")).unwrap();
        assert!(content.contains("# 规则：用一次记一段"));
        assert!(content.contains("[claude-code]"));
        assert!(content.contains("Claude Code v2.1.220"));
        assert!(content.contains("[opencode]"));

        // 更新（同 key）：note 保留 + 新字段生效
        let upd = serde_json::json!({
            "key": "claude-code",
            "display_name": "Claude Code Pro",
            "icon": "terminal",
            "mode": "embedded",
            "open": "终端执行 claude",
            "process": "claude.exe",
            "description": "新职责"
        });
        assert!(!upsert_external_agent_at(&root, &upd).unwrap());
        let list = list_external_agents_at(&root).unwrap();
        let cc = list.iter().find(|a| a["key"] == "claude-code").unwrap();
        assert_eq!(cc["display_name"], "Claude Code Pro");
        assert_eq!(cc["icon"], "terminal");
        assert_eq!(cc["note"], "Claude Code v2.1.220"); // 更新保留原 note
        assert_eq!(cc["description"], "新职责");
        assert_eq!(list.len(), 3); // 不重复添加
    }

    #[test]
    fn test_upsert_rejects_bad_mode() {
        let root = tmp_root("badmode");
        std::fs::write(root.join("team.toml"), SAMPLE).unwrap();
        let agent = serde_json::json!({ "key": "x", "mode": "floating" });
        assert!(upsert_external_agent_at(&root, &agent).is_err());
    }

    #[test]
    fn test_delete_agent() {
        let root = tmp_root("delete");
        std::fs::write(root.join("team.toml"), SAMPLE).unwrap();
        delete_external_agent_at(&root, "opencode").unwrap();
        let list = list_external_agents_at(&root).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["key"], "claude-code");
        let content = std::fs::read_to_string(root.join("team.toml")).unwrap();
        assert!(!content.contains("[opencode]"));
        assert!(content.contains("[claude-code]"));
        // 删除不存在段 → 无操作不报错
        delete_external_agent_at(&root, "ghost").unwrap();
        assert_eq!(list_external_agents_at(&root).unwrap().len(), 1);
    }

    #[test]
    fn test_rejects_unsafe_key() {
        let root = tmp_root("unsafe");
        std::fs::write(root.join("team.toml"), SAMPLE).unwrap();
        for key in ["../evil", "a.b", "a:b", "", "a/b", ".."] {
            let agent = serde_json::json!({ "key": key });
            assert!(
                upsert_external_agent_at(&root, &agent).is_err(),
                "key={key} 应被拒绝"
            );
        }
        // 含 '-' 的 key 合法（与 claude-code 对齐）
        let ok = serde_json::json!({ "key": "claude-code", "mode": "embedded" });
        assert!(upsert_external_agent_at(&root, &ok).is_ok());
    }

    #[test]
    fn test_upsert_new_initializes_handoff_dir() {
        let team_root = tmp_root("init-team");
        let handoff_root = tmp_root("init-handoff");
        std::fs::write(team_root.join("team.toml"), SAMPLE).unwrap();

        let agent = serde_json::json!({
            "key": "web_agent",
            "display_name": "Web Agent",
            "mode": "embedded",
            "open": "web_agent",
            "description": "负责网页任务"
        });
        assert!(upsert_and_init_at(&team_root, &handoff_root, &agent).unwrap());
        // handoff 目录已生成（read.md/memory.md/status.json）
        let dir = handoff_root.join("web_agent");
        assert!(dir.join("read.md").is_file());
        assert!(dir.join("memory.md").is_file());
        assert!(dir.join("status.json").is_file());
        let read = std::fs::read_to_string(dir.join("read.md")).unwrap();
        assert!(read.contains("负责网页任务"));
        // 更新不重复初始化（is_new=false）
        assert!(!upsert_and_init_at(&team_root, &handoff_root, &agent).unwrap());
    }

    /// 真实 plugin/team.toml 快照：验证拆分/重组不丢段、不丢首注释块，
    /// 且能处理注释中的 `[claude-code]` 示例段头、中文/特殊字符（✳）。
    #[test]
    fn test_roundtrip_real_team_toml_preserves_comments_and_order() {
        let real = r#"# team.toml — 外部 Agent 登记簿
# 规则：用一次记一段（渐进积累），只记稳定事实。
# 禁止：PID / 窗口句柄 / 屏幕坐标等易变值（坐标参数走 plugin/ui-maps/）。
# 调用时：process_list 匹配 process 字段看哪些在跑 + Leader 内部知识定能力匹配。
#
# 段格式：
#
# [claude-code]
# type = "terminal"           # terminal | web-ui | desktop — 决定交互协议（见 agent-orchestration §2）
# open = "终端执行 claude"     # 打开方式：命令 / exe 路径 / URL
# process = "claude.exe"      # 进程名特征，process_list 据此识别是否在跑
# note = "可选，一句话备注"

[claude-code]
type = "terminal"
open = "终端执行 claude"
process = "claude.exe"
note = "Claude Code v2.1.220,门铃测试已验证(2026-07-29),父进程 powershell,窗口标题 ✳ Claude Code,启动器位于 C:\\Users\\Administrator\\AppData\\Roaming\\npm\\claude.ps1"
[opencode]
type = "terminal"
open = "终端执行 opencode"
process = "opencode.exe"
note = "OpenCode CLI,父进程 powershell,窗口标题 OpenCode"

[hermes]
type = "terminal"
open = "终端执行 hermes"
process = "hermes.exe"
note = "Hermes Agent,父进程 powershell,窗口标题 管理员: Windows PowerShell"

[gemini]
type = "web-ui"
open = "https://gemini.google.com"
process = "chrome.exe"
note = "Google Gemini 官网问答版，Chrome 标签页，窗口标题 'Google Gemini - Google Chrome'，已登录 Yue 账号"
"#;
        let root = tmp_root("real");
        std::fs::write(root.join("team.toml"), real).unwrap();

        // 读回 4 个段；注释中的 `# [claude-code]` 不被当作段
        let list = list_external_agents_at(&root).unwrap();
        assert_eq!(list.len(), 4);
        let keys: Vec<&str> = list.iter().map(|a| a["key"].as_str().unwrap()).collect();
        assert_eq!(keys, vec!["claude-code", "gemini", "hermes", "opencode"]);
        assert_eq!(list[0]["mode"], "embedded"); // terminal → embedded
        assert_eq!(list[1]["mode"], "web"); // web-ui → web

        // upsert 新增段：首注释块 / 特殊字符 / 既有段保留，段顺序不变（新段追加尾部）
        let new_agent = serde_json::json!({
            "key": "web_agent",
            "mode": "embedded",
            "open": "web_agent"
        });
        upsert_external_agent_at(&root, &new_agent).unwrap();
        let content = std::fs::read_to_string(root.join("team.toml")).unwrap();
        assert!(content.contains("窗口标题 ✳ Claude Code"));
        assert!(content.contains("# 禁止：PID / 窗口句柄 / 屏幕坐标等易变值"));
        assert!(content.contains("[web_agent]"));
        let cc = content.find("[claude-code]").unwrap();
        let oc = content.find("[opencode]").unwrap();
        let we = content.find("[web_agent]").unwrap();
        assert!(cc < oc && oc < we, "段顺序应保持，新段追加在尾部");

        // delete 后消失
        delete_external_agent_at(&root, "gemini").unwrap();
        let list = list_external_agents_at(&root).unwrap();
        assert_eq!(list.len(), 4); // web_agent + 原 3 段 - gemini
        assert!(!list.iter().any(|a| a["key"] == "gemini"));
    }
}