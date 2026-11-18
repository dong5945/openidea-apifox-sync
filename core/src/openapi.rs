//! OpenAPI 3.0.3 rendering. Apifox's `import-openapi` accepts this document
//! verbatim, so the export format is the contract between the scanner and the
//! network layer.

use serde_json::{json, Map, Value};

use crate::model::{ApiDoc, Field, Param, Ty};

pub fn render(doc: &ApiDoc) -> String {
    let mut paths: Map<String, Value> = Map::new();
    let mut tags: Vec<Value> = Vec::new();
    let mut seen_tags: Vec<&str> = Vec::new();

    for ep in &doc.endpoints {
        if !seen_tags.contains(&ep.tag.as_str()) {
            seen_tags.push(&ep.tag);
            tags.push(json!({ "name": ep.tag }));
        }
        let mut op = Map::new();
        op.insert("tags".into(), json!([ep.tag]));
        op.insert("summary".into(), json!(ep.summary));
        if !ep.description.is_empty() {
            op.insert("description".into(), json!(ep.description));
        }
        op.insert("operationId".into(), json!(ep.operation_id));
        if ep.deprecated {
            op.insert("deprecated".into(), json!(true));
        }
        if !ep.params.is_empty() {
            op.insert(
                "parameters".into(),
                Value::Array(ep.params.iter().map(param_value).collect()),
            );
        }
        if let Some((mime, ty)) = &ep.body {
            op.insert(
                "requestBody".into(),
                json!({
                    "required": true,
                    "content": { mime.clone(): { "schema": schema_with_example(ty) } }
                }),
            );
        }
        let response = match &ep.response {
            Some(ty) => json!({
                "description": ep.response_description,
                "content": { "application/json": { "schema": schema_with_example(ty) } }
            }),
            None => json!({ "description": ep.response_description }),
        };
        op.insert("responses".into(), json!({ "200": response }));

        let entry = paths.entry(ep.path.clone()).or_insert_with(|| json!({}));
        if let Value::Object(map) = entry {
            map.insert(ep.method.to_lowercase(), Value::Object(op));
        }
    }

    let mut schemas = Map::new();
    for (name, ty) in &doc.schemas {
        schemas.insert(name.clone(), schema_value(ty));
    }

    let mut root = Map::new();
    root.insert("openapi".into(), json!("3.0.3"));
    root.insert(
        "info".into(),
        json!({ "title": doc.title, "version": doc.version }),
    );
    if !tags.is_empty() {
        root.insert("tags".into(), Value::Array(tags));
    }
    root.insert("paths".into(), Value::Object(paths));
    if !schemas.is_empty() {
        root.insert(
            "components".into(),
            json!({ "schemas": Value::Object(schemas) }),
        );
    }
    serde_json::to_string_pretty(&Value::Object(root)).expect("JSON value is serialisable")
}

fn param_value(p: &Param) -> Value {
    let mut v = json!({
        "name": p.name,
        "in": p.location.as_str(),
        "required": p.required,
        "schema": schema_value(&p.ty),
    });
    if !p.description.is_empty() {
        v["description"] = json!(p.description);
    }
    if let Some(d) = &p.default {
        v["schema"]["default"] = default_value(d, &p.ty);
    }
    v
}

fn default_value(raw: &str, ty: &Ty) -> Value {
    match ty {
        Ty::Integer | Ty::Long => raw.parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(raw)),
        Ty::Number => raw.parse::<f64>().map(Value::from).unwrap_or_else(|_| json!(raw)),
        Ty::Boolean => json!(raw == "true"),
        _ => json!(raw),
    }
}

/// A sample value so Apifox shows something instead of `null`.
pub fn example_value(ty: &Ty) -> Option<Value> {
    Some(match ty {
        Ty::String => json!("示例"),
        Ty::Integer | Ty::Long => json!(1),
        Ty::Number => json!(1.5),
        Ty::Boolean => json!(true),
        Ty::Binary => json!("file"),
        Ty::Enum(values) => json!(values.first().cloned().unwrap_or_default()),
        Ty::Array(inner) => json!([example_value(inner).unwrap_or(Value::Null)]),
        Ty::Object(fields) => {
            let mut obj = Map::new();
            for f in fields {
                obj.insert(f.name.clone(), example_value(&f.ty).unwrap_or(Value::Null));
            }
            Value::Object(obj)
        }
        Ty::Ref(_) | Ty::Any => return None,
    })
}

/// `schema_value` plus `example` (never on a `$ref`, whose siblings 3.0 ignores).
pub fn schema_with_example(ty: &Ty) -> Value {
    let mut value = schema_value(ty);
    if let Some(obj) = value.as_object_mut() {
        if obj.contains_key("$ref") {
            return value;
        }
        if let Some(example) = example_value(ty) {
            obj.insert("example".into(), example);
        }
    }
    value
}
pub fn schema_value(ty: &Ty) -> Value {
    match ty {
        Ty::String => json!({ "type": "string" }),
        Ty::Integer => json!({ "type": "integer", "format": "int32" }),
        Ty::Long => json!({ "type": "integer", "format": "int64" }),
        Ty::Number => json!({ "type": "number", "format": "double" }),
        Ty::Boolean => json!({ "type": "boolean" }),
        Ty::Binary => json!({ "type": "string", "format": "binary" }),
        Ty::Array(inner) => json!({ "type": "array", "items": schema_value(inner) }),
        Ty::Ref(name) => json!({ "$ref": format!("#/components/schemas/{name}") }),
        Ty::Enum(values) => json!({ "type": "string", "enum": values }),
        Ty::Object(fields) => object_value(fields),
        Ty::Any => json!({}),
    }
}

fn object_value(fields: &[Field]) -> Value {
    let mut props = Map::new();
    let mut required: Vec<String> = Vec::new();
    for f in fields {
        let mut schema = schema_value(&f.ty);
        let is_array = matches!(f.ty, Ty::Array(_));
        if let Some(obj) = schema.as_object_mut() {
            // `$ref` siblings are ignored by OpenAPI 3.0; keep the ref clean.
            if !obj.contains_key("$ref") {
                if !f.description.is_empty() {
                    obj.insert("description".into(), json!(f.description));
                }
                if let Some(v) = f.min {
                    obj.insert("minimum".into(), json!(v));
                }
                if let Some(v) = f.max {
                    obj.insert("maximum".into(), json!(v));
                }
                if let Some(v) = f.min_len {
                    let key = if is_array { "minItems" } else { "minLength" };
                    obj.insert(key.into(), json!(v));
                }
                if let Some(v) = f.max_len {
                    let key = if is_array { "maxItems" } else { "maxLength" };
                    obj.insert(key.into(), json!(v));
                }
                if let Some(p) = &f.pattern {
                    obj.insert("pattern".into(), json!(p));
                }
                if let Some(fmt) = &f.format {
                    obj.insert("format".into(), json!(fmt));
                }
                if let Some(example) = example_value(&f.ty) {
                    obj.insert("example".into(), example);
                }
            }
        }
        props.insert(f.name.clone(), schema);
        if f.required {
            required.push(f.name.clone());
        }
    }
    let mut out = Map::new();
    out.insert("type".into(), json!("object"));
    out.insert("properties".into(), Value::Object(props));
    if !required.is_empty() {
        out.insert("required".into(), json!(required));
    }
    Value::Object(out)
}