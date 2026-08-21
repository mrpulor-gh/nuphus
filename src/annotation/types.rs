//! Annotation — relationship annotation data types
//!
//! Replaces old `src/memory/annotation.rs`, as an independent product module.
//! keyword -> {description, file path list, tags, group} mapping,
//! Used for Leader prompt injection, inter-Agent routing, context retrieval.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Relationship annotation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    /// Unique identifier (UUID)
    pub id: String,
    /// Trigger keyword (primary)
    pub keyword: String,
    /// Additional trigger keywords (multi-keyword support)
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Relationship description
    pub description: String,
    /// Related file paths
    #[serde(default)]
    pub paths: Vec<String>,
    /// Classification tags (for future knowledge graph/search filtering)
    #[serde(default)]
    pub tags: Vec<String>,
    /// Functional group (system / user / custom)
    #[serde(default = "default_group")]
    pub group: String,
    /// Whether it's distribution built-in (users cannot delete builtin annotations, but can override)
    #[serde(default)]
    pub builtin: bool,
    /// Association priority (higher values rank earlier when matching)
    #[serde(default)]
    pub priority: i32,
    /// Related annotation ID list (knowledge graph reserved: annotation -> annotation link)
    #[serde(default)]
    pub relations: Vec<String>,
    /// Linked memory entry ID (for annotation→memory cross-reference)
    #[serde(default)]
    pub memory_entry_id: Option<String>,
    /// Created at
    pub created_at: DateTime<Utc>,
    /// Updated at
    pub updated_at: DateTime<Utc>,
}

fn default_group() -> String {
    "custom".to_string()
}

impl Annotation {
    pub fn new(
        keyword: String,
        description: String,
        paths: Vec<String>,
        tags: Vec<String>,
        group: String,
        builtin: bool,
        priority: i32,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            keyword,
            keywords: Vec::new(),
            description,
            paths,
            tags,
            group,
            builtin,
            priority,
            relations: Vec::new(),
            memory_entry_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Determine if keyword(s) match input text (word boundary priority + substring fallback)
    pub fn matches(&self, input: &str) -> MatchLevel {
        let lower = input.to_lowercase();
        let mut best = MatchLevel::None;

        // Check primary keyword
        best = best.max(Self::match_one(&lower, &self.keyword.to_lowercase()));

        // Check additional keywords
        for kw in &self.keywords {
            best = best.max(Self::match_one(&lower, &kw.to_lowercase()));
            if best == MatchLevel::Exact {
                break;
            }
        }

        best
    }

    /// Single keyword match helper
    fn match_one(lower_input: &str, kw: &str) -> MatchLevel {
        // Exact match (highest priority)
        if lower_input == kw {
            return MatchLevel::Exact;
        }
        // Word boundary match
        if is_word_boundary_match(lower_input, kw) {
            return MatchLevel::WordBoundary;
        }
        // Substring match (only when keyword length >= 2 for flexibility)
        if kw.len() >= 2 && lower_input.contains(kw) {
            return MatchLevel::Substring;
        }
        MatchLevel::None
    }
}

/// Match quality level
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchLevel {
    None = 0,
    Substring = 1,
    WordBoundary = 2,
    Exact = 3,
}

/// Check if keyword appears in text at word boundary
fn is_word_boundary_match(text: &str, keyword: &str) -> bool {
    if keyword.is_empty() {
        return false;
    }
    let mut search_start = 0;
    while let Some(pos) = text[search_start..].find(keyword) {
        let abs_pos = search_start + pos;
        let before = abs_pos == 0
            || !text[..abs_pos]
                .chars()
                .last()
                .is_some_and(|c| c.is_alphanumeric());
        let after = abs_pos + keyword.len() >= text.len()
            || !text[abs_pos + keyword.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric());
        if before && after {
            return true;
        }
        search_start = abs_pos + 1;
        while search_start < text.len() && !text.is_char_boundary(search_start) {
            search_start += 1;
        }
    }
    false
}

/// Annotation container — top-level structure of JSON file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationStoreData {
    /// Format version
    pub version: i32,
    /// Metadata
    pub metadata: AnnotationMetadata,
    /// Annotation list
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

impl Default for AnnotationStoreData {
    fn default() -> Self {
        Self {
            version: 1,
            metadata: AnnotationMetadata::default(),
            annotations: Vec::new(),
        }
    }
}

/// File metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationMetadata {
    /// Last updated at
    pub updated_at: DateTime<Utc>,
    /// Total annotation count
    pub count: usize,
}

impl Default for AnnotationMetadata {
    fn default() -> Self {
        Self {
            updated_at: Utc::now(),
            count: 0,
        }
    }
}
