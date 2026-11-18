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
    let (parsed, _) = parse_reachable(files);
    let index = spring::TypeIndex::build(&parsed);
    let refs: Vec<&java::JavaFile> = parsed.iter().collect();
    spring::scan_indexed(&index, &refs)
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
    let (parsed, origins) = parse_reachable(files);
    let index = spring::TypeIndex::build(&parsed);
    modules
        .iter()
        .map(|module| {
            let picked: Vec<&java::JavaFile> = origins
                .iter()
                .enumerate()
                .filter(|(_, idx)| {
                    in_module(&files[**idx].0, module) && in_selection(&files[**idx].0, selected)
                })
                .map(|(i, _)| &parsed[i])
                .collect();
            spring::scan_indexed(&index, &picked)
        })
        .collect()
}

/// Counts the UI needs to explain a module that produced nothing.
pub struct ScopedScan {
    pub docs: Vec<ApiDoc>,
    /// Java files offered to the scan.
    pub files: usize,
    /// Files that belong to at least one requested module.
    pub module_files: usize,
    /// A file/directory selection was ignored because nothing inside it can
    /// declare an endpoint, so the whole module was scanned instead.
    pub selection_ignored: bool,
}

/// Mapping-scoped scan: endpoints are classes with mapping annotations inside
/// the mapped modules (same rule as the reference plugin); DTO files elsewhere
/// are read only when a parsed type references them.
pub fn scan_modules_scoped<F>(
    paths: &[String],
    load: F,
    modules: &[String],
    selected: &[String],
) -> Vec<ApiDoc>
where
    F: FnMut(&str) -> Option<String>,
{
    scan_modules_scoped_report(paths, load, modules, selected).docs
}

/// Like [`scan_modules_scoped`], but keeps the numbers behind the result.
///
/// A project-view right-click carries whatever file or folder happens to be
/// selected. When that selection holds no controller (a DTO, a `pom.xml`, a
/// service package) the old code scanned it verbatim and reported "没有接口",
/// which looked like the module had lost its endpoints. The module is now
/// scanned whole in that case and the caller can say so.
pub fn scan_modules_scoped_report<F>(
    paths: &[String],
    mut load: F,
    modules: &[String],
    selected: &[String],
) -> ScopedScan
where
    F: FnMut(&str) -> Option<String>,
{
    let mut stems: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, path) in paths.iter().enumerate() {
        stems.entry(file_stem(path)).or_default().push(i);
    }
    let module_candidates: Vec<usize> = (0..paths.len())
        .filter(|&i| modules.iter().any(|m| in_module(&paths[i], m)))
        .collect();
    let mut candidates: Vec<usize> = module_candidates
        .iter()
        .copied()
        .filter(|&i| in_selection(&paths[i], selected))
        .collect();
    let mut texts: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    for &idx in &candidates {
        if let Some(text) = load(&paths[idx]) {
            texts.insert(idx, text);
        }
    }
    let mut selection_ignored = false;
    if !selected.is_empty() {
        let declares = candidates
            .iter()
            .any(|i| texts.get(i).is_some_and(|text| is_endpoint_source(text)));
        if !declares {
            candidates = module_candidates.clone();
            for &idx in &candidates {
                if !texts.contains_key(&idx) {
                    if let Some(text) = load(&paths[idx]) {
                        texts.insert(idx, text);
                    }
                }
            }
            selection_ignored = true;
        }
    }
    let queue: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|i| texts.get(i).is_some_and(|text| is_endpoint_source(text)))
        .collect();
    let parsed = expand(queue, &mut texts, paths, &stems, &mut load);
    let effective: &[String] = if selection_ignored { &[] } else { selected };
    ScopedScan {
        docs: docs_from(&parsed, paths, modules, effective),
        files: paths.len(),
        module_files: module_candidates.len(),
        selection_ignored,
    }
}

/// The directory a mapping row actually scans.
///
/// Rows store the module path relative to the project root detected when the
/// row was created. Opening the module folder itself (or renaming the folder)
/// leaves no file under that path, and the scan then finds nothing; when the
/// root's own name is the module, scanning the root is the same thing.
pub fn module_scope(paths: &[String], module: &str, root_name: &str) -> String {
    if module.is_empty() {
        return String::new();
    }
    if paths.iter().any(|p| in_module(p, module)) {
        return module.to_string();
    }
    let last = module.rsplit('/').find(|s| !s.is_empty()).unwrap_or("");
    if !last.is_empty() && last.eq_ignore_ascii_case(root_name.trim()) {
        String::new()
    } else {
        module.to_string()
    }
}

