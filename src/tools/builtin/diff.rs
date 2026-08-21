//! file_diff — 文件差异对比工具
//!
//! 基于 similar crate 的 TextDiff，输出 unified diff 格式。

use similar::TextDiff;
use std::fs;

/// 生成两个文件的 unified diff。
///
/// 参数：
/// - original_path: 源文件路径
/// - modified_path: 修改后文件路径
/// - context_lines: 上下文行数（可选，默认 3）
pub fn file_diff(
    original_path: &str,
    modified_path: &str,
    context_lines: usize,
) -> Result<String, String> {
    let original = fs::read_to_string(original_path)
        .map_err(|e| format!("无法读取源文件 '{}': {}", original_path, e))?;
    let modified = fs::read_to_string(modified_path)
        .map_err(|e| format!("无法读取修改后文件 '{}': {}", modified_path, e))?;

    if original == modified {
        return Ok("（文件内容相同，无差异）".to_string());
    }

    let diff = TextDiff::from_lines(&original, &modified);

    let mut output = Vec::new();
    diff.unified_diff()
        .context_radius(context_lines)
        .header(original_path, modified_path)
        .to_writer(&mut output)
        .map_err(|e| format!("diff 生成失败: {}", e))?;

    String::from_utf8(output).map_err(|e| format!("diff 输出编码错误: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_file_diff_basic() {
        let dir = std::env::temp_dir();
        let a = dir.join("nuphus_diff_a.txt");
        let b = dir.join("nuphus_diff_b.txt");

        {
            let mut f = std::fs::File::create(&a).unwrap();
            f.write_all(b"line1\nline2\nline3\n").unwrap();
        }
        {
            let mut f = std::fs::File::create(&b).unwrap();
            f.write_all(b"line1\nline2_modified\nline3\n").unwrap();
        }

        let result = file_diff(a.to_str().unwrap(), b.to_str().unwrap(), 3).unwrap();
        assert!(result.contains("---"));
        assert!(result.contains("+++"));
        assert!(result.contains("line2_modified"));

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn test_file_diff_identical() {
        let dir = std::env::temp_dir();
        let a = dir.join("nuphus_diff_same.txt");

        {
            let mut f = std::fs::File::create(&a).unwrap();
            f.write_all(b"same content\n").unwrap();
        }

        let result = file_diff(a.to_str().unwrap(), a.to_str().unwrap(), 3).unwrap();
        assert!(result.contains("无差异"));

        let _ = std::fs::remove_file(&a);
    }
}
