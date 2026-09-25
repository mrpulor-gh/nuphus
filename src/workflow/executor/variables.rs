//! 变量解析：模板替换与管道变换
use super::*;

fn replace_json_templates(text: &str, vars: &HashMap<String, serde_json::Value>) -> String {
    // Do not reinterpret legacy templates or evaluate text produced by a replacement.
    let mut result = String::new();
    let mut offset = 0;
    for (start, end, body) in crate::workflow::references::template_spans(text) {
        result.push_str(&Executor::replace_params_refs(&text[offset..start], vars));
        let body = body.trim();
        let value = crate::workflow::references::resolve(body, vars)
            .cloned()
            .or_else(|| vars.get(body).cloned())
            .or_else(|| Executor::lookup_inputs_ref(body, vars));
        if let Some(value) = value {
            result.push_str(
                &value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string()),
            );
        } else {
            result.push_str(&text[start..end]);
        }
        offset = end;
    }
    result.push_str(&Executor::replace_params_refs(&text[offset..], vars));
    result
}

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
                    if let Some(value) = crate::workflow::references::resolve(inner, vars) {
                        return value.clone();
                    }
                    // 管道表达式：{{var | op arg}}
                    if inner.contains('|') {
                        let mut parts = inner.splitn(2, '|');
                        let var_name = parts.next().unwrap_or("").trim();
                        let pipe_expr = parts.next().unwrap_or("").trim();
                        // ENV: 前缀 → 从环境变量取值（支持管道如 {{ENV:HOME | default "~/nuphus"}}）
                        let val = if let Some(env_name) = var_name.strip_prefix("ENV:") {
                            std::env::var(env_name).ok().map(serde_json::Value::String)
                        } else {
                            Self::lookup_value(var_name, vars)
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
                    // 命名空间引用：{{inputs.x}} → variables["inputs"]["x"]（返回原始类型）
                    if let Some(val) = Self::lookup_inputs_ref(inner, vars) {
                        return val;
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
                serde_json::Value::String(replace_json_templates(s, vars))
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

    /// 命名空间取值：`inputs.x` / `inputs.x.y` → variables["inputs"] 逐层下钻。
    /// 非 `inputs.` 前缀或路径非法 → None（顶层名字由既有分支处理，既有语义不变）。
    fn lookup_inputs_ref(
        name: &str,
        vars: &HashMap<String, serde_json::Value>,
    ) -> Option<serde_json::Value> {
        let path = name.strip_prefix("inputs.")?;
        Self::lookup_inputs_path(path, vars)
    }

    /// 已去 `inputs.` 前缀的路径取值（`x` / `x.y`）。
    ///
    /// 文本内嵌替换（`replace_inputs_refs`）拿到的就是去前缀后的后缀，
    /// 必须走同一入口——否则会二次剥离前缀导致恒为 None、静默不替换。
    fn lookup_inputs_path(
        path: &str,
        vars: &HashMap<String, serde_json::Value>,
    ) -> Option<serde_json::Value> {
        let valid = !path.is_empty()
            && path
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
        if !valid {
            return None;
        }
        vars.get("inputs")
            .and_then(|root| Self::resolve_path(root, path))
    }

    /// 变量取值：`inputs.x` 命名空间优先，其次顶层变量名（兼容既有 {{var}} 管道取值）
    fn lookup_value(
        name: &str,
        vars: &HashMap<String, serde_json::Value>,
    ) -> Option<serde_json::Value> {
        crate::workflow::references::resolve(name, vars)
            .cloned()
            .or_else(|| Self::lookup_inputs_ref(name, vars))
            .or_else(|| vars.get(name).cloned())
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
    let mut result = String::new();
    let mut offset = 0;
    for (start, end, body) in crate::workflow::references::template_spans(s) {
        result.push_str(&Executor::replace_params_refs(&s[offset..start], vars));
        let inner = body.trim();
        let resolved = if let Some(value) = crate::workflow::references::resolve(inner, vars) {
            Some(value.clone())
        } else if let Some((name, pipe)) = inner.split_once('|') {
            let name = name.trim();
            let value = if let Some(env) = name.strip_prefix("ENV:") {
                std::env::var(env).ok().map(serde_json::Value::String)
            } else {
                Executor::lookup_value(name, vars)
            };
            Some(Executor::apply_pipe(value, pipe.trim()))
        } else if let Some(env) = inner.strip_prefix("ENV:") {
            std::env::var(env).ok().map(serde_json::Value::String)
        } else if inner.chars().all(|c| c.is_alphanumeric() || c == '_') {
            vars.get(inner).cloned()
        } else {
            Executor::lookup_inputs_ref(inner, vars)
        };
        if let Some(value) = resolved {
            match value {
                serde_json::Value::String(value) => result.push_str(&value),
                serde_json::Value::Null if !inner.contains('[') => {}
                other => result.push_str(&other.to_string()),
            }
        }
        offset = end;
    }
    result.push_str(&Executor::replace_params_refs(&s[offset..], vars));
    result
}

/// 按变量名或点号路径从变量表中取值。
/// - 无点号：`variables.get(name)`
/// - 有点号：取根变量后按路径下钻，如 `coords.need_scroll` → `variables["coords"]["need_scroll"]`
pub(super) fn resolve_var_by_path<'a>(
    var_path: &str,
    vars: &'a HashMap<String, serde_json::Value>,
) -> Option<&'a serde_json::Value> {
    if var_path.contains('[') {
        return crate::workflow::references::resolve(var_path, vars);
    }
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
                if var.contains('[') {
                    return crate::workflow::references::resolve(var, vars)
                        .map(|value| {
                            value
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| value.to_string())
                        })
                        .unwrap_or_default();
                }
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
