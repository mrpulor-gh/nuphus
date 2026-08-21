use super::types::*;
use std::path::{Path, PathBuf};

/// 项目内置技能根目录（plugin/skills/）
///
/// nuphus crate 的 Cargo.toml 在 {workspace_root}/src/ 下，
/// CARGO_MANIFEST_DIR 的父目录 = 工作区根。
pub fn plugin_skills_dir() -> PathBuf {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    src_dir
        .parent()
        .map(|p| p.join("plugin").join("skills"))
        .unwrap_or_else(|| src_dir.join("plugin").join("skills"))
}

/// Skill 注册表 — 统一数据源：plugin/skills/
/// builtin/ — 内置，git追踪，不可删除
/// community/ — 第三方安装，gitignore，可增删
pub struct SkillRegistry;

impl SkillRegistry {
    /// 创建注册表（无状态）
    pub fn new() -> Self {
        Self
    }

    // ── 目录辅助 ──

    fn builtin_dir() -> PathBuf {
        plugin_skills_dir().join("builtin")
    }

    fn community_dir() -> PathBuf {
        plugin_skills_dir().join("community")
    }

    fn read_manifest(dir: &Path) -> Option<SkillManifest> {
        let mp = dir.join("skill.json");
        if !mp.exists() {
            return None;
        }
        let mut content = std::fs::read_to_string(&mp).ok()?;
        // 去除 UTF-8 BOM（\u{FEFF}），部分编辑器会写入
        if content.starts_with('\u{FEFF}') {
            content = content[3..].to_string();
        }
        match serde_json::from_str(&content) {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::warn!("Failed to parse skill manifest {}: {}", mp.display(), e);
                None
            }
        }
    }

    fn scan_subdir(subdir: &Path) -> Vec<(PathBuf, SkillManifest)> {
        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(subdir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if let Some(m) = Self::read_manifest(&path) {
                    results.push((path, m));
                }
            }
        }
        results
    }

    // ── 公共 API ──

    /// 读取 SKILL.md（builtin 优先 → community 兜底）
    pub fn get_skill_md(&self, name: &str) -> Option<String> {
        let path = Self::builtin_dir().join(name).join("SKILL.md");
        if let Ok(content) = std::fs::read_to_string(&path) {
            return Some(content);
        }
        let path = Self::community_dir().join(name).join("SKILL.md");
        std::fs::read_to_string(path).ok()
    }

    /// 校验 skill 名称，防止路径穿越（前端输入与 git 仓库 manifest 均不可信）
    fn validate_name(name: &str) -> std::result::Result<(), String> {
        if name.is_empty() || name.len() > 64 {
            return Err(format!("invalid skill name length: '{}'", name));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!(
                "invalid skill name (allowed: A-Z a-z 0-9 _ -): '{}'",
                name
            ));
        }
        Ok(())
    }

    /// 安装第三方 skill → community/
    pub fn install_from_path(&self, source: &str) -> std::result::Result<SkillManifest, String> {
        let src = Path::new(source);
        if !src.exists() {
            return Err(format!("source path does not exist: {}", source));
        }

        let manifest: SkillManifest = Self::read_manifest(src)
            .ok_or_else(|| format!("parse skill.json failed in {}", source))?;

        Self::validate_name(&manifest.name)?;

        // 内置 skill 同名冲突
        if Self::builtin_dir().join(&manifest.name).exists() {
            return Err(format!(
                "'{}' conflicts with a builtin skill",
                manifest.name
            ));
        }

        let dest = Self::community_dir().join(&manifest.name);
        if dest.exists() {
            return Err(format!("skill '{}' already installed", manifest.name));
        }

        self.copy_dir(src, &dest)
            .map_err(|e| format!("copy skill dir failed: {}", e))?;

        Ok(manifest)
    }

    /// 卸载第三方 skill（内置 skill 不可删除）
    pub fn remove(&self, name: &str) -> std::result::Result<(), String> {
        Self::validate_name(name)?;

        if Self::builtin_dir().join(name).exists() {
            return Err(format!(
                "'{}' is a builtin skill and cannot be removed",
                name
            ));
        }

        let path = Self::community_dir().join(name);
        if !path.exists() {
            return Err(format!("skill '{}' not found in community", name));
        }

        std::fs::remove_dir_all(&path).map_err(|e| format!("remove skill dir failed: {}", e))
    }

    /// 读取 skill.json（builtin → community 优先级）
    pub fn get_manifest(&self, name: &str) -> Option<SkillManifest> {
        Self::read_manifest(&Self::builtin_dir().join(name))
            .or_else(|| Self::read_manifest(&Self::community_dir().join(name)))
    }

    fn entry_from_manifest(m: &SkillManifest, path: &Path, builtin: bool) -> SkillEntry {
        SkillEntry {
            name: m.name.clone(),
            path: path.to_string_lossy().to_string(),
            version: m.version.clone(),
            display_name: m.display_name.clone(),
            description: m.description.clone(),
            installed_at: String::new(),
            active: true,
            keywords: m.keywords.clone(),
            builtin,
        }
    }

    /// 获取 skill 信息（builtin → community 优先级）
    pub fn get(&self, name: &str) -> Option<SkillEntry> {
        let bp = Self::builtin_dir().join(name);
        if let Some(m) = Self::read_manifest(&bp) {
            return Some(Self::entry_from_manifest(&m, &bp, true));
        }
        let cp = Self::community_dir().join(name);
        if let Some(m) = Self::read_manifest(&cp) {
            return Some(Self::entry_from_manifest(&m, &cp, false));
        }
        None
    }

    /// 列出所有 skill（builtin + community）
    pub fn list(&self) -> Vec<SkillEntry> {
        let mut results = Vec::new();
        for (path, m) in Self::scan_subdir(&Self::builtin_dir()) {
            results.push(Self::entry_from_manifest(&m, &path, true));
        }
        for (path, m) in Self::scan_subdir(&Self::community_dir()) {
            if results.iter().any(|e: &SkillEntry| e.name == m.name) {
                continue;
            }
            results.push(Self::entry_from_manifest(&m, &path, false));
        }
        results
    }

    /// 按名称/关键词搜索（builtin + community）
    pub fn search(&self, query: &str) -> Vec<SkillEntry> {
        let q = query.to_lowercase();
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (path, m) in Self::scan_subdir(&Self::builtin_dir())
            .into_iter()
            .chain(Self::scan_subdir(&Self::community_dir()))
        {
            if !seen.insert(m.name.clone()) {
                continue;
            }
            let nl = m.name.to_lowercase();
            let dl = m.display_name.to_lowercase();
            if nl.contains(&q)
                || dl.contains(&q)
                || m.keywords.iter().any(|k| k.to_lowercase().contains(&q))
            {
                let builtin = path.starts_with(Self::builtin_dir());
                results.push(Self::entry_from_manifest(&m, &path, builtin));
            }
        }
        results
    }

    /// 在已安装 skill 中查询（基于 tokenize + 关键词匹配）
    pub fn query(&self, input: &SkillQueryInput) -> SkillQueryOutput {
        let query_tokens = tokenize(&input.query);
        if query_tokens.is_empty() {
            return SkillQueryOutput {
                hits: vec![],
                total: 0,
            };
        }

        let mut hits: Vec<SkillQueryHit> = Vec::new();

        for (path, _) in Self::scan_subdir(&Self::builtin_dir())
            .into_iter()
            .chain(Self::scan_subdir(&Self::community_dir()))
        {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            if let Some(ref sn) = input.skill {
                if name != sn.as_str() {
                    continue;
                }
            }

            // SKILL.md 匹配
            if let Some(md) = self.get_skill_md(name) {
                let md_tokens = tokenize(&md);
                let mc = query_tokens
                    .iter()
                    .filter(|qt| md_tokens.contains(qt))
                    .count();
                if mc > 0 {
                    let relevance = (mc as f32 / query_tokens.len() as f32).min(1.0);
                    hits.push(SkillQueryHit {
                        skill: name.to_string(),
                        title: format!("{}/SKILL.md", name),
                        content: md,
                        relevance,
                        source: "SKILL.md".to_string(),
                    });
                }
            }

            // data/ 目录下的文件匹配
            let data_dir = path.join("data");
            if data_dir.exists() {
                if let Ok(files) = std::fs::read_dir(&data_dir) {
                    for file in files.flatten() {
                        let fp = file.path();
                        if let Ok(content) = std::fs::read_to_string(&fp) {
                            let ft = tokenize(&content);
                            let mc = query_tokens.iter().filter(|qt| ft.contains(qt)).count();
                            if mc > 0 {
                                let relevance = (mc as f32 / query_tokens.len() as f32).min(1.0);
                                hits.push(SkillQueryHit {
                                    skill: name.to_string(),
                                    title: format!(
                                        "{}/data/{}",
                                        name,
                                        fp.file_name().unwrap_or_default().to_string_lossy()
                                    ),
                                    content,
                                    relevance,
                                    source: format!(
                                        "data/{}",
                                        fp.file_name().unwrap_or_default().to_string_lossy()
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }

        hits.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let total = hits.len();
        SkillQueryOutput { hits, total }
    }

    /// 从 GitHub 仓库安装技能（git clone → install_from_path）
    pub fn install_from_git(&self, url: &str) -> std::result::Result<SkillManifest, String> {
        let tmp_dir = std::env::temp_dir().join(format!(
            "nuphus_skill_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));

        let output = std::process::Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                url,
                tmp_dir.to_string_lossy().as_ref(),
            ])
            .output()
            .map_err(|e| format!("Git not available: {}. Use folder install instead.", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            std::fs::remove_dir_all(&tmp_dir).ok();
            return Err(format!("Git clone failed: {}", stderr));
        }

        if !tmp_dir.join("skill.json").exists() {
            // 降级适配：Claude Code / Codex 生态仓库通常无 skill.json，
            // 尝试从 .claude-plugin/plugin.json、SKILL.md frontmatter 合成 manifest。
            if let Err(e) = self.synthesize_manifest(&tmp_dir) {
                std::fs::remove_dir_all(&tmp_dir).ok();
                return Err(e);
            }
        }

        let result = self.install_from_path(tmp_dir.to_string_lossy().as_ref());
        std::fs::remove_dir_all(&tmp_dir).ok();
        result
    }

    // ── 私有辅助 ──

    /// 降级适配：为缺少 skill.json 的第三方仓库合成 manifest。
    ///
    /// 支持的数据源（按优先级）：
    /// 1. `.claude-plugin/plugin.json`（Claude Code 插件清单，结构化 JSON）
    /// 2. `SKILL.md` frontmatter（YAML 轻量解析：name / description）
    /// 3. 仓库根目录名（git clone 后的目录名 = 仓库名，兜底）
    ///
    /// 成功时在 dir 下写入 skill.json，返回解析出的 manifest。
    fn synthesize_manifest(&self, dir: &Path) -> Result<SkillManifest, String> {
        let mut name: Option<String> = None;
        let mut version = String::new();
        let mut description = String::new();
        let mut author = String::new();
        let mut keywords: Vec<String> = Vec::new();
        let triggers = SkillTriggers::default();

        // 1. .claude-plugin/plugin.json
        let plugin_json = dir.join(".claude-plugin").join("plugin.json");
        if plugin_json.exists() {
            if let Ok(content) = std::fs::read_to_string(&plugin_json) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    name = v
                        .get("name")
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string())
                        .or(name);
                    version = v
                        .get("version")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    description = v
                        .get("description")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    author = v
                        .get("author")
                        .and_then(|a| a.as_str())
                        .or_else(|| {
                            v.get("author")
                                .and_then(|a| a.get("name"))
                                .and_then(|n| n.as_str())
                        })
                        .unwrap_or("")
                        .to_string();
                    if let Some(arr) = v.get("keywords").and_then(|k| k.as_array()) {
                        keywords = arr
                            .iter()
                            .filter_map(|k| k.as_str().map(|s| s.to_string()))
                            .collect();
                    }
                }
            }
        }

        // 2. SKILL.md frontmatter（补全缺失字段）
        let skill_md = dir.join("SKILL.md");
        if skill_md.exists() {
            if let Ok(content) = std::fs::read_to_string(&skill_md) {
                if let Some(fm) = parse_skill_frontmatter(&content) {
                    if name.is_none() {
                        name = fm.get("name").cloned();
                    }
                    if description.is_empty() {
                        description = fm.get("description").cloned().unwrap_or_default();
                    }
                }
            }
        }

        // 3. 兜底：仓库目录名
        let resolved_name = name
            .filter(|n| !n.trim().is_empty())
            .or_else(|| {
                dir.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            })
            .ok_or_else(|| "No skill.json found in repository root".to_string())?;

        let clean_name = sanitize_skill_name(&resolved_name);
        Self::validate_name(&clean_name)?;

        let manifest = SkillManifest {
            name: clean_name.clone(),
            version: if version.trim().is_empty() {
                "0.1.0".to_string()
            } else {
                version
            },
            display_name: clean_name.clone(),
            description,
            author,
            keywords,
            triggers,
            data_sources: vec![],
            requires_tools: vec![],
            conflicts: vec![],
        };

        let json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("serialize synthesized skill.json failed: {}", e))?;
        std::fs::write(dir.join("skill.json"), json)
            .map_err(|e| format!("write synthesized skill.json failed: {}", e))?;

        Ok(manifest)
    }

    fn copy_dir(&self, src: &Path, dst: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let path = entry.path();
            let dest = dst.join(entry.file_name());
            if path.is_dir() {
                self.copy_dir(&path, &dest)?;
            } else {
                std::fs::copy(&path, &dest)?;
            }
        }
        Ok(())
    }
}

/// 轻量解析 Markdown 开头的 YAML frontmatter（--- 包裹的键值块）。
/// 只提取 `key: value` 单行字段；不处理嵌套/数组/多行标量（够用即可）。
fn parse_skill_frontmatter(content: &str) -> Option<std::collections::HashMap<String, String>> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut map = std::collections::HashMap::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            let key = k.trim().to_string();
            let val = v.trim().to_string();
            if !key.is_empty() {
                map.insert(key, val);
            }
        }
    }
    Some(map)
}

