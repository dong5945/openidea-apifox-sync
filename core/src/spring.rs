//! Spring MVC mapping: `JavaClass` -> `Endpoint` + JSON Schema.
//!
//! Only the annotations that change the HTTP contract are interpreted; the
//! scanner in `java` already turned the source text into classes/methods.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::java::{ann_attr, ann_first_string, ann_string, split_top_level, Ann, JavaClass, JavaField, JavaFile};
use crate::model::{ApiDoc, Endpoint, Field, Param, ParamIn, Ty};

const WRAPPERS: &[&str] = &[
    "Result",
    "R",
    "ApiResponse",
    "AjaxResult",
    "CommonResult",
    "ResponseResult",
    "ResponseEntity",
    "HttpEntity",
    "Response",
    "Res",
];

const PAGE_TYPES: &[&str] = &["Page", "IPage", "PageResult", "PageInfo", "PagedResult"];

const PRIMITIVE_REQUIRED: &[&str] = &["int", "long", "short", "byte", "double", "float", "boolean", "char"];

const SKIPPED_PARAMS: &[&str] = &[
    "HttpServletRequest",
    "HttpServletResponse",
    "ServletRequest",
    "ServletResponse",
    "HttpSession",
    "Model",
    "ModelMap",
    "BindingResult",
    "Errors",
    "Principal",
    "Locale",
    "InputStream",
    "OutputStream",
    "Reader",
    "Writer",
];

/// Every class of the project, by simple name: cross-module DTOs resolve.
pub struct TypeIndex<'a> {
    classes: HashMap<String, &'a JavaClass>,
    known: HashSet<String>,
}

impl<'a> TypeIndex<'a> {
    pub fn build(files: &'a [JavaFile]) -> Self {
        let mut classes: HashMap<String, &JavaClass> = HashMap::new();
        for file in files {
            for cls in &file.classes {
                classes.entry(cls.name.clone()).or_insert(cls);
            }
        }
        let known = classes.keys().cloned().collect();
        Self { classes, known }
    }
}

pub fn scan(files: &[JavaFile]) -> ApiDoc {
    let index = TypeIndex::build(files);
    let refs: Vec<&JavaFile> = files.iter().collect();
    scan_indexed(&index, &refs)
}

/// Endpoints come from `endpoint_files`; types may come from the whole project.
pub fn scan_indexed(index: &TypeIndex, endpoint_files: &[&JavaFile]) -> ApiDoc {
    let mut endpoints = Vec::new();
    for file in endpoint_files {
        for cls in &file.classes {
            if cls.doc.tags.contains_key("ignore") || !is_controller(cls) {
                continue;
            }
            let base = class_base_path(cls);
            for m in &cls.methods {
                if m.doc.tags.contains_key("ignore") {
                    continue;
                }
                let mappings = method_mappings(&m.anns);
                if mappings.is_empty() {
                    continue;
                }
                for (method, sub) in mappings {
                    let path = join_path(&base, &sub);
                    endpoints.extend(build_endpoint(cls, m, &method, &path, &index.known));
                }
            }
        }
    }
    endpoints.sort_by(|a, b| a.path.cmp(&b.path).then(a.method.cmp(&b.method)));

    let mut doc = ApiDoc::new("API 文档");
    doc.endpoints = endpoints;
    doc.schemas = collect_schemas(&doc.endpoints, &index.classes, &index.known);
    doc
}

fn is_controller(cls: &JavaClass) -> bool {
    cls.anns.iter().any(|a| {
        matches!(
            a.name.as_str(),
            "RestController" | "Controller" | "RequestMapping" | "Path" | "FeignClient"
        )
    })
}

fn class_base_path(cls: &JavaClass) -> String {
    for a in &cls.anns {
        if a.name == "FeignClient" {
            return ann_string(&a.args, "path").unwrap_or_default();
        }
        if a.name == "RequestMapping" || a.name == "Path" {
            if let Some(p) = ann_first_string(a) {
                return p;
            }
        }
    }
    String::new()
}

