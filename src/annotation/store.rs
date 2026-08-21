//! AnnotationStore — relationship annotation persistent storage
//!
//! Storage location: `~/.nuphus/annotations/annotations.json`
//! Automatically writes built-in preset annotations on first launch.
//! Structure has version field, supports forward compatibility.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;

use super::presets;
use super::types::{Annotation, AnnotationStoreData, MatchLevel};

// ── Global config ──

static STORE_PATH: OnceLock<PathBuf> = OnceLock::new();

/// In-memory cache: avoids reading disk every process() cycle
/// Cleared after write operations (add/update/remove/reset), reloaded on next read
static DATA_CACHE: OnceLock<Mutex<Option<AnnotationStoreData>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<AnnotationStoreData>> {
    DATA_CACHE.get_or_init(|| Mutex::new(None))
}

fn invalidate_cache() {
    if let Ok(mut guard) = cache().lock() {
        *guard = None;
    }
}

fn load_data_cached() -> AnnotationStoreData {
    // Use cache if available
    if let Ok(guard) = cache().lock() {
        if let Some(ref data) = *guard {
            return data.clone();
        }
    }
    // No cache, read from disk
    let data = load_data_inner();
    if let Ok(mut guard) = cache().lock() {
        *guard = Some(data.clone());
    }
    data
}

fn store_path() -> &'static PathBuf {
    STORE_PATH.get_or_init(|| {
        let path = dirs::data_dir()
            .map(|d| d.join(".nuphus").join("annotations"))
            .unwrap_or_else(|| {
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".nuphus").join("annotations")
            });
        if let Err(e) = std::fs::create_dir_all(&path) {
            tracing::warn!("[AnnotationStore] failed to create dir: {}", e);
        }
        path.join("annotations.json")
    })
}

/// Initialize early in Tauri setup (override default path for testing)
pub fn init_store_path(path: PathBuf) {
    let _ = STORE_PATH.set(path);
}

// ── Data load and save ──

fn load_data_inner() -> AnnotationStoreData {
    let path = store_path();
    if !path.exists() {
        // First launch: create default data + write built-in presets
        let mut data = AnnotationStoreData {
            annotations: presets::get_builtins(),
            ..Default::default()
        };
        data.metadata.count = data.annotations.len();
        data.metadata.updated_at = chrono::Utc::now();
        save_data(&data);
        tracing::info!(
            "[AnnotationStore] initialized with {} built-in annotations",
            data.annotations.len()
        );
        return data;
    }
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<AnnotationStoreData>(&content) {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!(
                    "[AnnotationStore] parse error: {}. Falling back to defaults.",
                    e
                );
                let mut data = AnnotationStoreData {
                    annotations: presets::get_builtins(),
                    ..Default::default()
                };
                data.metadata.count = data.annotations.len();
                data
            }
        },
        Err(e) => {
            tracing::warn!("[AnnotationStore] read error: {}", e);
            AnnotationStoreData::default()
        }
    }
}

fn save_data(data: &AnnotationStoreData) {
    let path = store_path();
    if let Err(e) = std::fs::create_dir_all(
        path.parent()
            .expect("store_path() returns file path with parent directory"),
    ) {
        tracing::warn!("[AnnotationStore] failed to create dir: {}", e);
        return;
    }
    let json = serde_json::to_string_pretty(data).unwrap_or_default();
    if let Err(e) = std::fs::write(path, &json) {
        tracing::warn!("[AnnotationStore] write error: {}", e);
    }
}

fn load_all() -> Vec<Annotation> {
    load_data_cached().annotations
}

fn save_all(annotations: Vec<Annotation>) {
    let mut data = AnnotationStoreData {
        annotations,
        ..Default::default()
    };
    data.metadata.count = data.annotations.len();
    data.metadata.updated_at = chrono::Utc::now();
    save_data(&data);
    invalidate_cache(); // Invalidate cache after write, reload on next read
}

// ── Public API ──

pub struct AnnotationStore;

