//! annotations — Tauri annotation CRUD 命令
//!
//! 对应 nuphus::annotation::store::AnnotationStore 的静态度方法。

use nuphus::annotation::types::Annotation;

#[tauri::command]
pub fn get_annotations() -> Result<Vec<Annotation>, String> {
    Ok(nuphus::annotation::store::AnnotationStore::list())
}

#[tauri::command]
pub fn add_annotation(
    keyword: String,
    description: String,
    keywords: Option<Vec<String>>,
    paths: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    group: Option<String>,
    priority: Option<i32>,
) -> Result<Annotation, String> {
    nuphus::annotation::store::AnnotationStore::add(
        &keyword,
        &description,
        keywords.unwrap_or_default(),
        paths.unwrap_or_default(),
        tags.unwrap_or_default(),
        group,
        priority,
        None, // memory_entry_id — set via tool, not via Tauri
    )
}

#[tauri::command]
pub fn update_annotation(
    keyword: String,
    description: Option<String>,
    paths: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    group: Option<String>,
    priority: Option<i32>,
) -> Result<Annotation, String> {
    nuphus::annotation::store::AnnotationStore::update(
        &keyword,
        description.as_deref(),
        paths,
        tags,
        group,
        priority,
    )
}

#[tauri::command]
pub fn remove_annotation(keyword: String) -> Result<(), String> {
    nuphus::annotation::store::AnnotationStore::remove(&keyword)
}