fn expand<F>(
    mut queue: Vec<usize>,
    texts: &mut std::collections::HashMap<usize, String>,
    paths: &[String],
    stems: &std::collections::HashMap<String, Vec<usize>>,
    load: &mut F,
) -> std::collections::HashMap<usize, java::JavaFile>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut parsed: std::collections::HashMap<usize, java::JavaFile> =
        std::collections::HashMap::new();
    while let Some(idx) = queue.pop() {
        if parsed.contains_key(&idx) {
            continue;
        }
        let text = match texts.remove(&idx) {
            Some(text) => text,
            None => match load(&paths[idx]) {
                Some(text) => text,
                None => continue,
            },
        };
        let file = java::parse_file(&text);
        for name in file_refs(&file) {
            let base = name.rsplit('.').next().unwrap_or(&name);
            if let Some(candidates) = stems.get(base) {
                for &candidate in candidates {
                    if candidate != idx && !parsed.contains_key(&candidate) {
                        queue.push(candidate);
                    }
                }
            }
        }
        parsed.insert(idx, file);
    }
    parsed
}
fn docs_from(
    parsed: &std::collections::HashMap<usize, java::JavaFile>,
    paths: &[String],
    modules: &[String],
    selected: &[String],
) -> Vec<ApiDoc> {
    let mut origins: Vec<usize> = parsed.keys().copied().collect();
    origins.sort_unstable();
    let files: Vec<&java::JavaFile> = origins.iter().map(|i| &parsed[i]).collect();
    let index = spring::TypeIndex::build_refs(&files);
    modules
        .iter()
        .map(|module| {
            let picked: Vec<&java::JavaFile> = origins
                .iter()
                .enumerate()
                .filter(|(_, idx)| {
                    in_module(&paths[**idx], module) && in_selection(&paths[**idx], selected)
                })
                .map(|(i, _)| files[i])
                .collect();
            spring::scan_indexed(&index, &picked)
        })
        .collect()
}

fn file_stem(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".java")
        .to_string()
}
/// Endpoint classes plus every type they (transitively) reference. A monorepo
/// with thousands of classes otherwise blows the host's per-call CPU budget.
fn parse_reachable(files: &[(String, String)]) -> (Vec<java::JavaFile>, Vec<usize>) {
    let mut registry: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, (_, text)) in files.iter().enumerate() {
        for name in declared_types(text) {
            registry.entry(name).or_default().push(i);
        }
    }
    let mut queue: Vec<usize> = (0..files.len())
        .filter(|&i| is_endpoint_source(&files[i].1))
        .collect();
    queue.sort_unstable();
    queue.dedup();
    let mut parsed: std::collections::HashMap<usize, java::JavaFile> =
        std::collections::HashMap::new();
    while let Some(idx) = queue.pop() {
        if parsed.contains_key(&idx) {
            continue;
        }
        let file = java::parse_file(&files[idx].1);
        for name in file_refs(&file) {
            if let Some(candidates) = registry.get(&name) {
                for &candidate in candidates {
                    if candidate != idx && !parsed.contains_key(&candidate) {
                        queue.push(candidate);
                    }
                }
            }
        }
        parsed.insert(idx, file);
    }
    let mut origins: Vec<usize> = parsed.keys().copied().collect();
    origins.sort_unstable();
    let mut ordered = Vec::with_capacity(origins.len());
    for idx in &origins {
        if let Some(file) = parsed.remove(idx) {
            ordered.push(file);
        }
    }
    (ordered, origins)
}

fn is_endpoint_source(text: &str) -> bool {
    const MARKS: &[&str] = &[
        "@RestController",
        "@Controller",
        "@RequestMapping",
        "@GetMapping",
        "@PostMapping",
        "@PutMapping",
        "@DeleteMapping",
        "@PatchMapping",
        "@FeignClient",
        "@Path",
    ];
    MARKS.iter().any(|m| text.contains(m))
}

/// Class/interface/enum/record names declared in the source. Cheap string
/// pass; used only to find the file that owns a referenced type.
fn declared_types(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for keyword in ["class", "interface", "enum", "record"] {
        for (pos, _) in text.match_indices(keyword) {
            let bytes = text.as_bytes();
            if pos > 0 && is_ident_byte(bytes[pos - 1]) {
                continue;
            }
            let mut i = pos + keyword.len();
            if i < bytes.len() && !is_ident_byte(bytes[i]) {
                while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                    i += 1;
                }
                let start = i;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                if i > start {
                    out.push(text[start..i].to_string());
                }
            }
        }
    }
    out
}

fn file_refs(file: &java::JavaFile) -> Vec<String> {
    let mut out = Vec::new();
    for cls in &file.classes {
        out.extend(cls.bases.iter().cloned());
        for field in &cls.fields {
            out.extend(type_names(&field.ty));
        }
        for method in &cls.methods {
            out.extend(type_names(&method.ret));
            for param in &method.params {
                out.extend(type_names(&param.ty));
            }
        }
    }
    out
}

fn type_names(ty: &str) -> Vec<String> {
    ty.split(|c: char| !is_ident_byte(c as u8))
        .filter(|w| w.len() > 1 && w.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
        .map(|w| w.to_string())
        .collect()
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
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
        for response in &ep.responses {
            if let Some(ty) = &response.ty {
                collect_refs(ty, &mut wanted);
            }
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