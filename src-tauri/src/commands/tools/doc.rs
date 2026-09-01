// tools/doc.rs — 文档转文本（内部机制命令，非 agent 工具）
//
// 复用 nuphus core 的 office 读取链路（src/utils/office.rs read_office）：
// 支持 docx / pptx / xls / ods / odt / odp / pdf 文本提取。
// 不引入新依赖——office.rs 已在 nuphus crate 内（lib.rs pub mod utils）。

/// 提取文档文本（docx/pptx/xls/ods/odt/odp/pdf），返回 Markdown/纯文本。
/// 供工具页「文档转文本」能力使用；PDF 走既有三层降级（文本层 → lopdf → 渲染 OCR）。
#[tauri::command]
pub fn doc_extract_text(path: String) -> Result<serde_json::Value, String> {
    match nuphus::utils::office::read_office(&path) {
        Some(Ok(text)) => {
            let chars = text.chars().count();
            Ok(serde_json::json!({
                "path": path,
                "chars": chars,
                "text": text,
            }))
        }
        Some(Err(e)) => Err(format!("提取文档文本失败：{}", e)),
        None => {
            Err("不支持的文档格式（支持 docx / pptx / xls / ods / odt / odp / pdf）".to_string())
        }
    }
}