/// `(HTTP method, sub path)` pairs declared by a handler's annotations.
fn method_mappings(anns: &[Ann]) -> Vec<(String, String)> {
    // JAX-RS keeps the path in a separate `@Path`, Feign/Spring in the verb.
    let jaxrs_path = anns
        .iter()
        .find(|a| a.name == "Path")
        .and_then(ann_first_string)
        .unwrap_or_default();
    let mut out = Vec::new();
    for a in anns {
        let spring_verb = match a.name.as_str() {
            "GetMapping" => Some("GET"),
            "PostMapping" => Some("POST"),
            "PutMapping" => Some("PUT"),
            "DeleteMapping" => Some("DELETE"),
            "PatchMapping" => Some("PATCH"),
            _ => None,
        };
        if let Some(v) = spring_verb {
            out.push((v.to_string(), ann_first_string(a).unwrap_or_default()));
            continue;
        }
        let jaxrs_verb = match a.name.as_str() {
            "GET" => Some("GET"),
            "POST" => Some("POST"),
            "PUT" => Some("PUT"),
            "DELETE" => Some("DELETE"),
            "PATCH" => Some("PATCH"),
            "HEAD" => Some("HEAD"),
            "OPTIONS" => Some("OPTIONS"),
            _ => None,
        };
        if let Some(v) = jaxrs_verb {
            out.push((v.to_string(), jaxrs_path.clone()));
            continue;
        }
        if a.name == "RequestMapping" {
            let path = ann_first_string(a).unwrap_or_default();
            let verbs = explicit_verbs(a);
            if verbs.is_empty() {
                out.push(("GET".to_string(), path));
            } else {
                for v in verbs {
                    out.push((v, path.clone()));
                }
            }
        }
    }
    out
}

fn explicit_verbs(ann: &Ann) -> Vec<String> {
    const VERBS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "TRACE"];
    let Some(raw) = ann_attr(&ann.args, "method") else {
        return Vec::new();
    };
    let inner = raw.trim().trim_start_matches('{').trim_end_matches('}');
    let mut out = Vec::new();
    for part in split_top_level(inner, ',') {
        let verb = part.trim().rsplit('.').next().unwrap_or("").trim().to_uppercase();
        if VERBS.contains(&verb.as_str()) && !out.contains(&verb) {
            out.push(verb);
        }
    }
    out
}

