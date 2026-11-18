//! Apifox open-API client, minus the transport.
//!
//! Requests are built here (URL, headers, JSON body) and responses parsed
//! here; `host::fetch` in the plugin shell only performs the call. That keeps
//! the contract testable without a network or a token.
//!
//! Contracts used (verified against the open-API docs and the official CLI):
//! - `POST /v1/projects/{id}/import-openapi`, `X-Apifox-Api-Version: 2024-03-28`,
//!   `options.targetBranchId` / `options.moduleId` select the iteration branch
//!   and module (omitted = main branch / default module).
//! - `GET /api/v1/projects/{id}/sprint-branches` lists branches (`id`, `name`,
//!   `isMain`, `type` = SPRINT | VERSION, `automationLifecycleStatus` = AI).
//! - `GET /api/v1/projects/{id}/modules?branchId=` lists modules.
//!
//! The token is never read by the plugin: the shell sends
//! `{{secret:apifox_token}}` and the IDE fills it in on the way out.

use serde_json::{json, Value};

pub const API_BASE: &str = "https://api.apifox.com";
/// Header value for the public import API (its documented version).
pub const API_VERSION: &str = "2024-03-28";
/// The client endpoints (`/api/v1/...`) are versioned separately: the official
/// CLI sends this pair, and omitting `X-Apifox-Cli-Version` makes them answer
/// `400 400105 Client version too low`.
pub const CLIENT_VERSION: &str = "2.2.12";
pub const INTERNAL_API_VERSION: &str = "2026-05-28";
pub const SECRET_NAME: &str = "apifox_token";
/// AUTO_MERGE keeps hand edits made in Apifox when the same method+path is
/// imported again.
pub const OVERWRITE_BEHAVIOUR: &str = "AUTO_MERGE";

/// One HTTP call the shell has to perform.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiCall {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportOptions {
    /// Iteration branch; `None` = main branch.
    pub branch_id: Option<u64>,
    /// Module; `None` = the project's default module.
    pub module_id: Option<u64>,
    /// Folder for endpoints; `None` = root.
    pub endpoint_folder_id: Option<u64>,
    /// Folder for data models; `None` = root.
    pub schema_folder_id: Option<u64>,
    /// `AUTO_MERGE` | `OVERWRITE_EXISTING` | `KEEP_EXISTING` | `CREATE_NEW`.
    pub endpoint_mode: String,
    pub schema_mode: String,
    /// `updateFolderOfChangedEndpoint`. 
    pub update_folder: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            branch_id: None,
            module_id: None,
            endpoint_folder_id: None,
            schema_folder_id: None,
            endpoint_mode: OVERWRITE_BEHAVIOUR.into(),
            schema_mode: OVERWRITE_BEHAVIOUR.into(),
            update_folder: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub id: u64,
    pub name: String,
    pub is_main: bool,
    /// `main` | `sprint` | `general` | `ai` | `unknown`
    pub kind: String,
}

