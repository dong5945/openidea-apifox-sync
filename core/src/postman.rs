//! Postman Collection v2.1 export (`Export Postman collection`).

use serde_json::{json, Map, Value};

use crate::model::{ApiDoc, Endpoint, Ty};

pub fn collection(doc: &ApiDoc, title: &str) -> String {
    let mut folders: Vec<(String, Vec<Value>)> = Vec::new();
    for ep in &doc.endpoints {
        let item = item(ep, doc);
        match folders.iter_mut().find(|(tag, _)| tag == &ep.tag) {
            Some((_, items)) => items.push(item),
            None => folders.push((ep.tag.clone(), vec![item])),
        }
    }
    let items: Vec<Value> = folders
        .into_iter()
        .map(|(name, items)| json!({ "name": name, "item": items }))
        .collect();

    serde_json::to_string_pretty(&json!({
        "info": {
            "name": title,
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "item": items
    }))
    .expect("collection is serialisable")
}

fn item(ep: &Endpoint, doc: &ApiDoc) -> Value {
    let mut path: Vec<Value> = vec![json!("{{baseUrl}}")];
    for segment in ep.path.trim_matches('/').split('/').filter(|s| !s.is_empty()) {
        path.push(json!(segment));
    }
    let mut request = Map::new();
    request.insert("method".into(), json!(ep.method));
    request.insert("header".into(), json!([]));
    request.insert(
        "url".into(),
        json!({
            "raw": format!("{{{{baseUrl}}}}{}", ep.path),
            "host": ["{{baseUrl}}"],
            "path": path
        }),
    );
    if let Some((mime, ty)) = &ep.body {
        let raw = if mime == "application/json" {
            body_example(doc, ty)
        } else {
            String::new()
        };
        request.insert(
            "body".into(),
            json!({
                "mode": "raw",
                "raw": raw,
                "options": { "raw": { "language": if mime == "application/json" { "json" } else { "text" } } }
            }),
        );
    }
    json!({
        "name": ep.summary,
        "request": Value::Object(request),
        "response": []
    })
}

fn body_example(doc: &ApiDoc, ty: &Ty) -> String {
    let value = match ty {
        Ty::Ref(name) => doc.schemas.get(name).and_then(crate::openapi::example_value),
        other => crate::openapi::example_value(other),
    };
    value.unwrap_or_else(|| json!({})).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_items_by_tag() {
        let doc = crate::scan_sources(&[r#"
@RestController
@RequestMapping("/u")
public class U {
    @GetMapping("/a") public String a() { return null; }
    @PostMapping("/b") public String b(@RequestBody Body body) { return null; }
}
class Body { private String name; }
"#
        .to_string()]);
        let text = collection(&doc, "T");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["info"]["name"], "T");
        assert_eq!(v["item"][0]["item"].as_array().unwrap().len(), 2);
        assert_eq!(v["item"][0]["item"][1]["request"]["method"], "POST");
        assert!(v["item"][0]["item"][1]["request"]["body"]["raw"]
            .as_str()
            .unwrap()
            .contains("name"));
    }
}