fn build_endpoint(
    cls: &JavaClass,
    m: &crate::java::JavaMethod,
    method: &str,
    path: &str,
    known: &HashSet<String>,
) -> Vec<Endpoint> {
    let mut params = Vec::new();
    let mut body: Option<(String, Ty)> = None;

    for p in &m.params {
        let base = base_type(&p.ty);
        if SKIPPED_PARAMS.contains(&base.as_str()) {
            continue;
        }
        if let Some(a) = p.anns.iter().find(|a| a.name == "RequestBody") {
            let _ = a;
            body = Some(("application/json".into(), parse_ty(&p.ty, known, true)));
            continue;
        }
        if p.anns.iter().any(|a| a.name == "RequestPart") {
            let mut fields = Vec::new();
            fields.push(Field::new(p.name.clone(), Ty::Binary, true, ""));
            body = Some(("multipart/form-data".into(), Ty::Object(fields)));
            continue;
        }
        if base == "MultipartFile" || base == "Part" {
            let mut fields = Vec::new();
            fields.push(Field::new(p.name.clone(), Ty::Binary, true, ""));
            body = Some(("multipart/form-data".into(), Ty::Object(fields)));
            continue;
        }

        let (location, ann, name) = if let Some(a) = p.anns.iter().find(|a| a.name == "PathVariable") {
            (ParamIn::Path, Some(a), ann_name(a).unwrap_or_else(|| p.name.clone()))
        } else if let Some(a) = p.anns.iter().find(|a| a.name == "PathParam") {
            (ParamIn::Path, Some(a), ann_name(a).unwrap_or_else(|| p.name.clone()))
        } else if let Some(a) = p.anns.iter().find(|a| a.name == "RequestParam") {
            (ParamIn::Query, Some(a), ann_name(a).unwrap_or_else(|| p.name.clone()))
        } else if let Some(a) = p.anns.iter().find(|a| a.name == "QueryParam") {
            (ParamIn::Query, Some(a), ann_name(a).unwrap_or_else(|| p.name.clone()))
        } else if let Some(a) = p.anns.iter().find(|a| a.name == "RequestHeader") {
            (ParamIn::Header, Some(a), ann_name(a).unwrap_or_else(|| p.name.clone()))
        } else if let Some(a) = p.anns.iter().find(|a| a.name == "HeaderParam") {
            (ParamIn::Header, Some(a), ann_name(a).unwrap_or_else(|| p.name.clone()))
        } else if let Some(a) = p.anns.iter().find(|a| a.name == "CookieValue") {
            (ParamIn::Cookie, Some(a), ann_name(a).unwrap_or_else(|| p.name.clone()))
        } else if let Some(a) = p.anns.iter().find(|a| a.name == "CookieParam") {
            (ParamIn::Cookie, Some(a), ann_name(a).unwrap_or_else(|| p.name.clone()))
        } else {
            let simple = is_simple(&p.ty, known);
            let read_method = matches!(method, "GET" | "DELETE" | "HEAD" | "OPTIONS");
            if simple || read_method {
                (ParamIn::Query, None, p.name.clone())
            } else {
                body = Some(("application/json".into(), parse_ty(&p.ty, known, true)));
                continue;
            }
        };

        let swagger = p
            .anns
            .iter()
            .find(|a| matches!(a.name.as_str(), "Parameter" | "ApiParam"));
        if let Some(s) = swagger {
            if s.args.contains("hidden = true") || s.args.contains("hidden=true") {
                continue;
            }
        }
        let default = ann
            .and_then(|a| ann_string(&a.args, "defaultValue"))
            .or_else(|| {
                p.anns
                    .iter()
                    .find(|a| a.name == "DefaultValue")
                    .and_then(ann_first_string)
            })
            .filter(|v| !v.is_empty());
        let required = if location == ParamIn::Path {
            true
        } else if default.is_some() {
            false
        } else if let Some(s) = swagger {
            attr_bool(&s.args, "required").unwrap_or(true)
        } else {
            ann.and_then(|a| attr_bool(&a.args, "required")).unwrap_or(true)
        };
        let description = swagger
            .and_then(|s| ann_string(&s.args, "description").or_else(|| ann_string(&s.args, "value")))
            .or_else(|| m.doc.params.get(&name).cloned())
            .unwrap_or_default();
        params.push(Param {
            name,
            location,
            required,
            description,
            ty: parse_ty(&p.ty, known, false),
            default,
        });
    }

    if body.is_none() && !matches!(method, "GET" | "DELETE" | "HEAD" | "OPTIONS") {
        // No @RequestBody: Spring binds a single complex parameter from the body.
        if let Some(p) = m.params.iter().find(|p| {
            is_complex(&p.ty, known)
                && !SKIPPED_PARAMS.contains(&base_type(&p.ty).as_str())
                && p.anns.iter().all(|a| {
                    !matches!(
                        a.name.as_str(),
                        "RequestParam" | "PathVariable" | "RequestHeader" | "RequestPart"
                    )
                })
        }) {
            body = Some(("application/json".into(), parse_ty(&p.ty, known, true)));
        }
    }

    let response = if base_type(&m.ret) == "void" {
        None
    } else {
        Some(parse_ty(&m.ret, known, true))
    };

    let summary = operation_summary(&m.anns)
        .filter(|s| !s.is_empty())
        .or_else(|| (!m.doc.summary.is_empty()).then(|| m.doc.summary.clone()))
        .unwrap_or_else(|| m.name.clone());
    let mut description = operation_description(&m.anns)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| m.doc.description.clone());
    if !m.doc.ret.is_empty() {
        if !description.is_empty() {
            description.push_str("\n\n");
        }
        description.push_str("返回：");
        description.push_str(&m.doc.ret);
    }

    let operation_id = format!("{}_{}", lower_first(&cls.name), m.name);
    vec![Endpoint {
        method: method.to_string(),
        path: path.to_string(),
        tag: m
            .doc
            .tags
            .get("module")
            .or_else(|| cls.doc.tags.get("module"))
            .cloned()
            .or_else(|| class_tag_name(cls))
            .unwrap_or_else(|| cls.name.clone()),
        summary,
        description,
        operation_id,
        params,
        body,
        response,
        deprecated: m.doc.deprecated
            || m.anns.iter().any(|a| a.name == "Deprecated")
            || m.anns.iter().any(|a| {
                a.name == "Operation"
                    && (a.args.contains("deprecated = true") || a.args.contains("deprecated=true"))
            }),
        response_description: response_description(&m.anns).unwrap_or_else(|| "成功".into()),
        line: m.line,
    }]
}

