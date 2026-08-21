//! 变量解析：模板替换与管道变换
use super::*;

impl Executor {
    // ── Helpers ──

    /// 类型自动推断：纯数字字符串 → Number，"true"/"false" → Bool，null → 空字符串，其余保持原样
    fn coerce_value(val: serde_json::Value) -> serde_json::Value {
        match &val {
            serde_json::Value::Null => serde_json::Value::String(String::new()),
            serde_json::Value::String(s) => {
                let trimmed = s.trim();
                // 整数
                if let Ok(n) = trimmed.parse::<i64>() {
                    return serde_json::Value::Number(serde_json::Number::from(n));
                }
                // 浮点数
                if let Ok(n) = trimmed.parse::<f64>() {
                    if let Some(num) = serde_json::Number::from_f64(n) {
                        return serde_json::Value::Number(num);
                    }
                }
                // 布尔
                if trimmed.eq_ignore_ascii_case("true") {
                    return serde_json::Value::Bool(true);
                }
                if trimmed.eq_ignore_ascii_case("false") {
                    return serde_json::Value::Bool(false);
                }
                val
            }
            _ => val,
        }
    }

    /// 对 params JSON 做 {{var}} 模板替换
    /// 支持管道：{{var | json "key"}} 提取 JSON 字段，{{var | len}} 取长度
    pub(super) fn resolve_vars(
        params: &serde_json::Value,
        vars: &HashMap<String, serde_json::Value>,
    ) -> serde_json::Value {
        match params {
            serde_json::Value::String(s) => {
                if s.starts_with("{{") && s.ends_with("}}") {
                    let inner = &s[2..s.len() - 2].trim();
                    // 管道表达式：{{var | op arg}}
                    if inner.contains('|') {
                        let mut parts = inner.splitn(2, '|');
                        let var_name = parts.next().unwrap_or("").trim();
                        let pipe_expr = parts.next().unwrap_or("").trim();
                        // ENV: 前缀 → 从环境变量取值（支持管道如 {{ENV:HOME | default "~/nuphus"}}）
                        let val = if let Some(env_name) = var_name.strip_prefix("ENV:") {
                            std::env::var(env_name).ok().map(serde_json::Value::String)
                        } else {
                            vars.get(&var_name.to_string()).cloned()
                        };
                        return Self::apply_pipe(val, pipe_expr);
                    }
                    // 纯变量：{{var}}
                    if inner
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '@')
                    {
                        if let Some(val) = vars.get(&inner.to_string()) {
                            return Self::coerce_value(val.clone());
                        }
                    }
                    // ENV: 环境变量引用：{{ENV:HOME}}
                    if let Some(env_name) = inner.strip_prefix("ENV:") {
                        if env_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                            if let Ok(val) = std::env::var(env_name) {
                                return serde_json::Value::String(val);
                            }
                        }
                    }
                }
                // {params.xxx} 整串引用：返回原始类型（数字/布尔/嵌套对象不字符串化）
                if let Some(inner) = s.strip_prefix("{params.").and_then(|t| t.strip_suffix('}')) {
                    if !inner.is_empty()
                        && inner
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
                    {
                        if let Some(val) = vars
                            .get("params")
                            .and_then(|root| Self::resolve_path(root, inner))
                        {
                            return val;
                        }
                    }
                }
                // 部分替换：文本中含 {{var}}
                let mut result = s.clone();
                for (k, v) in vars {
                    let placeholder = format!("{{{{{}}}}}", k);
                    let replacement = match v {
                        serde_json::Value::String(sv) => sv.clone(),
                        other => other.to_string(),
                    };
                    result = result.replace(&placeholder, &replacement);
                }
                // 部分替换：文本中含 {params.xxx}（params.json 固化参数）
                let result = Self::replace_params_refs(&result, vars);
                serde_json::Value::String(result)
            }
            serde_json::Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    new_map.insert(k.clone(), Self::resolve_vars(v, vars));
                }
                serde_json::Value::Object(new_map)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(|v| Self::resolve_vars(v, vars)).collect())
            }
            other => other.clone(),
        }
    }

    /// params.json 路径取值：对嵌套 Value 按 `.` 分段逐层下钻
    pub(super) fn resolve_path(root: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
        let mut cur = root;
        for seg in path.split('.') {
            cur = cur.get(seg)?;
        }
        Some(cur.clone())
    }

    /// 文本内嵌 {params.xxx.yyy} 替换（未解析的保留原文，由编译期校验发现）
    pub(super) fn replace_params_refs(
        s: &str,
        vars: &HashMap<String, serde_json::Value>,
    ) -> String {
        if !s.contains("{params.") {
            return s.to_string();
        }
        let Some(root) = vars.get("params") else {
            return s.to_string();
        };
        let mut out = String::with_capacity(s.len());
        let mut rest = s;
        while let Some(start) = rest.find("{params.") {
            out.push_str(&rest[..start]);
            let after = &rest[start..];
            match after.find('}') {
                Some(end) => {
                    let path = &after[8..end];
                    let valid = !path.is_empty()
                        && path
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
                    let resolved = if valid {
                        Self::resolve_path(root, path)
                    } else {
                        None
                    };
                    match resolved {
                        Some(serde_json::Value::String(sv)) => out.push_str(&sv),
                        Some(other) => out.push_str(&other.to_string()),
                        None => out.push_str(&after[..=end]), // 未解析保留原文
                    }
                    rest = &after[end + 1..];
                }
                None => {
                    out.push_str(after);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// 管道变换：default / get / json key / len
    fn apply_pipe(val: Option<serde_json::Value>, pipe_expr: &str) -> serde_json::Value {
        let pipe_expr = pipe_expr.trim();

        // ── default：val 为空/Null 时用默认值（优先处理，不受 val 非空限制）──
        if let Some(rest) = pipe_expr.strip_prefix("default") {
            let default_raw = rest.trim().trim_matches('"');
            match val {
                None | Some(serde_json::Value::Null) => {
                    return serde_json::Value::String(default_raw.to_string());
                }
                Some(v) => return v,
            }
        }

        // ── get：对嵌套 JSON 做路径遍历（复用 resolve_path）──
        if let Some(rest) = pipe_expr.strip_prefix("get") {
            let path = rest.trim().trim_matches('"');
            return match val {
                Some(v) => Self::resolve_path(&v, path).unwrap_or(serde_json::Value::Null),
                None => serde_json::Value::Null,
            };
        }

        // ── 现有管道：len / json ──
        let val = match val {
            Some(v) => v,
            None => return serde_json::Value::Null,
        };

        if pipe_expr == "len" {
            return match &val {
                serde_json::Value::String(s) => serde_json::json!(s.len()),
                serde_json::Value::Array(a) => serde_json::json!(a.len()),
                _ => serde_json::json!(0),
            };
        }
        if let Some(rest) = pipe_expr.strip_prefix("json ") {
            let key = rest.trim().trim_matches('"');
            if let serde_json::Value::String(s) = &val {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
                    return parsed.get(key).cloned().unwrap_or(serde_json::Value::Null);
                }
            }
            return val.get(key).cloned().unwrap_or(serde_json::Value::Null);
        }
        val
    }
}

/// 字符串变量替换：{{var}} → value，支持管道 {{var | get "field"}}、{{var | default "x"}}、{{ENV:VAR}}。
/// 未解析的 {{...}} 清理为空。
pub(super) fn resolve_vars_str(s: &str, vars: &HashMap<String, serde_json::Value>) -> String {
    let mut result = s.to_string();

    // 1. ENV: 引用
    let re_env =
        regex::Regex::new(r"\{\{\s*ENV:([A-Za-z_][\w]*)\s*\}\}").expect("env ref regex valid");
    result = re_env
        .replace_all(&result, |caps: &regex::Captures| {
            std::env::var(&caps[1]).unwrap_or_default()
        })
        .to_string();

    // 2. 解析所有 {{...}} 模板（支持管道：{{var | get "field"}}、{{var | default "x"}}）
    let re_template = regex::Regex::new(r"\{\{(.+?)\}\}").expect("template regex valid");
    result = re_template
        .replace_all(&result, |caps: &regex::Captures| {
            let inner = caps[1].trim();
            if inner.is_empty() {
                return String::new();
            }
            // ENV: 前缀（已在步骤1处理，但防御）
            if inner.starts_with("ENV:") {
                return String::new();
            }
            // 管道表达式：{{var | op arg}}
            if inner.contains('|') {
                let mut parts = inner.splitn(2, '|');
                let var_name = parts.next().unwrap_or("").trim();
                let pipe_expr = parts.next().unwrap_or("").trim();
                let val = vars.get(var_name).cloned();
                let resolved = Executor::apply_pipe(val, pipe_expr);
                return match resolved {
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Null => String::new(),
                    other => other.to_string(),
                };
            }
            // 纯变量名
            if inner.chars().all(|c| c.is_alphanumeric() || c == '_') {
                if let Some(val) = vars.get(inner) {
                    return match val {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Null => String::new(),
                        other => other.to_string(),
                    };
                }
            }
            String::new()
        })
        .to_string();

    // 3. 清理仍未解析的 {{var}} 简单模板
    let re_simple = regex::Regex::new(r"\{\{[@\w_]+\}\}").expect("simple template regex valid");
    result = re_simple.replace_all(&result, "").to_string();

    // 4. {params.xxx} 内嵌引用
    Executor::replace_params_refs(&result, vars)
}

/// 按变量名或点号路径从变量表中取值。
/// - 无点号：`variables.get(name)`
/// - 有点号：取根变量后按路径下钻，如 `coords.need_scroll` → `variables["coords"]["need_scroll"]`
pub(super) fn resolve_var_by_path<'a>(
    var_path: &str,
    vars: &'a HashMap<String, serde_json::Value>,
) -> Option<&'a serde_json::Value> {
    if let Some(dot_pos) = var_path.find('.') {
        let root_name = &var_path[..dot_pos];
        let field_path = &var_path[dot_pos + 1..];
        let root = vars.get(root_name)?;
        // resolve_path 返回克隆值，但这里需要引用。我们用 .get() 链式查找。
        let mut cur = root;
        for seg in field_path.split('.') {
            cur = cur.get(seg)?;
        }
        Some(cur)
    } else {
        vars.get(var_path)
    }
}