impl Branch {
    /// Label for a picker: `dev（迭代分支）`.
    pub fn label(&self) -> String {
        let kind = match self.kind.as_str() {
            "main" => "主分支",
            "sprint" => "迭代分支",
            "general" => "通用分支",
            "ai" => "AI 分支",
            _ => "分支",
        };
        format!("{}（{kind}）", self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportOutcome {
    pub created: u64,
    pub updated: u64,
    pub failed: u64,
    pub ignored: u64,
    pub schemas_created: u64,
    pub schemas_updated: u64,
    pub errors: Vec<String>,
}

impl ImportOutcome {
    pub fn summary(&self) -> String {
        format!(
            "新建 {} 个接口，更新 {} 个，忽略 {} 个，失败 {} 个；数据模型新建 {}、更新 {}",
            self.created, self.updated, self.ignored, self.failed, self.schemas_created, self.schemas_updated
        )
    }
}

fn headers(json_body: bool) -> Vec<(String, String)> {
    let mut v = vec![
        ("X-Apifox-Api-Version".into(), API_VERSION.into()),
        ("Authorization".into(), format!("Bearer {{{{secret:{SECRET_NAME}}}}}")),
    ];
    if json_body {
        v.push(("Content-Type".into(), "application/json".into()));
    }
    v
}

/// Headers the official CLI uses for `/api/v1/...` (branch/module lists).
fn internal_headers(project_id: Option<&str>, branch_id: Option<u64>) -> Vec<(String, String)> {
    let mut v = vec![
        ("X-Apifox-Cli-Version".into(), CLIENT_VERSION.into()),
        ("X-Apifox-Api-Version".into(), INTERNAL_API_VERSION.into()),
        ("Authorization".into(), format!("Bearer {{{{secret:{SECRET_NAME}}}}}")),
    ];
    if let Some(project_id) = project_id {
        v.push(("X-Project-Id".into(), project_id.to_string()));
    }
    if let Some(id) = branch_id {
        v.push(("X-Branch-Id".into(), id.to_string()));
    }
    v
}

/// `GET /api/v1/projects/{id}/sprint-branches` (the CLI's `branch list`).
pub fn branches_request(project_id: &str) -> ApiCall {
    ApiCall {
        method: "GET",
        url: format!("{API_BASE}/api/v1/projects/{project_id}/sprint-branches"),
        headers: internal_headers(Some(project_id), None),
        body: None,
    }
}

/// `GET /api/v1/projects/{id}/modules?branchId=…`
pub fn modules_request(project_id: &str, branch_id: Option<u64>) -> ApiCall {
    let mut url = format!("{API_BASE}/api/v1/projects/{project_id}/modules");
    if let Some(id) = branch_id {
        url.push_str(&format!("?branchId={id}"));
    }
    ApiCall {
        method: "GET",
        url,
        headers: internal_headers(Some(project_id), branch_id),
        body: None,
    }
}

/// A folder in the project (`api-detail-folders` / `api-schema-detail-folders`).
#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub id: u64,
    /// `父/子` path built from `parentId`.
    pub path: String,
}

pub fn folders_request(
    project_id: &str,
    module_id: Option<u64>,
    branch_id: Option<u64>,
    schema: bool,
) -> ApiCall {
    let kind = if schema {
        "api-schema-detail-folders"
    } else {
        "api-detail-folders"
    };
    let mut url = format!("{API_BASE}/api/v1/projects/{project_id}/{kind}");
    if let Some(module) = module_id {
        url.push_str(&format!("?moduleId={module}"));
    }
    ApiCall {
        method: "GET",
        url,
        headers: internal_headers(Some(project_id), branch_id),
        body: None,
    }
}

pub fn parse_folders(status: u16, text: &str) -> Result<Vec<Folder>, String> {
    use std::collections::HashMap;
    let value = checked(status, text)?;
    let mut raw: HashMap<u64, (String, Option<u64>)> = HashMap::new();
    for item in items(&value) {
        let Some(id) = number(item.get("id")) else { continue };
        let name = item.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        let parent = number(item.get("parentId")).filter(|p| *p != 0);
        raw.insert(id, (name, parent));
    }
    let mut cache: HashMap<u64, String> = HashMap::new();
    fn path_of(
        id: u64,
        raw: &std::collections::HashMap<u64, (String, Option<u64>)>,
        cache: &mut std::collections::HashMap<u64, String>,
        depth: usize,
    ) -> String {
        if let Some(hit) = cache.get(&id) {
            return hit.clone();
        }
        let Some((name, parent)) = raw.get(&id) else {
            return String::new();
        };
        let path = match parent {
            Some(p) if depth < 32 => {
                let head = path_of(*p, raw, cache, depth + 1);
                if head.is_empty() {
                    name.clone()
                } else {
                    format!("{head}/{name}")
                }
            }
            _ => name.clone(),
        };
        cache.insert(id, path.clone());
        path
    }
    let mut out: Vec<Folder> = raw
        .keys()
        .map(|id| Folder { id: *id, path: path_of(*id, &raw, &mut cache, 0) })
        .filter(|f| !f.path.is_empty())
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
/// A team or project: `{ id, name }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Named {
    pub id: u64,
    pub name: String,
}

/// `GET /api/v1/user-teams` (the CLI's `team list`).
pub fn teams_request() -> ApiCall {
    ApiCall {
        method: "GET",
        url: format!("{API_BASE}/api/v1/user-teams"),
        headers: internal_headers(None, None),
        body: None,
    }
}

/// `GET /api/v1/teams/{teamId}/projects` (the CLI's `project list --team`).
pub fn projects_request(team_id: &str) -> ApiCall {
    ApiCall {
        method: "GET",
        url: format!("{API_BASE}/api/v1/teams/{team_id}/projects"),
        headers: internal_headers(None, None),
        body: None,
    }
}

pub fn parse_named(status: u16, text: &str) -> Result<Vec<Named>, String> {
    let value = checked(status, text)?;
    let mut out = Vec::new();
    for item in items(&value) {
        let Some(id) = number(item.get("id")) else { continue };
        let name = item.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        if !name.is_empty() {
            out.push(Named { id, name });
        }
    }
    Ok(out)
}
pub fn import_request(project_id: &str, openapi_json: &str, opts: &ImportOptions) -> ImportRequest {
    let url = format!("{API_BASE}/v1/projects/{project_id}/import-openapi");
    let mut options = json!({
        "endpointOverwriteBehavior": opts.endpoint_mode,
        "schemaOverwriteBehavior": opts.schema_mode,
        "prependBasePath": false
    });
    if let Some(branch) = opts.branch_id {
        options["targetBranchId"] = json!(branch);
    }
    if let Some(module) = opts.module_id {
        options["moduleId"] = json!(module);
    }
    if let Some(folder) = opts.endpoint_folder_id {
        options["targetEndpointFolderId"] = json!(folder);
    }
    if let Some(folder) = opts.schema_folder_id {
        options["targetSchemaFolderId"] = json!(folder);
    }
    if opts.update_folder {
        options["updateFolderOfChangedEndpoint"] = json!(true);
    }
    let body = json!({ "input": openapi_json, "options": options }).to_string();
    ImportRequest {
        url,
        headers: headers(true),
        body,
    }
}

/// Branches from `{ "success": true, "data": [ … ] }`.
pub fn parse_branches(status: u16, text: &str) -> Result<Vec<Branch>, String> {
    let value = checked(status, text)?;
    let mut out = Vec::new();
    for item in items(&value) {
        let Some(id) = number(item.get("id")) else { continue };
        let name = item
            .get("name")
            .or_else(|| item.get("branchName"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        out.push(Branch {
            id,
            name,
            is_main: item.get("isMain").and_then(Value::as_bool).unwrap_or(false),
            kind: branch_kind(item),
        });
    }
    // Main first, then sprints, general branches, AI branches.
    out.sort_by_key(|b| {
        let rank = match b.kind.as_str() {
            "main" => 0,
            "sprint" => 1,
            "general" => 2,
            "ai" => 3,
            _ => 4,
        };
        (rank, b.name.clone())
    });
    Ok(out)
}

pub fn parse_modules(status: u16, text: &str) -> Result<Vec<Module>, String> {
    let value = checked(status, text)?;
    let mut out = Vec::new();
    for item in items(&value) {
        let Some(id) = number(item.get("id")) else { continue };
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        out.push(Module { id, name });
    }
    Ok(out)
}

fn branch_kind(item: &Value) -> String {
    if item.get("automationLifecycleStatus").map(|v| !v.is_null()).unwrap_or(false) {
        return "ai".into();
    }
    if item.get("isMain").and_then(Value::as_bool).unwrap_or(false) {
        return "main".into();
    }
    match item.get("type").and_then(Value::as_str).unwrap_or("") {
        "SPRINT" => "sprint".into(),
        "VERSION" => "general".into(),
        _ => "unknown".into(),
    }
}

fn items(value: &Value) -> Vec<&Value> {
    let Some(data) = value.get("data") else {
        return Vec::new();
    };
    if let Some(list) = data.as_array() {
        return list.iter().collect();
    }
    if let Some(list) = data.get("list").and_then(Value::as_array) {
        return list.iter().collect();
    }
    Vec::new()
}

fn number(v: Option<&Value>) -> Option<u64> {
    let v = v?;
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

pub fn parse_response(status: u16, text: &str) -> Result<ImportOutcome, String> {
    let value = checked(status, text)?;
    let data = value.get("data").cloned().unwrap_or(Value::Null);
    let counters = data.get("counters").cloned().unwrap_or(Value::Null);
    let count = |key: &str| counters.get(key).and_then(Value::as_u64).unwrap_or(0);

    let mut errors = Vec::new();
    if let Some(list) = data.get("errors").and_then(Value::as_array) {
        for item in list {
            let message = item.get("message").and_then(Value::as_str).unwrap_or("").trim();
            let code = item.get("code").and_then(Value::as_str).unwrap_or("").trim();
            if message.is_empty() {
                continue;
            }
            errors.push(if code.is_empty() {
                message.to_string()
            } else {
                format!("[{code}] {message}")
            });
        }
    }

    Ok(ImportOutcome {
        created: count("endpointCreated"),
        updated: count("endpointUpdated"),
        failed: count("endpointFailed"),
        ignored: count("endpointIgnored"),
        schemas_created: count("schemaCreated"),
        schemas_updated: count("schemaUpdated"),
        errors,
    })
}

/// Status + `success: false` check shared by every response.
fn checked(status: u16, text: &str) -> Result<Value, String> {
    if !(200..300).contains(&status) {
        return Err(format!("Apifox 返回 HTTP {status}：{}", truncate(text, 600)));
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|e| format!("无法解析 Apifox 响应（{e}）：{}", truncate(text, 600)))?;
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        let message = value
            .get("errorMessage")
            .or_else(|| value.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("请求失败");
        return Err(format!("Apifox：{}", truncate(message, 600)));
    }
    Ok(value)
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}…（已截断）")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_documented_request() {
        let req = import_request("3760990", "{\"openapi\":\"3.0.3\"}", &ImportOptions::default());
        assert_eq!(
            req.url,
            "https://api.apifox.com/v1/projects/3760990/import-openapi"
        );
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "X-Apifox-Api-Version" && v == "2024-03-28"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer {{secret:apifox_token}}"));
        let body: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["options"]["endpointOverwriteBehavior"], "AUTO_MERGE");
        assert_eq!(body["options"]["prependBasePath"], false);
        assert!(body["options"].get("targetBranchId").is_none());
        assert!(body["options"].get("moduleId").is_none());
        assert_eq!(body["input"], "{\"openapi\":\"3.0.3\"}");
    }

    #[test]
    fn import_targets_a_branch_and_module() {
        let opts = ImportOptions {
            branch_id: Some(4321),
            module_id: Some(77),
            ..ImportOptions::default()
        };
        let req = import_request("42", "{}", &opts);
        let body: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["options"]["targetBranchId"], 4321);
        assert_eq!(body["options"]["moduleId"], 77);
    }

    #[test]
    fn import_can_target_folders_and_choose_overwrite_modes() {
        let opts = ImportOptions {
            branch_id: Some(1),
            module_id: Some(2),
            endpoint_folder_id: Some(10),
            schema_folder_id: Some(11),
            endpoint_mode: "OVERWRITE_EXISTING".into(),
            schema_mode: "KEEP_EXISTING".into(),
            update_folder: true,
        };
        let req = import_request("4", "{}", &opts);
        let body: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["options"]["targetEndpointFolderId"], 10);
        assert_eq!(body["options"]["targetSchemaFolderId"], 11);
        assert_eq!(body["options"]["endpointOverwriteBehavior"], "OVERWRITE_EXISTING");
        assert_eq!(body["options"]["schemaOverwriteBehavior"], "KEEP_EXISTING");
    }
    #[test]
    fn folder_paths_follow_parent_id() {
        let text = r#"{"success":true,"data":[
            {"id":1,"name":"用户"},
            {"id":2,"name":"管理","parentId":1},
            {"id":3,"name":"订单"}
        ]}"#;
        let folders = parse_folders(200, text).unwrap();
        let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["用户", "用户/管理", "订单"]);
    }
    #[test]
    fn lists_branches_with_main_first() {
        let text = r#"{"success":true,"data":[
            {"id":9,"name":"dev","isMain":false,"type":"SPRINT"},
            {"id":1,"name":"main","isMain":true,"type":"VERSION"},
            {"id":12,"name":"ai/20260101","isMain":false,"type":"SPRINT","automationLifecycleStatus":"ACTIVE"},
            {"id":8,"name":"v2","isMain":false,"type":"VERSION"}
        ]}"#;
        let list = parse_branches(200, text).unwrap();
        assert_eq!(
            list.iter().map(|b| b.kind.as_str()).collect::<Vec<_>>(),
            vec!["main", "sprint", "general", "ai"]
        );
        assert_eq!(list[0].name, "main");
        assert!(list[0].is_main);
        assert_eq!(list[1].label(), "dev（迭代分支）");
        assert_eq!(list[3].label(), "ai/20260101（AI 分支）");
    }

    #[test]
    fn branch_ids_may_be_strings() {
        let text = r#"{"success":true,"data":[{"id":"77","name":"dev","type":"SPRINT"}]}"#;
        assert_eq!(parse_branches(200, text).unwrap()[0].id, 77);
    }

    #[test]
    fn lists_modules_from_a_wrapped_list() {
        let text = r#"{"success":true,"data":{"list":[{"id":3,"name":"默认模块"}]}}"#;
        let list = parse_modules(200, text).unwrap();
        assert_eq!(list, vec![Module { id: 3, name: "默认模块".into() }]);
    }

    #[test]
    fn branch_request_is_a_get_with_auth() {
        let call = branches_request("42");
        assert_eq!(call.method, "GET");
        assert_eq!(call.url, "https://api.apifox.com/api/v1/projects/42/sprint-branches");
        assert!(call.body.is_none());
        assert!(call.headers.iter().any(|(k, _)| k == "Authorization"));
        assert!(
            call.headers
                .iter()
                .any(|(k, v)| k == "X-Apifox-Cli-Version" && v == CLIENT_VERSION),
            "the client endpoints reject requests without a client version"
        );
        assert!(call
            .headers
            .iter()
            .any(|(k, v)| k == "X-Apifox-Api-Version" && v == INTERNAL_API_VERSION));
        assert!(call.headers.iter().any(|(k, v)| k == "X-Project-Id" && v == "42"));
        assert!(!call.headers.iter().any(|(k, _)| k == "X-Branch-Id"));

        let call = modules_request("42", Some(7));
        assert!(call.url.ends_with("/modules?branchId=7"));
        assert!(call.headers.iter().any(|(k, v)| k == "X-Branch-Id" && v == "7"));
    }

    #[test]
    fn failed_branch_list_keeps_the_body() {
        let err = parse_branches(403, r#"{"errorMessage":"no permission"}"#).unwrap_err();
        assert!(err.contains("403"));
        assert!(err.contains("no permission"));
        let err = parse_branches(200, r#"{"success":false,"errorMessage":"token invalid"}"#).unwrap_err();
        assert!(err.contains("token invalid"));
    }

    #[test]
    fn reads_counters_and_partial_errors() {
        let text = r#"{"data":{"counters":{"endpointCreated":3,"endpointUpdated":1,
            "endpointIgnored":0,"endpointFailed":1,"schemaCreated":2,"schemaUpdated":0},
            "errors":[{"code":"invalidSchema","message":"字段类型无法识别"}]}}"#;
        let out = parse_response(200, text).unwrap();
        assert_eq!(out.created, 3);
        assert_eq!(out.updated, 1);
        assert_eq!(out.failed, 1);
        assert_eq!(out.schemas_created, 2);
        assert_eq!(out.errors, vec!["[invalidSchema] 字段类型无法识别"]);
    }

    #[test]
    fn http_error_keeps_the_body() {
        let err = parse_response(401, r#"{"message":"token invalid"}"#).unwrap_err();
        assert!(err.contains("401"));
        assert!(err.contains("token invalid"));
    }
}