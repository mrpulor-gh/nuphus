//! Changelog command — 仓库 CHANGELOG 全文（编译期嵌入）
//!
//! 打包版既没有仓库文件、也不该为读一段说明去联网，故用 `include_str!` 把仓库根目录的
//! `CHANGELOG.md` 在编译期内嵌进二进制（相对路径按**本文件所在目录**解析：
//! `src-tauri/src/commands/` → `../../../CHANGELOG.md`）。
//! 前端「版本与更新」据此离线展示当前版本的改动内容。

/// 仓库根目录 `CHANGELOG.md` 的编译期快照。
const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// 返回 CHANGELOG 全文（Markdown 原文）。
///
/// 内容在编译期已确定，读取不可能失败，故没有错误分支——保留 `Result` 只为与前端
/// `invoke` 的错误通道同形，调用方无需分支出错路径。
#[tauri::command]
pub fn get_changelog() -> Result<String, String> {
    Ok(CHANGELOG.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changelog_is_embedded() {
        assert!(!CHANGELOG.trim().is_empty());
    }

    #[test]
    fn changelog_has_release_sections() {
        // 版本段落标题形如 `## [0.2.16] - 2026-09-19`
        assert!(CHANGELOG.contains("## ["));
        // 开发中轮次标题为 `## [Unreleased]`：前端版本段落切分依赖它作为段落上界
        assert!(CHANGELOG.contains("[Unreleased]"));
    }
}