/// 清洗为合法 skill 名（ascii alnum / _ / -，≤64，兜底 fallback）
fn sanitize_skill_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if out.len() < 64 && !out.is_empty() {
            // 非合法字符用 - 连接（避免相邻分隔符重复）
            if !out.ends_with('-') {
                out.push('-');
            }
        }
        if out.len() >= 64 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_valid_names() {
        assert_eq!(sanitize_skill_name("video-shotcraft"), "video-shotcraft");
        assert_eq!(sanitize_skill_name("my_skill"), "my_skill");
        assert_eq!(sanitize_skill_name("A1-b_c"), "A1-b_c");
    }

    #[test]
    fn sanitize_replaces_invalid_chars() {
        assert_eq!(sanitize_skill_name("My Skill!"), "My-Skill");
        assert_eq!(sanitize_skill_name("  spaced  "), "spaced");
        assert_eq!(sanitize_skill_name("视频"), "");
    }

    #[test]
    fn sanitize_truncates_to_64() {
        let long = "a".repeat(100);
        assert!(sanitize_skill_name(&long).len() <= 64);
    }

    #[test]
    fn parse_frontmatter_basic() {
        let md = "---\nname: video-shotcraft\ndescription: Make videos\n---\n# body";
        let fm = parse_skill_frontmatter(md).unwrap();
        assert_eq!(fm.get("name").map(|s| s.as_str()), Some("video-shotcraft"));
        assert_eq!(
            fm.get("description").map(|s| s.as_str()),
            Some("Make videos")
        );
    }

    #[test]
    fn parse_frontmatter_missing_returns_none() {
        assert!(parse_skill_frontmatter("# no frontmatter").is_none());
        assert!(parse_skill_frontmatter("---\nname: x\n").is_some());
    }

    #[test]
    fn synthesize_uses_plugin_json() {
        let dir = std::env::temp_dir().join("nuphus_skill_synth_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        std::fs::write(
            dir.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"my-plugin","version":"2.0.0","description":"A plugin","author":{"name":"Alice"},"keywords":["video","demo"]}"#,
        )
        .unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: other-name\n---\n").unwrap();

        let reg = SkillRegistry::new();
        let m = reg.synthesize_manifest(&dir).unwrap();
        assert_eq!(m.name, "my-plugin");
        assert_eq!(m.version, "2.0.0");
        assert_eq!(m.author, "Alice");
        assert_eq!(m.keywords, vec!["video", "demo"]);
        assert!(dir.join("skill.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn synthesize_falls_back_to_skill_md_and_dirname() {
        let dir = std::env::temp_dir().join("nuphus-skill-fallback");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: fallback-skill\ndescription: From md\n---\n",
        )
        .unwrap();

        let reg = SkillRegistry::new();
        let m = reg.synthesize_manifest(&dir).unwrap();
        assert_eq!(m.name, "fallback-skill");
        assert_eq!(m.description, "From md");
        let _ = std::fs::remove_dir_all(&dir);

        // 无任何元数据 → 兜底目录名（带非法字符被清洗）
        let dir2 = std::env::temp_dir().join("My Cool Repo");
        let _ = std::fs::remove_dir_all(&dir2);
        std::fs::create_dir_all(&dir2).unwrap();
        let m2 = reg.synthesize_manifest(&dir2).unwrap();
        assert_eq!(m2.name, "My-Cool-Repo");
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn install_from_git_synthesizes_for_claude_repo() {
        // 构造一个 Claude Code 格式的本地 git 仓库（根目录无 skill.json，只有 SKILL.md + .claude-plugin/plugin.json）
        let repo = std::env::temp_dir().join("nuphus_skill_git_test_repo");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join(".claude-plugin")).unwrap();
        std::fs::write(
            repo.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"git-synth-test","version":"1.0.0","description":"Git synth skill","keywords":["test","video"]}"#,
        )
        .unwrap();
        std::fs::write(
            repo.join("SKILL.md"),
            "---\nname: git-synth-test\ndescription: Git synth skill\n---\n# body",
        )
        .unwrap();

        let run = |cmd: &str, dir: &std::path::Path| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(cmd.split_whitespace())
                .env("GIT_AUTHOR_NAME", "test")
                .env("GIT_AUTHOR_EMAIL", "test@test.local")
                .env("GIT_COMMITTER_NAME", "test")
                .env("GIT_COMMITTER_EMAIL", "test@test.local")
                .output()
                .expect("git should be available");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                cmd,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run("init -b main", &repo);
        run("add -A", &repo);
        run("commit -m init", &repo);

        let reg = SkillRegistry::new();
        let url = format!("file://{}", repo.display());
        let m = reg
            .install_from_git(&url)
            .expect("install_from_git should synthesize manifest");
        assert_eq!(m.name, "git-synth-test");

        let dest = SkillRegistry::community_dir().join("git-synth-test");
        assert!(
            dest.join("skill.json").exists(),
            "synthesized skill.json should be copied"
        );
        assert!(dest.join("SKILL.md").exists(), "SKILL.md should be copied");

        let _ = reg.remove("git-synth-test");
        let _ = std::fs::remove_dir_all(&repo);
    }
}
fn tokenize(text: &str) -> Vec<String> {
    crate::segmenter::segment(text)
}
