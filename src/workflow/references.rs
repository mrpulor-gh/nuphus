//! Explicit field references shared by templates, conditions and loop inputs.
//! Bracket segments are JSON strings or non-negative array indices, never code.
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldReference {
    pub root: String,
    pub segments: Vec<Segment>,
}

pub fn parse_field_reference(text: &str) -> Option<FieldReference> {
    let (root, rest) = text.trim().split_once('[')?;
    let root = root.trim();
    if !root.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        || !root
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.'))
    {
        return None;
    }
    let mut tail = rest;
    let mut segments = Vec::new();
    loop {
        let mut stream = serde_json::Deserializer::from_str(tail.trim_start()).into_iter::<Value>();
        let segment = match stream.next()?.ok()? {
            Value::String(key) => Segment::Key(key),
            Value::Number(index) => {
                let index = index.as_u64()?;
                // Match JavaScript's safe integer range used by the editor.
                if index > 9_007_199_254_740_991 {
                    return None;
                }
                Segment::Index(usize::try_from(index).ok()?)
            }
            _ => return None,
        };
        tail = tail
            .trim_start()
            .get(stream.byte_offset()..)?
            .trim_start()
            .strip_prefix(']')?
            .trim_start();
        segments.push(segment);
        if tail.is_empty() {
            break;
        }
        tail = tail.strip_prefix('[')?;
    }
    Some(FieldReference {
        root: root.to_string(),
        segments,
    })
}

pub fn resolve_field_reference<'a>(
    reference: &FieldReference,
    variables: &'a HashMap<String, Value>,
) -> Option<&'a Value> {
    // Preserve the reserved inputs namespace and literal dotted capture names.
    let mut value = if let Some(path) = reference.root.strip_prefix("inputs.") {
        let mut value = variables.get("inputs")?;
        for key in path.split('.') {
            value = value.get(key)?;
        }
        value
    } else {
        variables.get(&reference.root)?
    };
    for segment in &reference.segments {
        value = match segment {
            Segment::Key(key) => value.as_object()?.get(key)?,
            Segment::Index(index) => value.as_array()?.get(*index)?,
        };
    }
    Some(value)
}

pub fn resolve<'a>(text: &str, variables: &'a HashMap<String, Value>) -> Option<&'a Value> {
    resolve_field_reference(&parse_field_reference(text)?, variables)
}

pub fn template_spans(text: &str) -> Vec<(usize, usize, &str)> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut offset = 0;
    while let Some(start) = text[offset..].find("{{").map(|start| start + offset) {
        let mut quoted = false;
        let mut escaped = false;
        let mut found = false;
        for i in start + 2..bytes.len().saturating_sub(1) {
            let c = bytes[i];
            if escaped {
                escaped = false;
                continue;
            }
            if quoted && c == b'\\' {
                escaped = true;
                continue;
            }
            if c == b'"' {
                quoted = !quoted;
                continue;
            }
            if !quoted && c == b'}' && bytes[i + 1] == b'}' {
                result.push((start, i + 2, &text[start + 2..i]));
                offset = i + 2;
                found = true;
                break;
            }
        }
        if !found {
            break;
        }
    }
    result
}

/// Check an expression without evaluating code. Legacy missing roots are errors only in
/// explicit debug runs; normal workflows retain their existing compatibility behavior.
fn check_expression(
    expression: &str,
    path: &str,
    variables: &HashMap<String, Value>,
    strict: bool,
) -> Result<(), String> {
    let expression = expression.trim();
    if let Some((head, _)) = expression.split_once('[') {
        if !head.trim().is_empty()
            && head
                .trim()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.'))
        {
            let reference = parse_field_reference(expression)
                .ok_or_else(|| format!("invalid_field_reference: {path}: {expression}"))?;
            if resolve_field_reference(&reference, variables).is_none() {
                return Err(format!("missing_field_reference: {path}: {expression}; refresh the source or supply a test value"));
            }
            return Ok(());
        }
    }
    if !strict {
        return Ok(());
    }
    let (name, pipe) = expression.split_once('|').unwrap_or((expression, ""));
    if pipe.trim().starts_with("default ") {
        return Ok(());
    }
    let name = name.trim();
    if name.starts_with("ENV:") {
        return Ok(());
    }
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@'))
    {
        return Ok(());
    }
    let present = variables.contains_key(name) || {
        let mut parts = name.split('.');
        let mut value = parts.next().and_then(|root| variables.get(root));
        for key in parts {
            value = value.and_then(|v| v.get(key));
        }
        value.is_some()
    };
    if !present {
        return Err(format!(
            "missing_test_variable: {path}: {name}; supply a value or choose execution data"
        ));
    }
    Ok(())
}

