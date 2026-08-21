use crate::config::provider::*;
use crate::transports::chat_completions::{ChatCompletionsConfig, ChatCompletionsTransport};
use crate::transports::Transport;
use std::sync::Arc;

pub struct KimiProvider;

impl Provider for KimiProvider {
    fn id(&self) -> &'static str {
        "kimi"
    }
    fn display_name(&self) -> &'static str {
        "Kimi (Moonshot)"
    }
    fn default_base_url(&self) -> &'static str {
        "https://api.kimi.com/coding/v1"
    }
    fn auth_header(&self) -> &'static str {
        "x-api-key"
    }
    fn auth_prefix(&self) -> &'static str {
        ""
    }
    fn default_model(&self) -> &'static str {
        "kimi-for-coding"
    }

    fn models(&self) -> &'static [ModelDef] {
        &[ModelDef {
            id: "kimi-for-coding",
            aliases: &["kimi", "default"],
            context_window: 262_144,
            max_output_tokens: 8_192,
            supports_streaming: true,
            supports_vision: false,
            supports_reasoning: true,
            supports_audio: false,
            supports_image_generation: false,
            cost_per_million_in: 0.0,
            cost_per_million_out: 0.0,
            reasoning_field: "reasoning_content",
            reasoning_efforts: &[],
            default_effort: None,
        }]
    }

    fn quirks(&self) -> ProviderQuirks {
        ProviderQuirks {
            requires_reasoning_echo: true,
            supports_reasoning_effort: true,
            effort_excludes_tools: false,
            sanitize_tools: Some(self::sanitize_moonshot_tools),
            extra_headers: vec![],
            forbidden_request_fields: &[],
            max_tokens_field: MaxTokensField::MaxTokens,
            user_agent: Some(concat!("nuphus/", env!("CARGO_PKG_VERSION"))),
            content_tool_tags: &[],
            cache_hit_field: "",
        }
    }

    fn transport(&self, cfg: &ProviderConfig, model_id: &str) -> Arc<dyn Transport> {
        Arc::new(ChatCompletionsTransport::new(ChatCompletionsConfig {
            name: "kimi".to_string(),
            api_key: cfg.api_key.clone(),
            base_url: if cfg.base_url.is_empty() {
                self.default_base_url().to_string()
            } else {
                cfg.base_url.clone()
            },
            model: model_id.to_string(),
            timeout_secs: cfg.timeout_secs,
            auth_header: self.auth_header().to_string(),
            auth_prefix: self.auth_prefix().to_string(),
            provider_kind: Some(crate::api::ProviderKind::Kimi),
            quirks: self.quirks(),
            reasoning_effort: cfg.reasoning_effort.clone(),
        }))
    }
}

use crate::api::ToolDefinition;
use serde_json::{Map, Value};

/// Kimi / Moonshot model detection (moved from schema_fix.rs).
pub fn is_kimi_model(model: &str) -> bool {
    let bare = model.trim().to_lowercase();
    let tail = bare.rsplit('/').next().unwrap_or("");
    if tail.starts_with("kimi-") || tail == "kimi" {
        return true;
    }
    if bare.contains("moonshot") || bare.contains("/kimi") || bare.starts_with("kimi") {
        return true;
    }
    false
}

/// Apply Kimi/Moonshot-style tool schema sanitization to every tool's
/// `function.parameters`. Also normalises tool names (:: → _, . → _).
pub fn sanitize_moonshot_tools(tools: &[ToolDefinition]) -> Vec<ToolDefinition> {
    if tools.is_empty() {
        return tools.to_vec();
    }

    let mut sanitized = Vec::with_capacity(tools.len());
    let mut any_change = false;

    for tool in tools {
        let repaired_params = sanitize_moonshot_tool_parameters(&tool.function.parameters);
        let repaired_name = sanitize_tool_name(&tool.function.name);
        let name_changed = repaired_name != tool.function.name;
        let params_changed = repaired_params != tool.function.parameters;

        if name_changed || params_changed {
            any_change = true;
            let mut new_tool = tool.clone();
            new_tool.function.name = repaired_name;
            new_tool.function.parameters = repaired_params;
            sanitized.push(new_tool);
        } else {
            sanitized.push(tool.clone());
        }
    }

    if any_change {
        tracing::info!(
            "[tools] Sanitized {} tool names/schemas for API compatibility",
            sanitized.len()
        );
    }
    sanitized
}

fn sanitize_moonshot_tool_parameters(parameters: &Value) -> Value {
    if !parameters.is_object() {
        return serde_json::json!({"type": "object", "properties": {}});
    }

    let mut repaired = repair_schema(parameters.clone(), true);
    let obj = repaired
        .as_object_mut()
        .expect("repair_schema on object input always returns object");

    if obj.get("type").and_then(|t| t.as_str()) != Some("object") {
        obj.insert("type".to_string(), Value::String("object".to_string()));
    }
    if !obj.contains_key("properties") {
        obj.insert("properties".to_string(), Value::Object(Map::new()));
    }

    repaired
}

fn sanitize_tool_name(name: &str) -> String {
    name.replace("::", "_").replace(".", "_")
}