/// Evaluate a Condition (V2 untagged enum) against variable bindings.
pub(super) fn eval_condition(
    condition: &Condition,
    variables: &HashMap<String, serde_json::Value>,
) -> bool {
    /// Resolve a VarRef to its string value from the variable pool
    fn resolve_ref(
        r: &crate::workflow::types::VarRef,
        vars: &HashMap<String, serde_json::Value>,
    ) -> String {
        match r {
            crate::workflow::types::VarRef::Var { var } => {
                if let Some(dot_pos) = var.find('.') {
                    let root = &var[..dot_pos];
                    let field = &var[dot_pos + 1..];
                    vars.get(root)
                        .and_then(|v| v.get(field))
                        .map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default()
                } else {
                    vars.get(var.as_str())
                        .map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Null => String::new(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default()
                }
            }
            crate::workflow::types::VarRef::Lit(s) => s.clone(),
        }
    }

    fn compare_numeric(
        refs: &[crate::workflow::types::VarRef],
        vars: &HashMap<String, serde_json::Value>,
        op: ConditionOp,
    ) -> bool {
        if refs.len() < 2 {
            return false;
        }
        let lhs = resolve_ref(&refs[0], vars).parse::<f64>().ok();
        let rhs = resolve_ref(&refs[1], vars).parse::<f64>().ok();
        match (lhs, rhs) {
            (Some(l), Some(r)) => match op {
                ConditionOp::Gt => l > r,
                ConditionOp::Lt => l < r,
                ConditionOp::Gte => l >= r,
                ConditionOp::Lte => l <= r,
            },
            _ => false,
        }
    }

    match condition {
        Condition::Always { always } => *always,
        Condition::NotEmpty { not_empty } => !resolve_ref(not_empty, variables).is_empty(),
        Condition::Empty { empty } => resolve_ref(empty, variables).is_empty(),
        Condition::Equals { equals } => {
            if equals.len() < 2 {
                return false;
            }
            resolve_ref(&equals[0], variables) == resolve_ref(&equals[1], variables)
        }
        Condition::NotEquals { not_equals } => {
            if not_equals.len() < 2 {
                return false;
            }
            resolve_ref(&not_equals[0], variables) != resolve_ref(&not_equals[1], variables)
        }
        Condition::Contains { contains } => {
            if contains.len() < 2 {
                return false;
            }
            resolve_ref(&contains[0], variables).contains(&resolve_ref(&contains[1], variables))
        }
        Condition::StartsWith { starts_with } => {
            if starts_with.len() < 2 {
                return false;
            }
            resolve_ref(&starts_with[0], variables)
                .starts_with(&resolve_ref(&starts_with[1], variables))
        }
        Condition::Regex { regex } => {
            if regex.len() < 2 {
                return false;
            }
            let pattern = resolve_ref(&regex[0], variables);
            let target = resolve_ref(&regex[1], variables);
            regex::Regex::new(&pattern).is_ok_and(|re| re.is_match(&target))
        }
        Condition::Gt { gt } => compare_numeric(gt, variables, ConditionOp::Gt),
        Condition::Lt { lt } => compare_numeric(lt, variables, ConditionOp::Lt),
        Condition::Gte { gte } => compare_numeric(gte, variables, ConditionOp::Gte),
        Condition::Lte { lte } => compare_numeric(lte, variables, ConditionOp::Lte),
    }
}
/// 将步骤输出写入变量池（共享实现：tool / script / mcp 步骤统一使用）。
/// 行为：capture 变量名先做 {{var}} 模板替换；输出优先 JSON 解析，失败回退为字符串。
pub(super) fn capture_output(
    capture: &Option<String>,
    output: &str,
    variables: &mut HashMap<String, serde_json::Value>,
) -> crate::Result<()> {
    if let Some(ref cap) = capture {
        let var_name = resolve_vars_str(cap, variables);
        let value = match serde_json::from_str::<serde_json::Value>(output) {
            Ok(v) => v,
            Err(_) => serde_json::Value::String(output.to_string()),
        };
        variables.insert(var_name, value);
    }
    Ok(())
}
