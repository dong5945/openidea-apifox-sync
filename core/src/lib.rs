//! Pure core of the Apifox export plugin.
//!
//! Layering (no host calls anywhere in this crate):
//! `java` (text -> classes) -> `spring` (annotations -> endpoints) ->
//! `openapi` (endpoints -> OpenAPI 3.0.3) -> `apifox` (document -> HTTP request).
//!
//! The OpenIDEA shell (`src/lib.rs` of the plugin crate) only does three
//! things: read project files, call `host::fetch`, show notifications.

pub mod apifox;
pub mod java;
pub mod model;
pub mod naming;
pub mod project;
pub mod openapi;
pub mod postman;
pub mod yaml;
pub mod spring;

use model::ApiDoc;

/// Scans Java source texts and builds the document.
pub fn scan_sources(sources: &[String]) -> ApiDoc {
    let files: Vec<java::JavaFile> = sources.iter().map(|s| java::parse_file(s)).collect();
    spring::scan(&files)
}

/// `files`: project-relative path and source text of every `.java`.
pub fn scan_project_files(files: &[(String, String)]) -> ApiDoc {
    let parsed: Vec<java::JavaFile> = files.iter().map(|(_, text)| java::parse_file(text)).collect();
    spring::scan(&parsed)
}

/// One document per module path (`""` = whole project). Types are resolved
/// against **all** files, so a controller can use DTOs from another module —
/// which is the normal Maven/Gradle layout and the reason responses used to
/// come out as `{}`.
pub fn scan_modules(files: &[(String, String)], modules: &[String]) -> Vec<ApiDoc> {
    scan_modules_selected(files, modules, &[])
}

/// Like [`scan_modules`], but endpoints only come from `selected` files or
/// directories (project-relative). Types still resolve against everything.
pub fn scan_modules_selected(
    files: &[(String, String)],
    modules: &[String],
    selected: &[String],
) -> Vec<ApiDoc> {
    let parsed: Vec<java::JavaFile> = files.iter().map(|(_, text)| java::parse_file(text)).collect();
    let index = spring::TypeIndex::build(&parsed);
    modules
        .iter()
        .map(|module| {
            let picked: Vec<&java::JavaFile> = files
                .iter()
                .zip(parsed.iter())
                .filter(|((path, _), _)| in_module(path, module) && in_selection(path, selected))
                .map(|(_, file)| file)
                .collect();
            spring::scan_indexed(&index, &picked)
        })
        .collect()
}

fn in_selection(path: &str, selected: &[String]) -> bool {
    if selected.is_empty() {
        return true;
    }
    selected.iter().any(|s| {
        let s = s.trim_end_matches('/');
        if s.is_empty() {
            return false;
        }
        if s.to_ascii_lowercase().ends_with(".java") {
            path.eq_ignore_ascii_case(s)
        } else {
            path == s || path.starts_with(&format!("{s}/"))
        }
    })
}

/// True when `path` belongs to the module directory `module`.
pub fn in_module(path: &str, module: &str) -> bool {
    if module.is_empty() {
        return true;
    }
    let module = module.trim_end_matches('/');
    path == module || path.starts_with(&format!("{module}/"))
}

/// Scans Java source texts and returns an OpenAPI 3.0.3 JSON document.
pub fn generate(sources: &[String]) -> String {
    openapi::render(&scan_sources(sources))
}

/// Prepends `server.servlet.context-path` to every endpoint path (once).
pub fn prefix_paths(doc: &mut ApiDoc, prefix: &str) {
    let prefix = normalize_prefix(prefix);
    if prefix.is_empty() {
        return;
    }
    for ep in &mut doc.endpoints {
        if !ep.path.starts_with(&prefix) {
            ep.path = format!("{prefix}{}", ep.path);
        }
    }
}

fn normalize_prefix(prefix: &str) -> String {
    let p = prefix.trim().trim_matches('"').trim_matches('\'');
    let p = p.trim_end_matches('/');
    if p.is_empty() || p == "/" {
        String::new()
    } else if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{p}")
    }
}
/// Keeps only the chosen endpoints (by index) and the schemas they reach,
/// so a partial export does not ship unrelated models.
pub fn select_endpoints(doc: &ApiDoc, keep: &[usize]) -> ApiDoc {
    let mut out = doc.clone();
    out.endpoints = keep
        .iter()
        .filter_map(|i| doc.endpoints.get(*i).cloned())
        .collect();

    let mut wanted: Vec<String> = Vec::new();
    for ep in &out.endpoints {
        for p in &ep.params {
            collect_refs(&p.ty, &mut wanted);
        }
        if let Some((_, ty)) = &ep.body {
            collect_refs(ty, &mut wanted);
        }
        if let Some(ty) = &ep.response {
            collect_refs(ty, &mut wanted);
        }
    }
    let mut schemas = std::collections::BTreeMap::new();
    while let Some(name) = wanted.pop() {
        if schemas.contains_key(&name) {
            continue;
        }
        let Some(ty) = doc.schemas.get(&name) else { continue };
        collect_refs(ty, &mut wanted);
        schemas.insert(name, ty.clone());
    }
    out.schemas = schemas;
    out
}

fn collect_refs(ty: &model::Ty, out: &mut Vec<String>) {
    match ty {
        model::Ty::Ref(name) => out.push(name.clone()),
        model::Ty::Array(inner) => collect_refs(inner, out),
        model::Ty::Object(fields) => {
            for f in fields {
                collect_refs(&f.ty, out);
            }
        }
        _ => {}
    }
}