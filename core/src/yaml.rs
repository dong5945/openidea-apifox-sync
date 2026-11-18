//! Minimal JSON -> YAML for the OpenAPI export (`Export OpenAPI (YAML)`).
//! Only the subset serde_json produces: objects, arrays, strings, numbers,
//! booleans and null.

use serde_json::Value;

pub fn to_yaml(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0, false);
    out
}

fn write_value(out: &mut String, value: &Value, indent: usize, inline: bool) {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}\n");
                return;
            }
            if !inline {
                out.push('\n');
            }
            for (key, item) in map {
                push_indent(out, indent);
                out.push_str(&quote_key(key));
                out.push(':');
                write_child(out, item, indent);
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]\n");
                return;
            }
            if !inline {
                out.push('\n');
            }
            for item in items {
                push_indent(out, indent);
                out.push_str("- ");
                write_value(out, item, indent + 1, true);
            }
        }
        _ => {
            out.push(' ');
            out.push_str(&scalar(value));
            out.push('\n');
        }
    }
}

fn write_child(out: &mut String, item: &Value, indent: usize) {
    match item {
        Value::Array(items) if items.is_empty() => out.push_str(" []\n"),
        Value::Object(map) if map.is_empty() => out.push_str(" {}\n"),
        Value::Object(_) | Value::Array(_) => write_value(out, item, indent + 1, false),
        _ => {
            out.push(' ');
            out.push_str(&scalar(item));
            out.push('\n');
        }
    }
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

fn quote_key(key: &str) -> String {
    if key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/')
        && !key.is_empty()
    {
        key.to_string()
    } else {
        format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            if s.is_empty() || s.contains(':') || s.contains('#') || s.contains('\n') || s.starts_with(' ') {
                format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
            } else {
                s.clone()
            }
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_nested_documents() {
        let value = json!({"openapi":"3.0.3","info":{"title":"API","version":"1.0.0"},
            "paths":{"/a":{"get":{"summary":"A"}}},"tags":[]});
        let yaml = to_yaml(&value);
        assert!(yaml.contains("openapi: 3.0.3"));
        assert!(yaml.contains("info:"));
        assert!(yaml.contains("  title: API"));
        assert!(yaml.contains("/a:"));
        assert!(yaml.contains("tags: []"));
    }
}