impl AnnotationStore {
    /// Add new annotation (keyword dedup: same primary keyword treated as update)
    pub fn add(
        keyword: &str,
        description: &str,
        keywords: Vec<String>,
        paths: Vec<String>,
        tags: Vec<String>,
        group: Option<String>,
        priority: Option<i32>,
        memory_entry_id: Option<String>,
    ) -> Result<Annotation, String> {
        if keyword.is_empty() || description.is_empty() {
            return Err("keyword and description are required".into());
        }
        let mut all = load_all();
        // Same keyword treated as update
        if let Some(existing) = all.iter_mut().find(|a| a.keyword == keyword) {
            existing.description = description.to_string();
            existing.keywords = keywords;
            existing.paths = paths;
            existing.tags = tags;
            if let Some(g) = group {
                existing.group = g;
            }
            if let Some(p) = priority {
                existing.priority = p;
            }
            if let Some(meid) = memory_entry_id {
                existing.memory_entry_id = Some(meid);
            }
            existing.updated_at = chrono::Utc::now();
            let ann = existing.clone();
            save_all(all);
            return Ok(ann);
        }
        let mut ann = Annotation::new(
            keyword.to_string(),
            description.to_string(),
            paths,
            tags,
            group.unwrap_or_else(|| "custom".into()),
            false,
            priority.unwrap_or(0),
        );
        ann.keywords = keywords;
        ann.memory_entry_id = memory_entry_id;
        all.push(ann.clone());
        save_all(all);
        Ok(ann)
    }

    /// Update existing annotation
    pub fn update(
        keyword: &str,
        description: Option<&str>,
        paths: Option<Vec<String>>,
        tags: Option<Vec<String>>,
        group: Option<String>,
        priority: Option<i32>,
    ) -> Result<Annotation, String> {
        let mut all = load_all();
        let existing = all
            .iter_mut()
            .find(|a| a.keyword == keyword)
            .ok_or_else(|| format!("Annotation '{}' not found", keyword))?;
        if let Some(desc) = description {
            existing.description = desc.to_string();
        }
        if let Some(p) = paths {
            existing.paths = p;
        }
        if let Some(t) = tags {
            existing.tags = t;
        }
        if let Some(g) = group {
            existing.group = g;
        }
        if let Some(p) = priority {
            existing.priority = p;
        }
        existing.updated_at = chrono::Utc::now();
        let ann = existing.clone();
        save_all(all);
        Ok(ann)
    }

    /// Remove annotation (builtin annotations cannot be removed)
    pub fn remove(keyword: &str) -> Result<(), String> {
        let mut all = load_all();
        let len_before = all.len();
        all.retain(|a| a.keyword != keyword || a.builtin);
        if all.len() == len_before {
            return Err(format!("Annotation '{}' not found or is built-in", keyword));
        }
        save_all(all);
        Ok(())
    }

    /// List all annotations
    pub fn list() -> Vec<Annotation> {
        load_all()
    }

    /// Search by keyword (case-insensitive, checks all keywords)
    pub fn search(keyword: &str) -> Vec<Annotation> {
        let lower = keyword.to_lowercase();
        load_all()
            .into_iter()
            .filter(|a| {
                let kw = a.keyword.to_lowercase();
                if kw.contains(&lower) || lower.contains(&kw) {
                    return true;
                }
                a.keywords.iter().any(|k| {
                    let kl = k.to_lowercase();
                    kl.contains(&lower) || lower.contains(&kl)
                })
            })
            .collect()
    }

    /// List by group
    pub fn list_by_group(group: &str) -> Vec<Annotation> {
        let all = load_all();
        all.into_iter().filter(|a| a.group == group).collect()
    }

    /// Find by keyword (fuzzy match, sorted by match quality, max 8 results)
    pub fn find_by_keyword(input: &str) -> Vec<Annotation> {
        let lower = input.to_lowercase();
        let all = load_all();
        let mut matched: Vec<(Annotation, i32)> = all
            .into_iter()
            .filter_map(|a| {
                let priority = a.priority;
                let level = a.matches(&lower);
                match level {
                    MatchLevel::None => None,
                    _ => Some((a, level as i32 * 100 + priority)),
                }
            })
            .collect();

        // Sort by match quality + priority
        matched.sort_by_key(|a| std::cmp::Reverse(a.1));

        matched.into_iter().take(8).map(|(a, _)| a).collect()
    }

    /// Get all keyword list (for prompt generation)
    pub fn keyword_list() -> Vec<String> {
        load_all().into_iter().map(|a| a.keyword).collect()
    }

    /// Reset all builtin annotations to factory state (clear user modifications, but do not delete user custom ones)
    pub fn reset_builtins() -> Result<usize, String> {
        let mut all = load_all();
        // Delete existing builtins
        all.retain(|a| !a.builtin);
        // Re-insert factory presets
        let builtins = presets::get_builtins();
        let count = builtins.len();
        all.extend(builtins);
        save_all(all);
        Ok(count)
    }

    /// Get total annotation count
    pub fn count() -> usize {
        load_all().len()
    }
}