/// Swagger 3 `@Tag(name = …)` / Swagger 2 `@Api(tags = {…})`.
fn class_tag_name(cls: &JavaClass) -> Option<String> {
    for a in &cls.anns {
        match a.name.as_str() {
            "Tag" => {
                if let Some(name) = ann_string(&a.args, "name") {
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
            "Api" => {
                if let Some(name) = ann_first_string(a) {
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
            _ => {}
        }
    }
    None
}
fn operation_summary(anns: &[Ann]) -> Option<String> {
    for a in anns {
        match a.name.as_str() {
            "Operation" => {
                if let Some(s) = ann_string(&a.args, "summary") {
                    return Some(s);
                }
            }
            "ApiOperation" => {
                if let Some(s) = ann_string(&a.args, "value") {
                    return Some(s);
                }
            }
            _ => {}
        }
    }
    None
}

fn operation_description(anns: &[Ann]) -> Option<String> {
    for a in anns {
        match a.name.as_str() {
            "Operation" => {
                if let Some(s) = ann_string(&a.args, "description") {
                    return Some(s);
                }
            }
            "ApiOperation" => {
                if let Some(s) = ann_string(&a.args, "notes") {
                    return Some(s);
                }
            }
            _ => {}
        }
    }
    None
}

fn response_description(anns: &[Ann]) -> Option<String> {
    for a in anns {
        if matches!(a.name.as_str(), "ApiResponse" | "ApiResponses") {
            if let Some(d) = ann_string(&a.args, "description") {
                return Some(d);
            }
        }
    }
    None
}

/// `@PathParam("id")`: a positional string only when no `key = value` is present.
fn positional_string(a: &Ann) -> Option<String> {
    if a.args.contains('=') {
        None
    } else {
        ann_first_string(a)
    }
}
fn ann_name(a: &Ann) -> Option<String> {
    ann_string(&a.args, "value")
        .or_else(|| ann_string(&a.args, "name"))
        .or_else(|| positional_string(a))
}

/// Very small annotation-argument reader; enough for `value=`, `path=`,
/// `required=false`, `defaultValue="1"`.
fn attr_bool(args: &str, key: &str) -> Option<bool> {
    ann_attr(args, key).map(|v| v.trim() == "true")
}

fn join_path(base: &str, sub: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in [base, sub] {
        for seg in p.split('/') {
            if !seg.is_empty() {
                parts.push(seg);
            }
        }
    }
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

fn lower_first(s: &str) -> String {
    let mut it = s.chars();
    match it.next() {
        Some(c) => c.to_lowercase().collect::<String>() + it.as_str(),
        None => String::new(),
    }
}

/// `Result<UserDto>` -> `UserDto`, `List<Long>` -> `List<Long>`.
pub fn unwrap_wrapper(raw: &str) -> String {
    let base = base_type(raw);
    if WRAPPERS.contains(&base.as_str()) {
        if let Some(inner) = generic_args(raw).first() {
            return inner.clone();
        }
    }
    raw.trim().to_string()
}

fn base_type(raw: &str) -> String {
    let t = raw.trim();
    let t = t.split('<').next().unwrap_or(t);
    t.rsplit('.').next().unwrap_or(t).trim().to_string()
}

pub fn generic_args(raw: &str) -> Vec<String> {
    let Some(open) = raw.find('<') else { return Vec::new() };
    let Some(close) = raw.rfind('>') else { return Vec::new() };
    if close <= open {
        return Vec::new();
    }
    split_top_level(&raw[open + 1..close], ',')
}

fn is_simple(raw: &str, known: &HashSet<String>) -> bool {
    let base = base_type(raw);
    if known.contains(&base) {
        return false;
    }
    !matches!(
        base.as_str(),
        "List" | "Set" | "Collection" | "Map" | "HashMap" | "Object" | "T" | "?"
    )
}

fn is_complex(raw: &str, known: &HashSet<String>) -> bool {
    !is_simple(raw, known)
}

/// Resolves a Java type expression into the schema model.
pub fn parse_ty(raw: &str, known: &HashSet<String>, unwrap: bool) -> Ty {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ty::Any;
    }
    if unwrap {
        let inner = unwrap_wrapper(raw);
        if inner != raw {
            return parse_ty(&inner, known, false);
        }
    }
    if let Some(stripped) = raw.strip_suffix("[]") {
        return Ty::Array(Box::new(parse_ty(stripped, known, false)));
    }
    let base = base_type(raw);
    let args = generic_args(raw);
    match base.as_str() {
        "String" | "CharSequence" | "char" | "Character" | "UUID" => Ty::String,
        "int" | "Integer" | "short" | "Short" | "byte" | "Byte" | "BigInteger" => Ty::Integer,
        "long" | "Long" => Ty::Long,
        "double" | "Double" | "float" | "Float" | "BigDecimal" | "Number" => Ty::Number,
        "boolean" | "Boolean" => Ty::Boolean,
        "MultipartFile" | "File" | "Path" | "Resource" | "InputStream" => Ty::Binary,
        "List" | "Set" | "Collection" | "Iterable" | "Queue" | "Deque" | "Stream" | "ArrayList"
        | "LinkedList" | "HashSet" => match args.first() {
            Some(a) => Ty::Array(Box::new(parse_ty(a, known, false))),
            None => Ty::Array(Box::new(Ty::Any)),
        },
        "Optional" => match args.first() {
            Some(a) => parse_ty(a, known, false),
            None => Ty::Any,
        },
        b if PAGE_TYPES.contains(&b) => {
            let item = args
                .first()
                .map(|a| parse_ty(a, known, false))
                .unwrap_or(Ty::Any);
            Ty::Object(vec![
                Field::new("records", Ty::Array(Box::new(item)), true, "当前页数据"),
                Field::new("total", Ty::Long, true, "总条数"),
                Field::new("size", Ty::Long, false, "每页条数"),
                Field::new("current", Ty::Long, false, "当前页"),
                Field::new("pages", Ty::Long, false, "总页数"),
            ])
        }
        "Map" | "HashMap" | "JSONObject" | "Object" | "T" | "?" => Ty::Any,
        b if known.contains(b) => Ty::Ref(b.to_string()),
        _ => Ty::Any,
    }
}

fn collect_schemas(
    endpoints: &[Endpoint],
    classes: &HashMap<String, &JavaClass>,
    known: &HashSet<String>,
) -> BTreeMap<String, Ty> {
    let mut schemas = BTreeMap::new();
    let mut queue: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for ep in endpoints {
        for p in &ep.params {
            collect_refs(&p.ty, &mut queue);
        }
        if let Some((_, ty)) = &ep.body {
            collect_refs(ty, &mut queue);
        }
        if let Some(ty) = &ep.response {
            collect_refs(ty, &mut queue);
        }
    }
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(cls) = classes.get(&name) else { continue };
        let ty = class_schema(cls, known, &mut queue);
        schemas.insert(name, ty);
    }
    schemas
}

fn collect_refs(ty: &Ty, out: &mut Vec<String>) {
    match ty {
        Ty::Ref(n) => out.push(n.clone()),
        Ty::Array(inner) => collect_refs(inner, out),
        Ty::Object(fields) => {
            for f in fields {
                collect_refs(&f.ty, out);
            }
        }
        _ => {}
    }
}

fn class_schema(cls: &JavaClass, known: &HashSet<String>, queue: &mut Vec<String>) -> Ty {
    if !cls.enum_values.is_empty() {
        return Ty::Enum(cls.enum_values.clone());
    }
    let mut fields = Vec::new();
    for f in &cls.fields {
        if is_hidden(f) {
            continue;
        }
        let ty = parse_ty(&f.ty, known, false);
        collect_refs(&ty, queue);
        let required = !PRIMITIVE_REQUIRED.contains(&base_type(&f.ty).as_str())
            && f.anns.iter().any(|a| {
                matches!(
                    a.name.as_str(),
                    "NotNull" | "NotBlank" | "NotEmpty" | "NonNull"
                )
            });
        let description = if f.doc.is_empty() {
            schema_description(&f.anns).unwrap_or_default()
        } else {
            f.doc.clone()
        };
        let mut field = Field::new(field_name(f), ty, required, description);
        apply_constraints(&mut field, &f.anns);
        fields.push(field);
    }
    Ty::Object(fields)
}

/// `@JsonProperty("user_name")` / `@JSONField(name = …)` / `@SerializedName` /
/// `@Schema(name = …)` rename the JSON property.
fn field_name(f: &JavaField) -> String {
    for a in &f.anns {
        let key = match a.name.as_str() {
            "JsonProperty" | "SerializedName" | "JSONField" => "value",
            "Schema" | "ApiModelProperty" => "name",
            _ => continue,
        };
        if let Some(v) = ann_string(&a.args, key) {
            if !v.is_empty() {
                return v;
            }
        }
        if matches!(a.name.as_str(), "JsonProperty" | "SerializedName" | "JSONField") {
            if let Some(v) = ann_first_string(a) {
                if !v.is_empty() {
                    return v;
                }
            }
        }
    }
    f.name.clone()
}

/// Bean Validation -> JSON Schema keywords.
fn apply_constraints(field: &mut Field, anns: &[Ann]) {
    // `@Min(1)` has a positional value, `@Min(value = 1)` a named one.
    let arg = |a: &Ann, key: &str| {
        ann_attr(&a.args, key)
            .map(|v| v.trim().trim_matches('"').to_string())
            .or_else(|| {
                let t = a.args.trim();
                if t.is_empty() || t.contains('=') {
                    None
                } else {
                    Some(t.trim_matches('"').to_string())
                }
            })
    };
    let num = |a: &Ann, key: &str| arg(a, key).and_then(|v| v.parse::<f64>().ok());
    let int = |a: &Ann, key: &str| arg(a, key).and_then(|v| v.parse::<u64>().ok());
    for a in anns {
        match a.name.as_str() {
            "Min" => field.min = num(a, "value"),
            "Max" => field.max = num(a, "value"),
            "DecimalMin" => field.min = num(a, "value"),
            "DecimalMax" => field.max = num(a, "value"),
            "Size" => {
                field.min_len = int(a, "min");
                field.max_len = int(a, "max");
            }
            "Length" => {
                field.min_len = int(a, "min");
                field.max_len = int(a, "max");
            }
            "Pattern" => {
                field.pattern = ann_string(&a.args, "regexp")
                    .or_else(|| ann_first_string(a))
                    .map(|p| unescape_java(&p));
            }
            "Email" => field.format = Some("email".into()),
            _ => {}
        }
    }
}

/// Java string escapes in annotation values: `"^\\d+$"` means `^\d+$`.
fn unescape_java(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}
fn is_hidden(f: &JavaField) -> bool {
    let base = base_type(&f.ty);
    if base.contains("serialVersionUID") || f.name == "serialVersionUID" {
        return true;
    }
    if f.anns.iter().any(|a| a.name == "JsonIgnore") {
        return true;
    }
    // `@JSONField(name = "x")` renames; only `serialize = false` hides.
    if f.anns.iter().any(|a| {
        a.name == "JSONField"
            && (a.args.contains("serialize = false") || a.args.contains("serialize=false"))
    }) {
        return true;
    }
    f.anns.iter().any(|a| {
        matches!(a.name.as_str(), "Schema" | "ApiModelProperty")
            && (a.args.contains("hidden = true") || a.args.contains("hidden=true"))
    })
}

fn schema_description(anns: &[Ann]) -> Option<String> {
    for a in anns {
        if matches!(a.name.as_str(), "Schema" | "ApiModelProperty") {
            if let Some(d) = ann_string(&a.args, "description").or_else(|| ann_string(&a.args, "value")) {
                return Some(d);
            }
        }
    }
    None
}