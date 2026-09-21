//! Workflow external-input contract validation and resolution.

use crate::workflow::types::{InputKind, InputSpec};
use std::collections::{HashMap, HashSet};

/// Names owned by the executor and therefore unavailable to external inputs.
const RESERVED_NAMES: &[&str] = &["inputs", "params", "_index"];

pub fn value_matches_kind(kind: InputKind, value: &serde_json::Value) -> bool {
    match kind {
        InputKind::String | InputKind::Path => value.is_string(),
        InputKind::Number => value.is_number(),
        InputKind::Boolean => value.is_boolean(),
        InputKind::Json => true,
    }
}

fn kind_label(kind: InputKind) -> &'static str {
    match kind {
        InputKind::String => "string",
        InputKind::Number => "number",
        InputKind::Boolean => "boolean",
        InputKind::Path => "path",
        InputKind::Json => "json",
    }
}

/// Validate the declarations themselves. Errors are stable, user-facing strings.
pub fn validate_specs(specs: &[InputSpec]) -> Vec<String> {
    let name_re = regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$")
        .expect("input name regex is statically valid");
    let mut errors = Vec::new();
    let mut seen = HashSet::new();

    for spec in specs {
        if !name_re.is_match(&spec.name) {
            errors.push(format!(
                "输入声明名称 '{}' 非法（必须匹配 [A-Za-z_][A-Za-z0-9_]*）",
                spec.name
            ));
        }
        if RESERVED_NAMES.contains(&spec.name.as_str()) {
            errors.push(format!("输入声明名称 '{}' 是保留名", spec.name));
        }
        if !seen.insert(spec.name.as_str()) {
            errors.push(format!("重复的输入声明名称: '{}'", spec.name));
        }
        if let Some(default) = &spec.default {
            if !value_matches_kind(spec.kind, default) {
                errors.push(format!(
                    "输入 '{}': 默认值类型必须是 {}",
                    spec.name,
                    kind_label(spec.kind)
                ));
            }
        }
    }
    errors
}

/// Resolve declared inputs using explicit value > declaration default > absent optional.
/// Undeclared values are deliberately ignored here; callers that preserve legacy top-level
/// injection can still use the original `provided` map after this function succeeds.
pub fn resolve_declared_inputs(
    specs: &[InputSpec],
    provided: &HashMap<String, serde_json::Value>,
) -> crate::Result<serde_json::Map<String, serde_json::Value>> {
    let mut resolved = serde_json::Map::new();
    for spec in specs {
        let value = provided.get(&spec.name).or(spec.default.as_ref());
        match value {
            Some(value) if value_matches_kind(spec.kind, value) => {
                resolved.insert(spec.name.clone(), value.clone());
            }
            Some(_) => {
                return Err(crate::NuphusError::agent(format!(
                    "输入 '{}': 值类型必须是 {}",
                    spec.name,
                    kind_label(spec.kind)
                )));
            }
            None if spec.required => {
                return Err(crate::NuphusError::agent(format!(
                    "缺少必填输入：{}",
                    spec.name
                )));
            }
            None => {}
        }
    }
    Ok(resolved)
}

/// Reject keys not present in the target workflow declaration, then resolve the contract.
/// Used by persisted bindings such as schedules where compatibility-only ad-hoc keys are unsafe.
pub fn resolve_declared_only(
    specs: &[InputSpec],
    provided: &HashMap<String, serde_json::Value>,
) -> crate::Result<serde_json::Map<String, serde_json::Value>> {
    let declared: HashSet<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
    if let Some(name) = provided
        .keys()
        .find(|name| !declared.contains(name.as_str()))
    {
        return Err(crate::NuphusError::agent(format!(
            "输入 '{}' 未在目标工作流中声明",
            name
        )));
    }
    resolve_declared_inputs(specs, provided)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, kind: InputKind) -> InputSpec {
        InputSpec {
            name: name.into(),
            kind,
            required: false,
            default: None,
            description: None,
            sensitive: false,
        }
    }

    #[test]
    fn declaration_names_duplicates_reserved_and_defaults_are_validated() {
        let mut bad_default = spec("count", InputKind::Number);
        bad_default.default = Some(serde_json::json!("one"));
        let errors = validate_specs(&[
            spec("bad-name", InputKind::String),
            spec("inputs", InputKind::Json),
            spec("same", InputKind::String),
            spec("same", InputKind::String),
            bad_default,
        ]);
        assert_eq!(errors.len(), 4, "{errors:?}");
    }

    #[test]
    fn runtime_types_and_required_values_are_checked() {
        let mut required = spec("enabled", InputKind::Boolean);
        required.required = true;
        let err = resolve_declared_inputs(&[required.clone()], &HashMap::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("缺少必填输入"));

        let provided = HashMap::from([("enabled".into(), serde_json::json!("yes"))]);
        let err = resolve_declared_inputs(&[required], &provided)
            .unwrap_err()
            .to_string();
        assert!(err.contains("boolean"));
    }

    #[test]
    fn undeclared_values_are_compatible_for_runs_but_rejected_for_bindings() {
        let provided = HashMap::from([("legacy".into(), serde_json::json!(1))]);
        assert!(resolve_declared_inputs(&[], &provided).unwrap().is_empty());
        assert!(resolve_declared_only(&[], &provided).is_err());
    }
}