fn visit_references(
    value: &Value,
    path: &str,
    variables: &HashMap<String, Value>,
    strict: bool,
) -> Result<(), String> {
    match value {
        Value::String(text) => {
            if path.ends_with("/var")
                && (path.contains("/condition/")
                    || path.starts_with("/do/loop/until/")
                    || path == "/do/loop/for_each/items/var")
            {
                check_expression(text, path, variables, strict)?;
            }
            for (_, _, body) in template_spans(text) {
                check_expression(body, path, variables, strict)?;
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                visit_references(item, &format!("{path}/{index}"), variables, strict)?;
            }
        }
        Value::Object(items) => {
            for (key, item) in items {
                visit_references(
                    item,
                    &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                    variables,
                    strict,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Children and post-body until conditions are deliberately excluded at entry.
pub fn validate_step_fields(
    step: &super::types::Step,
    variables: &HashMap<String, Value>,
) -> Result<(), String> {
    let mut definition = serde_json::to_value(step).map_err(|error| error.to_string())?;
    for (parent, key) in [
        ("/do", "seq"),
        ("/do/loop", "do"),
        ("/do/loop", "until"),
        ("/do/if", "then"),
        ("/do/if", "else"),
        ("/do", "auto"),
    ] {
        if let Some(object) = definition
            .pointer_mut(parent)
            .and_then(Value::as_object_mut)
        {
            object.remove(key);
        }
    }
    visit_references(
        &definition["do"],
        "/do",
        variables,
        super::debug::current().is_some(),
    )
}

pub fn validate_until(
    condition: &super::types::Condition,
    variables: &HashMap<String, Value>,
) -> Result<(), String> {
    let condition = serde_json::to_value(condition).map_err(|error| error.to_string())?;
    visit_references(
        &condition,
        "/do/loop/until",
        variables,
        super::debug::current().is_some(),
    )
}

/// Declaration preflight for a selected subtree: unused workflow inputs should
/// not prevent testing an independent node. Conservative for legacy aliases.
pub fn uses_input(step: &super::types::Step, name: &str) -> bool {
    fn matches(expression: &str, name: &str) -> bool {
        if let Some(reference) = parse_field_reference(expression) {
            return reference.root == name
                || reference.root == format!("inputs.{name}")
                || reference.root == "inputs"
                    && reference.segments.first() == Some(&Segment::Key(name.into()));
        }
        let reference = expression.split('|').next().unwrap_or("").trim();
        reference == name
            || reference == format!("inputs.{name}")
            || reference.starts_with(&format!("inputs.{name}."))
    }
    fn visit(value: &Value, name: &str) -> bool {
        match value {
            Value::String(text) => template_spans(text)
                .iter()
                .any(|(_, _, body)| matches(body, name)),
            Value::Array(items) => items.iter().any(|value| visit(value, name)),
            Value::Object(items) => {
                items
                    .get("var")
                    .and_then(Value::as_str)
                    .is_some_and(|value| matches(value, name))
                    || items.values().any(|value| visit(value, name))
            }
            _ => false,
        }
    }
    serde_json::to_value(step).is_ok_and(|value| visit(&value, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fields_arrays_unicode_and_literal_dotted_names() {
        let vars = HashMap::from([
            (
                "wn".into(),
                json!({"window_id":123, "a.b": [{"含空 格":null}], "\"x]":true}),
            ),
            ("a.b".into(), json!({"v":"003"})),
            ("inputs".into(), json!({"items":[2]})),
        ]);
        assert_eq!(resolve(r#"wn["window_id"]"#, &vars), Some(&json!(123)));
        assert_eq!(
            resolve(r#"wn["a.b"][0]["含空 格"]"#, &vars),
            Some(&Value::Null)
        );
        assert_eq!(resolve(r#"wn["missing"]"#, &vars), None);
        assert_eq!(resolve(r#"wn["\"x]"]"#, &vars), Some(&json!(true)));
        assert_eq!(resolve(r#"a.b["v"]"#, &vars), Some(&json!("003")));
        assert_eq!(resolve(r#"inputs["items"][0]"#, &vars), Some(&json!(2)));
        assert_eq!(resolve(r#"inputs.items[0]"#, &vars), Some(&json!(2)));
    }

    #[test]
    fn rejects_executable_or_ambiguous_segments() {
        for invalid in [
            "x[-1]",
            "x[1.5]",
            "x[true]",
            "x[func()]",
            "x['key']",
            "x[0] trailing",
            "x[{}]",
            "x[0",
        ] {
            assert!(parse_field_reference(invalid).is_none(), "{invalid}");
        }
    }
}
