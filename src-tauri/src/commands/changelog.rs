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

    /// 只校验「段落形状」，不断言某个特定标题名存在。
    ///
    /// 前端 `parseChangelogSection`（frontend/src/main-window/lib/changelog.ts:47-58）
    /// 的切分规则是「`## [版本]` 起，到**下一个 `## [` 行**止」——与段名叫什么无关。
    /// 因此这里若写 `assert!(CHANGELOG.contains("[Unreleased]"))`，就会把测试与发版惯例
    /// 耦死：Keep a Changelog 的惯例正是发版时把 `[Unreleased]` 改名为 `[版本] - 日期`，
    /// 结果每次发版后 cargo test 必红，且红因与任何 PR 的内容无关（issue #86）。
    ///
    /// 「发版后补一个空 `## [Unreleased]`」仍是本仓库的发版惯例（用户看得到"开发中"轮次），
    /// 但它不再是测试的前置条件。
    #[test]
    fn changelog_sections_are_well_formed() {
        // 至少一个版本段落标题
        assert!(
            CHANGELOG.contains("## ["),
            "CHANGELOG 应含至少一个 `## [版本]` 段落"
        );

        // 每个二级标题都是 `## [<非空版本>]` 形状——与前端 SECTION_RE 同构，
        // 这是段落切分真正依赖的契约（标题名本身不参与切分）。
        let headings: Vec<&str> = CHANGELOG.lines().filter(|l| l.starts_with("## ")).collect();
        assert!(!headings.is_empty(), "CHANGELOG 应含至少一个 `##` 段落标题");
        for h in &headings {
            let inner = h
                .strip_prefix("## ")
                .and_then(|rest| rest.strip_prefix('['))
                .and_then(|rest| rest.split(']').next())
                .unwrap_or_default()
                .trim();
            assert!(
                !inner.is_empty(),
                "`##` 段落标题必须是 `## [<版本>]` 形状，当前为: {h}"
            );
        }
        // 末段落以 EOF 收尾即合法：前端 end 默认为行数，Keep a Changelog 同样容忍。
    }
}
