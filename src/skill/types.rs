use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, alias = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub triggers: SkillTriggers,
    #[serde(default, deserialize_with = "deserialize_data_sources")]
    pub data_sources: Vec<DataSource>,
    #[serde(default)]
    pub requires_tools: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
}

/// 反序列化 data_sources：容忍 `["str",...]` 简写和 `[{...},...]` 完整格式
fn deserialize_data_sources<'de, D>(deserializer: D) -> Result<Vec<DataSource>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Item {
        Full(DataSource),
        Simple(String),
    }
    let items: Vec<Item> = Vec::deserialize(deserializer)?;
    Ok(items
        .into_iter()
        .map(|item| match item {
            Item::Full(ds) => ds,
            Item::Simple(s) => DataSource {
                data_type: String::new(),
                path: s,
                description: String::new(),
            },
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillTriggers {
    #[serde(default)]
    pub auto_suggest: bool,
    #[serde(default)]
    pub context_hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataSource {
    #[serde(default)]
    pub data_type: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub name: String,
    pub path: String,
    pub version: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub installed_at: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillQueryInput {
    pub query: String,
    pub skill: Option<String>,
    pub domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillQueryHit {
    pub skill: String,
    pub title: String,
    pub content: String,
    pub relevance: f32,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillQueryOutput {
    pub hits: Vec<SkillQueryHit>,
    pub total: usize,
}
