//! 数据目录（设置中心「数据目录」分区）—— 只读列举本机各数据目录的真实路径。
//!
//! 定位：让用户知道自己的数据落在磁盘哪里（排障 / 备份 / 手动查看），
//! **不提供修改、不提供迁移**。因此本模块只做「路径解析 + 存在性判定」，
//! 不统计体积、不递归扫描、不缓存结果——`%APPDATA%\Nuphus` 实测有数千文件，
//! 递归 sum 是 IO 密集操作，而体积数字没有对应的用户动作（见运行时宪法：零扫描）。
//!
//! 条目的路径解析全部复用各功能已有的权威实现，本模块不重复推导：
//! - `data`      → `nuphus_data_dir()` 同源的 `dirs::data_dir()/Nuphus`
//!   （models / browser_profile_v2 / dicts / tools / nuphus.db / providers.toml）
//! - `runtime`   → `nuphus::utils::nuphus_data_dir()`（annotations / memory / tasks / workflows）
//! - `generated` → `dirs::home_dir()/.nuphus/generated`（`tools/builtin/generation.rs` 的产出目录）
//! - `plugin`    → `nuphus::utils::plugin_root()`（skills / knowledge / workflows / apps）
//! - `config`    → `get_config_path()` 所在目录（providers.toml 等配置文件的父目录）
//!
//! `data` 与 `runtime` 在默认配置下同盘同源，但语义不同（前者是各功能实现各自
//! 拼出的数据落点，后者是运行时写入的权威目录，且可被 `NUPHUS_DATA_DIR` 覆盖）；
//! 一律按权威实现分别解析，不合并——否则环境变量覆盖时会给出错误路径。

use serde::Serialize;

/// 单个数据目录条目的展示契约。
///
/// `path` 为空串表示该目录在本机无法解析（例如配置文件尚不存在）；
/// 此时 `exists` 恒为 false，前端据此标记「未创建」并隐藏「打开」按钮。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DataDirEntry {
    /// 稳定标识（前端 i18n key 的组成部分，不走 IPC 传中文名）
    pub key: String,
    /// 绝对路径（不可解析时为空串）
    pub path: String,
    /// 当前是否存在（不递归、不统计内容）
    pub exists: bool,
}

/// 条目顺序 = 前端展示顺序：主数据目录 → 运行时 → 产出 → 插件 → 配置。
const DATA_DIR_KEYS: [&str; 5] = ["data", "runtime", "generated", "plugin", "config"];

/// 主数据目录：与 `dict_ocr` / `models::bootstrap` / `store::db` 的路径拼法同源
/// （`dirs::data_dir()/{Nuphus|nuphus}`，Windows 大小写不敏感，实为同一目录）。
fn data_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|base| base.join("Nuphus"))
}

/// 生成产物目录：`~/.nuphus/generated`（`tools/builtin/generation.rs::generated_dir` 同源）。
/// 这里不调用 `generated_dir()` —— 那个函数会 `create_dir_all`，列举页面不应有写副作用。
fn generated_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|home| home.join(".nuphus").join("generated"))
}

/// 配置文件所在目录：`get_config_path()` 返回的是文件路径，取父目录。
/// 配置文件尚不存在（全新安装）→ None，条目留空并标记不存在。
fn config_dir() -> Option<std::path::PathBuf> {
    super::toml_ops::get_config_path().and_then(|p| p.parent().map(std::path::Path::to_path_buf))
}

/// 按 key 解析目录（未识别的 key → None）。
fn resolve_dir(key: &str) -> Option<std::path::PathBuf> {
    match key {
        "data" => data_dir(),
        "runtime" => Some(nuphus::utils::nuphus_data_dir()),
        "generated" => generated_dir(),
        "plugin" => Some(nuphus::utils::plugin_root()),
        "config" => config_dir(),
        _ => None,
    }
}

/// 从已解析的目录构造条目：解析失败 → 空路径 + 不存在。
fn entry_from(key: &str, dir: Option<std::path::PathBuf>) -> DataDirEntry {
    match dir {
        Some(dir) => DataDirEntry {
            key: key.to_string(),
            path: dir.to_string_lossy().to_string(),
            exists: dir.exists(),
        },
        None => DataDirEntry {
            key: key.to_string(),
            path: String::new(),
            exists: false,
        },
    }
}

/// 列举全部数据目录（只读：仅路径解析 + `Path::exists`，无目录扫描/无副作用）。
#[tauri::command]
pub fn list_data_dirs() -> Result<Vec<DataDirEntry>, String> {
    Ok(DATA_DIR_KEYS
        .iter()
        .map(|key| entry_from(key, resolve_dir(key)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn entry_marks_existing_dir_with_absolute_path() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let entry = entry_from("data", Some(dir.clone()));
        assert_eq!(entry.key, "data");
        assert_eq!(entry.path, dir.to_string_lossy());
        assert!(entry.exists);
    }

    #[test]
    fn entry_marks_missing_dir_as_not_existing() {
        let dir = std::env::temp_dir()
            .join("nuphus_data_dirs_test_missing")
            .join("never_created");
        let entry = entry_from("generated", Some(dir.clone()));
        assert_eq!(entry.path, dir.to_string_lossy());
        assert!(!entry.exists, "不存在的目录必须标记 exists=false 而非隐藏");
    }

    /// 解析失败（如配置尚未落盘）→ 空路径 + 不存在，而不是让整条命令报错。
    #[test]
    fn unresolvable_dir_yields_empty_path() {
        let entry = entry_from("config", None);
        assert_eq!(entry.key, "config");
        assert!(entry.path.is_empty());
        assert!(!entry.exists);
    }

    #[test]
    fn unknown_key_does_not_resolve() {
        assert!(resolve_dir("repo").is_none());
        assert!(resolve_dir("").is_none());
    }

    /// 契约：key 集合稳定且无重复（前端 i18n key 依赖它）。
    #[test]
    fn keys_are_stable_and_unique() {
        let mut keys: Vec<&str> = DATA_DIR_KEYS.to_vec();
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "DATA_DIR_KEYS 存在重复项");
        assert_eq!(
            DATA_DIR_KEYS,
            ["data", "runtime", "generated", "plugin", "config"]
        );
    }
}
