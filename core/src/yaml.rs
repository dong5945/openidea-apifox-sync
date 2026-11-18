//! Minimal JSON -> YAML for the OpenAPI export (`Export OpenAPI (YAML)`).
//! Only the subset serde_json produces: objects, arrays, strings, numbers,
//! booleans and null.
//!
//! Block style, two spaces per level. A mapping inside a sequence starts on
//! the `- ` line and its remaining keys line up with the first key: that is
//! what a YAML parser expects, and getting it wrong makes the whole document
//! unreadable (`expected <block end>, but found '<block mapping start>'`).

use serde_json::Value;

pub fn to_yaml(value: &Value) -> String {
    let mut out = String::new();
    match value {
        Value::Object(map) if !map.is_empty() => write_object_body(&mut out, map, 0),
        Value::Array(items) if !items.is_empty() => write_array_body(&mut out, items, 0),
        other => out.push_str(&inline(other)),
    }
    out
}

/// Keys of a mapping at `indent`, each on its own line (no leading newline).
fn write_object_body(out: &mut String, map: &serde_json::Map<String, Value>, indent: usize) {
    for (key, item) in map {
        push_indent(out, indent);
        out.push_str(&quote_key(key));
        out.push(':');
        write_child(out, item, indent);
    }
}

/// Items of a sequence at `indent`, each starting with `- ` (no leading newline).
fn write_array_body(out: &mut String, items: &[Value], indent: usize) {
    for item in items {
        push_indent(out, indent);
        out.push_str("- ");
        write_item(out, item, indent);
    }
}

/// A value that follows `key:` on the same line.
fn write_child(out: &mut String, item: &Value, indent: usize) {
    match item {
        Value::Object(map) if map.is_empty() => out.push_str(" {}\n"),
        Value::Array(items) if items.is_empty() => out.push_str(" []\n"),
        Value::Object(map) => {
            out.push('\n');
            write_object_body(out, map, indent + 1);
        }
        Value::Array(items) => {
            out.push('\n');
            write_array_body(out, items, indent + 1);
        }
        other => {
            out.push(' ');
            out.push_str(&inline(other));
        }
    }
}

/// An item that follows `- ` on the same line; `indent` is the `-` column.
fn write_item(out: &mut String, item: &Value, indent: usize) {
    match item {
        // The first key sits after `- `; the rest align with it, so they are
        // indented one level deeper than the dash.
        Value::Object(map) if !map.is_empty() => {
            for (i, (key, value)) in map.iter().enumerate() {
                if i > 0 {
                    push_indent(out, indent + 1);
                }
                out.push_str(&quote_key(key));
                out.push(':');
                write_child(out, value, indent + 1);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            out.push('\n');
            write_array_body(out, items, indent + 1);
        }
        other => out.push_str(&inline(other)),
    }
}

/// Scalars and empty collections, terminated by a newline.
fn inline(value: &Value) -> String {
    match value {
        Value::Object(_) => "{}\n".into(),
        Value::Array(_) => "[]\n".into(),
        Value::Null => "null\n".into(),
        Value::Bool(b) => format!("{b}\n"),
        Value::Number(n) => format!("{n}\n"),
        Value::String(s) => format!("{}\n", quote_string(s)),
    }
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

fn quote_key(key: &str) -> String {
    let plain = key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/')
        && !key.is_empty()
        // A bare `200` key would come back as the number 200.
        && key.parse::<f64>().is_err()
        && !matches!(
            key.to_ascii_lowercase().as_str(),
            "null" | "true" | "false" | "yes" | "no" | "on" | "off"
        );
    if plain {
        key.to_string()
    } else {
        format!("\"{}\"", escape(key))
    }
}

/// Plain when a parser reads it back as the same string; quoted otherwise
/// (empty, leading/trailing space, `#`, `:`, quotes, or a YAML indicator start).
fn quote_string(s: &str) -> String {
    const INDICATORS: &[char] = &['-', '?', ':', '!', '&', '*', '|', '>', '%', '@', '`', '[', ']', '{', '}', ',', '~'];
    let plain = !s.is_empty()
        && !s.starts_with(' ')
        && !s.ends_with(' ')
        && !s.contains(':')
        && !s.contains('#')
        && !s.contains('\n')
        && !s.contains('"')
        && !s.contains('\'')
        && !s.starts_with(INDICATORS)
        && !matches!(
            s.to_ascii_lowercase().as_str(),
            "null" | "true" | "false" | "yes" | "no" | "on" | "off" | "~"
        )
        && !s.parse::<f64>().is_ok();
    if plain {
        s.to_string()
    } else {
        format!("\"{}\"", escape(s))
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
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
        // serde_json's default map is sorted, so the keys come out in byte order.
        assert!(yaml.starts_with("info:\n  title: API\n  version: 1.0.0\n"), "{yaml}");
        assert!(yaml.contains("openapi: 3.0.3\n"), "{yaml}");
        assert!(yaml.contains("paths:\n  /a:\n    get:\n      summary: A\n"), "{yaml}");
        assert!(yaml.contains("tags: []\n"), "{yaml}");
    }

    #[test]
    fn sequence_of_objects_aligns_continuation_keys_with_the_dash() {
        let value = json!({"parameters":[
            {"in":"query","name":"name","required":true,"schema":{"type":"string"}},
            {"in":"header","name":"X-Trace"}
        ]});
        let yaml = to_yaml(&value);
        let expected = "parameters:\n  - in: query\n    name: name\n    required: true\n    schema:\n      type: string\n  - in: header\n    name: X-Trace\n";
        assert_eq!(yaml, expected);
    }

    #[test]
    fn quotes_strings_a_parser_would_read_as_something_else() {
        let value = json!({"a":"true","b":"123","c":"a: b","d":"","e":"#tag","f":"plain text","g":"line\nbreak"});
        let yaml = to_yaml(&value);
        assert!(yaml.contains("a: \"true\""), "{yaml}");
        assert!(yaml.contains("b: \"123\""), "{yaml}");
        assert!(yaml.contains("c: \"a: b\""), "{yaml}");
        assert!(yaml.contains("d: \"\""), "{yaml}");
        assert!(yaml.contains("e: \"#tag\""), "{yaml}");
        assert!(yaml.contains("f: plain text"), "{yaml}");
        assert!(yaml.contains("g: \"line\\nbreak\""), "{yaml}");
    }

    #[test]
    fn nested_sequences_of_objects() {
        let value = json!({"paths":{"/a":{"get":{"parameters":[
            {"name":"id","in":"path","schema":{"type":"integer","format":"int64"}}
        ]}}}});
        let yaml = to_yaml(&value);
        let expected = "paths:\n  /a:\n    get:\n      parameters:\n        - in: path\n          name: id\n          schema:\n            format: int64\n            type: integer\n";
        assert_eq!(yaml, expected);
    }
}
