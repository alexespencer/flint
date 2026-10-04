use std::collections::BTreeSet;

use serde_json::Value;

pub(crate) struct Front {
    pub value: Value,
    pub tags: BTreeSet<String>,
    pub aliases: Vec<String>,
}

/// `Ok(None)` for an empty block, `Err` (with a message) for YAML that cannot be used.
pub(crate) fn parse(yaml: &str) -> Result<Option<Front>, String> {
    if yaml.trim().is_empty() {
        return Ok(None);
    }
    // The parser's message quotes a snippet of the document over many lines; the first line says what is wrong.
    // Obsidian reads YAML 1.2, where only `true` and `false` are booleans. The parser's default also
    // accepts the 1.1 forms, which would turn `reply: n` or `status: yes` into booleans.
    let mut options = serde_saphyr::Options::default();
    options.strict_booleans = true;
    let value: Value = serde_saphyr::from_str_with_options(yaml, options).map_err(|e| {
        let message = e.to_string();
        format!(
            "invalid YAML: {}",
            message.lines().next().unwrap_or("parse error")
        )
    })?;
    match &value {
        Value::Null => return Ok(None),
        Value::Object(_) => {}
        _ => return Err("frontmatter must be a mapping of keys to values".into()),
    }
    let tags = ["tags", "tag"]
        .iter()
        .filter_map(|key| value.get(key))
        .flat_map(tag_strings)
        .collect();
    let mut aliases: Vec<String> = Vec::new();
    for alias in ["aliases", "alias"]
        .iter()
        .filter_map(|key| value.get(key))
        .flat_map(alias_strings)
    {
        if !aliases.contains(&alias) {
            aliases.push(alias);
        }
    }
    Ok(Some(Front {
        value,
        tags,
        aliases,
    }))
}

/// `tags: [a, b]`, a YAML list, or `tags: a, b c` (Obsidian splits a string on commas and spaces).
fn tag_strings(value: &Value) -> Vec<String> {
    let words: Vec<String> = match value {
        Value::String(s) => s
            .split(|c: char| c == ',' || c.is_whitespace())
            .map(str::to_owned)
            .collect(),
        Value::Array(items) => items.iter().filter_map(scalar_string).collect(),
        other => scalar_string(other).into_iter().collect(),
    };
    words
        .into_iter()
        .map(|w| w.trim().trim_start_matches('#').to_owned())
        .filter(|w| !w.is_empty())
        .collect()
}

/// `aliases: Name` is one alias (names may contain commas), `aliases: [A, B]` is several.
fn alias_strings(value: &Value) -> Vec<String> {
    let items: Vec<String> = match value {
        Value::Array(items) => items.iter().filter_map(scalar_string).collect(),
        other => scalar_string(other).into_iter().collect(),
    };
    items
        .into_iter()
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty())
        .collect()
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}