fn repair_schema(mut node: Value, is_schema: bool) -> Value {
    use serde_json::Value;
    if let Some(arr) = node.as_array_mut() {
        let repaired: Vec<Value> = arr
            .drain(..)
            .map(|item| repair_schema(item, true))
            .collect();
        return Value::Array(repaired);
    }

    let obj = match node.as_object_mut() {
        Some(o) => o,
        None => return node,
    };

    let keys: Vec<String> = obj.keys().cloned().collect();
    const SCHEMA_MAP_KEYS: &[&str] = &["properties", "patternProperties", "$defs", "definitions"];
    const SCHEMA_LIST_KEYS: &[&str] = &["anyOf", "oneOf", "allOf", "prefixItems"];
    const SCHEMA_NODE_KEYS: &[&str] = &[
        "items",
        "contains",
        "not",
        "additionalProperties",
        "propertyNames",
    ];

    for key in &keys {
        let value = obj
            .get_mut(key)
            .expect("key from obj.keys() iterator, must exist");
        if SCHEMA_MAP_KEYS.contains(&key.as_str()) && value.is_object() {
            let inner_map = value
                .as_object_mut()
                .expect("guarded by value.is_object() check above");
            for (_sub_key, sub_val) in inner_map.iter_mut() {
                *sub_val = repair_schema(std::mem::take(sub_val), true);
            }
        } else if SCHEMA_LIST_KEYS.contains(&key.as_str()) && value.is_array() {
            let arr = value
                .as_array_mut()
                .expect("guarded by value.is_array() check above");
            for item in arr.iter_mut() {
                *item = repair_schema(std::mem::take(item), true);
            }
        } else if key == "items" && value.is_array() {
            let arr = value
                .as_array_mut()
                .expect("guarded by value.is_array() check above");
            let first = if !arr.is_empty() {
                repair_schema(std::mem::take(&mut arr[0]), true)
            } else {
                Value::Object(Map::new())
            };
            *value = first;
        } else if SCHEMA_NODE_KEYS.contains(&key.as_str()) && value.is_object() {
            *value = repair_schema(std::mem::take(value), true);
        }
    }

    if !is_schema {
        return Value::Object(std::mem::take(obj));
    }

    if obj.contains_key("anyOf") {
        if let Some(any_of_arr) = obj.get("anyOf").and_then(|v| v.as_array()).cloned() {
            let any_of_len = any_of_arr.len();
            obj.remove("type");
            let non_null: Vec<Value> = any_of_arr
                .into_iter()
                .filter(|b| {
                    b.as_object()
                        .and_then(|o| o.get("type"))
                        .and_then(|t| t.as_str())
                        != Some("null")
                })
                .collect();
            let had_null = non_null.len() < any_of_len;
            if had_null && !non_null.is_empty() {
                if non_null.len() == 1 {
                    let mut merge = Map::new();
                    for (k, v) in obj.iter() {
                        if k != "anyOf" {
                            merge.insert(k.clone(), v.clone());
                        }
                    }
                    if let Some(branch) = non_null
                        .into_iter()
                        .next()
                        .and_then(|v| v.as_object().cloned())
                    {
                        for (k, v) in branch {
                            merge.insert(k, v);
                        }
                    }
                    return repair_schema(Value::Object(merge), true);
                } else {
                    obj.insert("anyOf".to_string(), Value::Array(non_null));
                    return Value::Object(std::mem::take(obj));
                }
            }
        }
    }

    obj.remove("nullable");

    if !obj.contains_key("$ref") {
        fill_missing_type(obj);
    }

    if let Some(enum_arr) = obj.get("enum").and_then(|v| v.as_array()).cloned() {
        let node_type = obj.get("type").and_then(|t| t.as_str());
        if matches!(node_type, Some("string" | "integer" | "number" | "boolean")) {
            let cleaned: Vec<Value> = enum_arr
                .into_iter()
                .filter(|v| !v.is_null() && v.as_str() != Some(""))
                .collect();
            if cleaned.is_empty() {
                obj.remove("enum");
            } else {
                obj.insert("enum".to_string(), Value::Array(cleaned));
            }
        }
    }

    if obj.contains_key("$ref") {
        let ref_val = obj
            .get("$ref")
            .expect("guarded by obj.contains_key(\"$ref\") above")
            .clone();
        let mut stripped = Map::new();
        stripped.insert("$ref".to_string(), ref_val);
        return Value::Object(stripped);
    }

    Value::Object(std::mem::take(obj))
}

fn fill_missing_type(node: &mut Map<String, Value>) {
    if node
        .get("type")
        .and_then(|t| t.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        return;
    }

    let inferred = if node.contains_key("properties")
        || node.contains_key("required")
        || node.contains_key("additionalProperties")
    {
        "object"
    } else if node.contains_key("items") || node.contains_key("prefixItems") {
        "array"
    } else if let Some(first) = node
        .get("enum")
        .and_then(|e| e.as_array())
        .and_then(|a| a.first())
    {
        if first.is_boolean() {
            "boolean"
        } else if first.is_i64() {
            "integer"
        } else if first.is_f64() {
            "number"
        } else {
            "string"
        }
    } else {
        "string"
    };

    node.insert("type".to_string(), Value::String(inferred.to_string()));
}
