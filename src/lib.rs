//! OpenIDEA shell for the Apifox export.
//!
//! Two surfaces share one storage:
//! - the `settings` view holds the server + access token;
//! - the `mapping` view holds one row per **code module** (Maven/Gradle),
//!   each mapping to its own Apifox project / branch / module.
//!
//! Double-clicking a row opens a form with real dropdowns (branch/module),
//! because table cells are text-only in this host. `Apifox Sync: 同步` walks the
//! enabled rows, scans the module's sources and imports each separately.

use apifox_core::{
    apifox, openapi, postman, prefix_paths, project as code_project, scan_project_files,
    yaml,
};
use openidea_plugin::views::{
    Body, Button, Field, FieldValue, Table, View, ViewEvent, Viewport, Views,
};
use openidea_plugin::{
    host, register_plugin, Error, HttpRequest, Invocation, NotificationLevel, Plugin, Project,
};
use serde_json::{json, Value};

const TITLE: &str = "Apifox Sync";
const MAX_FILES: u32 = 20_000;
const OPENAPI_PATH: &str = "docs/openapi.json";
const APIFOX_SECRET: &str = "apifox_token";

/// One code module -> one Apifox target.
#[derive(Clone)]
struct Mapping {
    enabled: bool,
    /// Display name (Maven artifactId / Gradle project name).
    name: String,
    /// Directory relative to the project root; `""` = whole project.
    path: String,
    project_id: String,
    branch_id: Option<u64>,
    branch_name: String,
    module_id: Option<u64>,
    module_name: String,
    /// Apifox folder ids; `None` = project root.
    endpoint_folder_id: Option<u64>,
    schema_folder_id: Option<u64>,
    /// Overwrite behaviours (`AUTO_MERGE`, `OVERWRITE_EXISTING`, ...).
    endpoint_mode: String,
    schema_mode: String,
    /// Property/parameter naming (`original`, `camel`, `snake`, ...).
    naming: String,
    /// `updateFolderOfChangedEndpoint`.
    update_folder: bool,
}

impl Default for Mapping {
    fn default() -> Self {
        Self {
            enabled: true,
            name: String::new(),
            path: String::new(),
            project_id: String::new(),
            branch_id: None,
            branch_name: String::new(),
            module_id: None,
            module_name: String::new(),
            endpoint_folder_id: None,
            schema_folder_id: None,
            endpoint_mode: "AUTO_MERGE".into(),
            schema_mode: "AUTO_MERGE".into(),
            naming: "original".into(),
            update_folder: false,
        }
    }
}

impl Mapping {
    fn branch_label(&self) -> String {
        if self.branch_name.is_empty() {
            "(主分支)".to_string()
        } else {
            self.branch_name.clone()
        }
    }

    fn module_label(&self) -> String {
        if self.module_name.is_empty() {
            "(默认模块)".to_string()
        } else {
            self.module_name.clone()
        }
    }
}

#[derive(Default)]
pub struct Main {
    mappings: Vec<Mapping>,
    /// Root the mappings were loaded for.
    loaded_for: String,
    /// Row being edited in the form (None = table).
    editing: Option<usize>,
    e_enabled: bool,
    e_project: String,
    e_branch: String,
    e_module: String,
    e_endpoint_folder: String,
    e_schema_folder: String,
    e_endpoint_mode: String,
    e_schema_mode: String,
    e_naming: String,
    e_update_folder: bool,
    server: String,
    server_draft: String,
    teams: Vec<apifox::Named>,
    projects: Vec<apifox::Named>,
    detected: Vec<code_project::CodeModule>,
    e_team: String,
    e_project_pick: String,
    e_code_module: String,
    endpoint_folders: Vec<apifox::Folder>,
    schema_folders: Vec<apifox::Folder>,
    selected_row: Option<usize>,
    branches: Vec<apifox::Branch>,
    modules: Vec<apifox::Module>,
    token_draft: String,
    status: Option<String>,
}

impl Plugin for Main {
    fn run_command(
        &mut self,
        id: &str,
        ctx: &Invocation,
        _arg: Option<&str>,
    ) -> Result<Option<String>, Error> {
        match id {
            "sync-apifox" => sync(ctx),
            "export-openapi" => export(ctx),
            "export-yaml" => export_yaml(ctx),
            "export-postman" => export_postman(ctx),
            "configure-apifox" => configure(ctx),
            other => Err(format!("未知命令 {other}").into()),
        }
    }
}

const NO_BRANCH: &str = "(主分支)";
const DEFAULT_MODULE: &str = "(默认模块)";

impl Views for Main {
    fn render(
        &mut self,
        view: &str,
        _viewport: Option<Viewport>,
        ctx: &Invocation,
    ) -> Result<View, Error> {
        let Some(project) = ctx.project() else {
            return Ok(View::message("请先打开一个 Java 项目，再打开 Apifox Sync。"));
        };
        let root = project.root().unwrap_or_default();
        if self.loaded_for != root {
            self.mappings = load_mappings(&root);
            self.server = server_base(&root);
            self.loaded_for = root.clone();
            self.editing = None;
        }
        if view == "mapping" {
            if let Some(i) = self.editing {
                if i < self.mappings.len() {
                    return Ok(self.editor_view(i, &root));
                }
                self.editing = None;
            }
            return Ok(self.mapping_view(&root));
        }
        Ok(self.settings_view())
    }

    fn handle(&mut self, view: &str, event: ViewEvent, ctx: &Invocation) -> Result<(), Error> {
        let root = ctx
            .project()
            .map(|p| p.root().unwrap_or_default())
            .unwrap_or_default();

        if view == "mapping" {
            if let Some(i) = self.editing {
                self.handle_editor(i, event, &root, ctx)?;
            } else {
                self.handle_table(event, &root, ctx)?;
            }
        } else {
            match event {
                ViewEvent::Field(key, value) if key == "token" => {
                    self.token_draft = value;
                    return Ok(());
                }
                ViewEvent::Field(key, value) if key == "server_url" => {
                    self.server_draft = value.trim().to_string();
                    return Ok(());
                }
                ViewEvent::Field(key, value) if key == "server_kind" => {
                    if value == "公网 SaaS" {
                        self.server_draft = DEFAULT_SERVER.to_string();
                    }
                    return Ok(());
                }
                ViewEvent::Button(id) if id == "save_server" => {
                    let url = if self.server_draft.trim().is_empty() {
                        DEFAULT_SERVER.to_string()
                    } else {
                        self.server_draft.trim().trim_end_matches('/').to_string()
                    };
                    if !url.starts_with("http://") && !url.starts_with("https://") {
                        self.status = Some("服务器地址必须以 http:// 或 https:// 开头".into());
                        return Ok(());
                    }
                    host::storage::set(&server_key(&root), &url)?;
                    self.server = url.clone();
                    self.server_draft.clear();
                    self.status = Some(format!("服务器地址已设为 {url}"));
                }
                ViewEvent::Button(id) if id == "test" => {
                    let project = self
                        .mappings
                        .iter()
                        .find_map(|m| (!m.project_id.is_empty()).then(|| m.project_id.clone()));
                    match project {
                        Some(pid) => {
                            self.status = Some(match call_apifox(&apifox::branches_request(&pid), &self.server) {
                                Ok((status, _)) if (200..300).contains(&status) => {
                                    format!("连接正常（项目 {pid} 的分支列表可读）")
                                }
                                Ok((status, text)) => format!("连接失败：HTTP {status} {text}"),
                                Err(e) => format!("连接失败：{e}"),
                            });
                        }
                        None => self.status = Some("先在映射里填一个 Apifox 项目 ID".into()),
                    }
                }
                ViewEvent::Button(id) if id == "save_token" => {
                    if !self.token_draft.trim().is_empty() {
                        let raw = self.token_draft.trim();
                        let token = raw.strip_prefix("Bearer ").unwrap_or(raw).trim();
                        host::secret_set(APIFOX_SECRET, token)?;
                        self.token_draft.clear();
                        self.status = Some("访问令牌已保存到钥匙串".into());
                    }
                }
                _ => return Ok(()),
            }
        }
        host::refresh_view(view);
        Ok(())
    }
}

impl Main {
    fn settings_view(&mut self) -> View {
        let token_set = host::secret_is_set(APIFOX_SECRET);
        let public = self.server == DEFAULT_SERVER;
        let fields = vec![
            Field::new(
                "server_kind",
                "Apifox 服务器",
                FieldValue::Choice(
                    vec!["公网 SaaS".into(), "私有化部署".into()],
                    if public { "公网 SaaS".into() } else { "私有化部署".into() },
                ),
            ),
            Field::new(
                "server_url",
                "地址",
                FieldValue::Text(if self.server_draft.is_empty() {
                    self.server.clone()
                } else {
                    self.server_draft.clone()
                }),
            ),
            Field::new(
                "token",
                "API 访问令牌（留空保持不变）",
                FieldValue::Text(self.token_draft.clone()),
            ),
            Field::new(
                "token_state",
                "令牌状态",
                FieldValue::label(if token_set { "已保存到系统钥匙串" } else { "未设置" }),
            ),
            Field::new(
                "hint",
                "映射",
                FieldValue::label("在「映射」标签里按代码模块配置项目/分支/模块"),
            ),
        ];
        View::new(Body::Form(fields))
            .button(Button::new("save_server", "保存服务器"))
            .button(Button::new("save_token", "保存令牌"))
            .button(Button::new("test", "测试连接").icon("run"))
            .status(
                &self
                    .status
                    .clone()
                    .unwrap_or_else(|| "令牌存钥匙串，插件读不到明文".into()),
            )
    }

    fn mapping_view(&self, root: &str) -> View {
        let rows: Vec<Vec<String>> = self
            .mappings
            .iter()
            .map(|m| {
                vec![
                    if m.enabled { "是".into() } else { "否".into() },
                    if m.name.is_empty() { m.path.clone() } else { m.name.clone() },
                    if m.project_id.is_empty() { "—".into() } else { m.project_id.clone() },
                    m.branch_label(),
                    m.module_label(),
                    folder_label(m.endpoint_folder_id),
                    folder_label(m.schema_folder_id),
                    format!("{} / {}", mode_label(&m.endpoint_mode), mode_label(&m.schema_mode)),
                ]
            })
            .collect();
        let table = Table {
            columns: vec![
                "启用".into(),
                "代码模块".into(),
                "Apifox 项目".into(),
                "分支/版本".into(),
                "模块".into(),
                "接口目录".into(),
                "模型目录".into(),
                "覆盖策略（接口/模型）".into(),
            ],
            total_rows: rows.len() as u32,
            first_row: 0,
            rows,
            // Not editable: double-click/Enter must open the editor form, where
            // the project ID, branch and module are; editable cells swallow it.
            editable: false,
        };
        let status = self.status.clone().unwrap_or_else(|| {
            if self.mappings.is_empty() {
                "点 + 新增一行，再在编辑页选源代码模块".into()
            } else {
                "双击任意行打开编辑（项目 ID、分支、模块、启用都在里面）".to_string()
            }
        });
        let _ = root;
        View::new(Body::Table(table))
            .button(Button::new("add_row", "+"))
            .button(Button::new("edit_row", "编辑"))
            .button(Button::new("delete_row", "−"))
            .button(Button::new("clear_all", "清空"))
            .button(Button::new("from_config", "从配置导入"))
            .button(Button::new("preview", "预览变更"))
            .button(Button::new("refresh", "刷新分支").icon("refresh"))
            .button(Button::new("sync", "同步全部").icon("run"))
            .status(&status)
    }

    fn editor_view(&self, index: usize, root: &str) -> View {
        let m = &self.mappings[index];
        let mut branches = vec![NO_BRANCH.to_string()];
        branches.extend(self.branches.iter().map(|b| b.label()));
        let mut modules = vec![DEFAULT_MODULE.to_string()];
        modules.extend(self.modules.iter().map(|m| m.name.clone()));
        let branch_selected = if branches.contains(&self.e_branch) {
            self.e_branch.clone()
        } else {
            NO_BRANCH.to_string()
        };
        let module_selected = if modules.contains(&self.e_module) {
            self.e_module.clone()
        } else {
            DEFAULT_MODULE.to_string()
        };
        let mut code_labels: Vec<String> = self
            .module_labels()
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        if code_labels.is_empty() {
            code_labels.push(self.code_label(&m.path));
        }
        let selected_code = if code_labels.contains(&self.e_code_module) {
            self.e_code_module.clone()
        } else {
            code_labels[0].clone()
        };
        let mut team_labels = vec!["（未选择）".to_string()];
        team_labels.extend(self.teams.iter().map(|t| t.name.clone()));
        let selected_team = if team_labels.contains(&self.e_team) {
            self.e_team.clone()
        } else {
            "（未选择）".to_string()
        };
        let mut project_labels = vec!["（未选择）".to_string()];
        project_labels.extend(self.projects.iter().map(|p| p.name.clone()));
        let selected_project = if project_labels.contains(&self.e_project_pick) {
            self.e_project_pick.clone()
        } else {
            "（未选择）".to_string()
        };
        let ep_folders = folder_options(&self.endpoint_folders, m.endpoint_folder_id, &self.e_endpoint_folder);
        let sc_folders = folder_options(&self.schema_folders, m.schema_folder_id, &self.e_schema_folder);
        let fields = vec![
            Field::new(
                "e_code_module",
                "源代码模块",
                FieldValue::Choice(code_labels, selected_code),
            ),
            Field::new(
                "e_enabled",
                "启用",
                FieldValue::Checkbox(self.e_enabled),
            ),
            Field::new("e_team", "团队", FieldValue::Choice(team_labels, selected_team)),
            Field::new(
                "e_project_pick",
                "Apifox 项目",
                FieldValue::Choice(project_labels, selected_project),
            ),
            Field::new(
                "e_project",
                "Apifox 项目 ID（必填，数字；也可用上面的项目下拉回填）",
                FieldValue::Text(self.e_project.clone()),
            ),
            Field::new("e_branch", "分支 / 版本", FieldValue::Choice(branches, branch_selected)),
            Field::new("e_module", "模块", FieldValue::Choice(modules, module_selected)),
            Field::new(
                "e_endpoint_folder",
                "接口导入到",
                FieldValue::Choice(ep_folders.0, ep_folders.1),
            ),
            Field::new(
                "e_schema_folder",
                "数据模型导入到",
                FieldValue::Choice(sc_folders.0, sc_folders.1),
            ),
            Field::new(
                "e_endpoint_mode",
                "接口覆盖策略",
                FieldValue::Choice(
                    MODES.iter().map(|(l, _)| l.to_string()).collect(),
                    self.e_endpoint_mode.clone(),
                ),
            ),
            Field::new(
                "e_schema_mode",
                "数据模型覆盖策略",
                FieldValue::Choice(
                    MODES.iter().map(|(l, _)| l.to_string()).collect(),
                    self.e_schema_mode.clone(),
                ),
            ),
            Field::new(
                "e_naming",
                "参数命名转换",
                FieldValue::Choice(
                    NAMINGS.iter().map(|(l, _)| l.to_string()).collect(),
                    naming_label(&self.e_naming).to_string(),
                ),
            ),
            Field::new(
                "e_update_folder",
                "覆盖时同步更新接口目录",
                FieldValue::Checkbox(self.e_update_folder),
            ),
        ];
        let _ = root;
        View::new(Body::Form(fields))
            .button(Button::new("save", "保存").icon("save"))
            .button(Button::new("reload_modules", "刷新模块").icon("refresh"))
            .button(Button::new("reload_folders", "刷新目录").icon("refresh"))
            .button(Button::new("reload_teams", "刷新团队").icon("refresh"))
            .button(Button::new("reload", "刷新分支").icon("refresh"))
            .button(Button::new("delete", "删除该行").icon("remove"))
            .button(Button::new("cancel", "返回列表"))
            .status(
                &self
                    .status
                    .clone()
                    .unwrap_or_else(|| "选分支会自动拉该分支的模块".into()),
            )
    }

    fn handle_table(
        &mut self,
        event: ViewEvent,
        root: &str,
        ctx: &Invocation,
    ) -> Result<(), Error> {
        if let Some(row) = event.table_select() {
            self.selected_row = Some(row as usize);
        }
        if let Some(row) = event.table_activate() {
            self.open_editor(row as usize, root, ctx)?;
            return Ok(());
        }
        match event {
            ViewEvent::Cell { row, column, value } => {
                if column == 0 {
                    if let Some(m) = self.mappings.get_mut(row as usize) {
                        m.enabled = matches!(value.trim(), "是" | "true" | "1" | "✓");
                        save_mappings(root, &self.mappings)?;
                    }
                } else if column == 1 {
                    self.status = Some("代码模块由「检测模块」生成".into());
                } else {
                    self.status = Some("双击行打开编辑（分支/模块是下拉框）".into());
                }
            }
            ViewEvent::InsertRow(row) => {
                let at = (row as usize).min(self.mappings.len());
                self.mappings.insert(at, Mapping { enabled: true, ..Default::default() });
                save_mappings(root, &self.mappings)?;
                self.open_editor(at, root, ctx)?;
            }
            ViewEvent::DeleteRows(rows) => {
                let mut idx: Vec<usize> = rows.into_iter().map(|r| r as usize).collect();
                idx.sort_unstable();
                idx.dedup();
                for i in idx.into_iter().rev() {
                    if i < self.mappings.len() {
                        self.mappings.remove(i);
                    }
                }
                save_mappings(root, &self.mappings)?;
                self.status = Some("已删除选中行".into());
            }
            ViewEvent::Button(id) => match id.as_str() {
                "detect" => self.detect(root, ctx)?,
                "add_row" => {
                    self.mappings.push(Mapping::default());
                    save_mappings(root, &self.mappings)?;
                    let row = self.mappings.len() - 1;
                    self.open_editor(row, root, ctx)?;
                }
                "edit_row" => {
                    match self.selected_row {
                        Some(row) if row < self.mappings.len() => self.open_editor(row, root, ctx)?,
                        _ => self.status = Some("先单击选中一行".into()),
                    }
                }
                "clear_all" => {
                    if host::confirm(
                        "清空映射表？",
                        &format!("将删除全部 {} 行，此操作不可撤销", self.mappings.len()),
                    )? {
                        self.mappings.clear();
                        save_mappings(root, &self.mappings)?;
                        self.selected_row = None;
                        self.status = Some("映射表已清空".into());
                    }
                }
                "delete_row" => match self.selected_row {
                    Some(row) if row < self.mappings.len() => {
                        self.mappings.remove(row);
                        save_mappings(root, &self.mappings)?;
                        self.selected_row = None;
                        self.status = Some("已删除选中行".into());
                    }
                    _ => self.status = Some("先单击选中一行".into()),
                },
                "preview" => self.preview(root, ctx)?,
                "from_config" => self.from_config(root, ctx)?,
                "refresh" => {
                    if let Some(pid) = self.mappings.iter().find_map(|m| {
                        (!m.project_id.is_empty()).then(|| m.project_id.clone())
                    }) {
                        self.refresh_lists(&pid);
                    } else {
                        self.status = Some("先在映射行里填 Apifox 项目 ID".into());
                    }
                }
                "sync" => {
                    if let Some(msg) = sync(ctx)? {
                        self.status = Some(msg);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn handle_editor(
        &mut self,
        index: usize,
        event: ViewEvent,
        root: &str,
        ctx: &Invocation,
    ) -> Result<(), Error> {
        match event {
            ViewEvent::Field(key, value) => match key.as_str() {
                "e_code_module" => {
                    self.e_code_module = value;
                    return Ok(());
                }
                "e_project" => {
                    self.e_project = value.trim().to_string();
                    return Ok(());
                }
                "e_team" => {
                    self.e_team = value.clone();
                    self.e_project_pick = "（未选择）".into();
                    self.projects.clear();
                    if let Some(team) = self.teams.iter().find(|t| t.name == value) {
                        let id = team.id;
                        self.fetch_projects(id);
                    }
                }
                "e_project_pick" => {
                    self.e_project_pick = value.clone();
                    if let Some(project) = self.projects.iter().find(|p| p.name == value) {
                        self.e_project = project.id.to_string();
                        let id = project.id.to_string();
                        self.refresh_lists(&id);
                    }
                }
                "e_enabled" => {
                    self.e_enabled = value == "true";
                }
                "e_branch" => {
                    self.e_branch = value.clone();
                    if let Some(b) = self.branches.iter().find(|b| b.label() == value) {
                        let pid = self.e_project.clone();
                        self.load_modules(&pid, Some(b.id));
                        self.fetch_folders();
                    }
                }
                "e_module" => {
                    self.e_module = value;
                    self.fetch_folders();
                }
                "e_endpoint_folder" => self.e_endpoint_folder = value,
                "e_schema_folder" => self.e_schema_folder = value,
                "e_endpoint_mode" => self.e_endpoint_mode = value,
                "e_schema_mode" => self.e_schema_mode = value,
                "e_naming" => self.e_naming = naming_value(&value).to_string(),
                "e_update_folder" => self.e_update_folder = value == "true",
                _ => return Ok(()),
            },
            ViewEvent::Button(id) => match id.as_str() {
                "save" => {
                    if self.e_project.is_empty() || !self.e_project.chars().all(|c| c.is_ascii_digit()) {
                        self.status = Some("Apifox 项目 ID 必填且必须是数字".into());
                        return Ok(());
                    }
                    let code_path = self.code_path(&self.e_code_module);
                    let code_name = self
                        .detected
                        .iter()
                        .find(|c| c.path == code_path)
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    if let Some(m) = self.mappings.get_mut(index) {
                        m.enabled = self.e_enabled;
                        m.path = code_path;
                        m.name = code_name;
                        m.project_id = self.e_project.clone();
                        let branch = self.branches.iter().find(|b| b.label() == self.e_branch);
                        m.branch_id = branch.map(|b| b.id);
                        m.branch_name = branch.map(|b| b.name.clone()).unwrap_or_default();
                        let module = self.modules.iter().find(|m| m.name == self.e_module);
                        m.module_id = module.map(|m| m.id);
                        m.module_name = module.map(|m| m.name.clone()).unwrap_or_default();
                        m.endpoint_folder_id = folder_id_by_label(&self.endpoint_folders, &self.e_endpoint_folder);
                        m.schema_folder_id = folder_id_by_label(&self.schema_folders, &self.e_schema_folder);
                        m.endpoint_mode = mode_value(&self.e_endpoint_mode).to_string();
                        m.schema_mode = mode_value(&self.e_schema_mode).to_string();
                        m.naming = self.e_naming.clone();
                        m.update_folder = self.e_update_folder;
                    }
                    save_mappings(root, &self.mappings)?;
                    self.editing = None;
                    self.status = Some("映射已保存".into());
                }
                "reload_folders" => {
                    self.fetch_folders();
                    self.status = Some(format!(
                        "目录：接口 {} 个，模型 {} 个",
                        self.endpoint_folders.len(),
                        self.schema_folders.len()
                    ));
                }
                "reload_modules" => {
                    self.detect(root, ctx)?;
                }
                "reload_teams" => {
                    self.fetch_teams();
                    self.status = Some(format!("已加载 {} 个团队", self.teams.len()));
                }
                "reload" => {
                    if self.e_project.is_empty() {
                        self.status = Some("先填 Apifox 项目 ID".into());
                    } else {
                        let pid = self.e_project.clone();
                        self.refresh_lists(&pid);
                    }
                }
                "delete" => {
                    if index < self.mappings.len() {
                        self.mappings.remove(index);
                    }
                    save_mappings(root, &self.mappings)?;
                    self.editing = None;
                    self.status = Some("已删除该映射".into());
                }
                "cancel" => {
                    self.editing = None;
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn open_editor(&mut self, row: usize, root: &str, ctx: &Invocation) -> Result<(), Error> {
        if row >= self.mappings.len() {
            return Ok(());
        }
        self.editing = Some(row);
        let m = self.mappings[row].clone();
        self.e_enabled = m.enabled;
        self.e_project = m.project_id.clone();
        self.e_branch = m.branch_label();
        self.e_module = m.module_label();
        self.e_endpoint_folder.clear();
        self.e_schema_folder.clear();
        self.e_endpoint_mode = mode_label(&m.endpoint_mode).to_string();
        self.e_schema_mode = mode_label(&m.schema_mode).to_string();
        self.e_naming = m.naming.clone();
        self.e_update_folder = m.update_folder;
        self.e_code_module = self.code_label(&m.path);
        self.e_project_pick = m.project_id.clone();
        if self.detected.is_empty() {
            self.detected = self.code_modules(root, ctx);
        }
        if self.teams.is_empty() {
            self.fetch_teams();
        }
        self.branches.clear();
        self.modules.clear();
        if !self.e_project.is_empty() && host::secret_is_set(APIFOX_SECRET) {
            let pid = self.e_project.clone();
            self.refresh_lists(&pid);
        }
        self.fetch_folders();
        if let Some(id) = m.endpoint_folder_id {
            self.e_endpoint_folder = folder_label_by_id(&self.endpoint_folders, id);
        }
        if let Some(id) = m.schema_folder_id {
            self.e_schema_folder = folder_label_by_id(&self.schema_folders, id);
        }
        Ok(())
    }

    /// Compares the generated documents with the hashes stored by the last sync.
    fn preview(&mut self, root: &str, ctx: &Invocation) -> Result<(), Error> {
        let Some(project) = ctx.project() else {
            self.status = Some("先打开一个 Java 项目".into());
            return Ok(());
        };
        let rows: Vec<Mapping> = self
            .mappings
            .iter()
            .filter(|m| m.enabled && !m.project_id.is_empty())
            .cloned()
            .collect();
        if rows.is_empty() {
            self.status = Some("没有启用的映射行".into());
            return Ok(());
        }
        match docs_for(&project, root, &rows, &[]) {
            Ok(docs) => {
                let (mut changed, mut skipped) = (0, 0);
                for (row, doc) in rows.iter().zip(docs.iter()) {
                    if doc.endpoints.is_empty() {
                        skipped += 1;
                        continue;
                    }
                    let json = openapi::render(doc);
                    if stored_value(&hash_key(root, &row.path)) != fnv1a(&json) {
                        changed += 1;
                    }
                }
                let unchanged = rows.len() - changed - skipped;
                self.status = Some(format!(
                    "{changed} 个模块有变更，{unchanged} 个未变{}",
                    if skipped > 0 {
                        format!("，{skipped} 个无接口")
                    } else {
                        String::new()
                    }
                ));
            }
            Err(e) => self.status = Some(format!("预览失败：{e}")),
        }
        Ok(())
    }

    /// `.apifox-sync.json` in the project root seeds the mapping table.
    fn from_config(&mut self, root: &str, ctx: &Invocation) -> Result<(), Error> {
        let Some(project) = ctx.project() else {
            self.status = Some("先打开一个 Java 项目".into());
            return Ok(());
        };
        let trimmed = root.trim_end_matches(['\\', '/']);
        let candidates = [
            format!("{trimmed}/.apifox-sync.json"),
            format!("{trimmed}\\.apifox-sync.json"),
        ];
        let mut text = None;
        for path in &candidates {
            if let Ok(content) = project.read_file(path) {
                text = Some(content);
                break;
            }
        }
        let Some(text) = text else {
            self.status = Some("项目根目录没有 .apifox-sync.json".into());
            return Ok(());
        };
        let value: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                self.status = Some(format!(".apifox-sync.json 解析失败：{e}"));
                return Ok(());
            }
        };
        let items = value
            .get("mappings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if items.is_empty() {
            self.status = Some("配置里没有 mappings 数组".into());
            return Ok(());
        }
        self.mappings = items.iter().map(mapping_from).collect();
        save_mappings(root, &self.mappings)?;
        self.status = Some(format!("已从 .apifox-sync.json 导入 {} 行", self.mappings.len()));
        Ok(())
    }
    /// Refreshes the code-module list used by the editor dropdown. It never
    /// creates mapping rows: rows are added by the user with `+`.
    fn detect(&mut self, root: &str, ctx: &Invocation) -> Result<(), Error> {
        self.detected = self.code_modules(root, ctx);
        self.status = Some(if self.detected.is_empty() {
            "没检测到 pom.xml / settings.gradle".into()
        } else {
            format!(
                "检测到 {} 个源代码模块，可在「源代码模块」下拉里选择",
                self.detected.len()
            )
        });
        Ok(())
    }

    /// Build files -> code modules (also kept for the editor's module picker).
    /// Names only (no paths). Duplicated names get `(#n)` so a pick still maps
    /// to one module.
    fn module_labels(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (i, module) in self.detected.iter().enumerate() {
            let duplicated = self.detected.iter().filter(|m| m.name == module.name).count() > 1;
            let label = if module.name.is_empty() {
                module.path.clone()
            } else if duplicated {
                format!("{} (#{})", module.name, i + 1)
            } else {
                module.name.clone()
            };
            out.push((label, module.path.clone()));
        }
        out
    }

    fn code_label(&self, path: &str) -> String {
        if path.is_empty() {
            return "（项目根）".to_string();
        }
        self.module_labels()
            .into_iter()
            .find(|(_, p)| p == path)
            .map(|(label, _)| label)
            .unwrap_or_else(|| path.to_string())
    }

    fn code_path(&self, label: &str) -> String {
        if label == "（项目根）" {
            return String::new();
        }
        self.module_labels()
            .into_iter()
            .find(|(l, _)| l == label)
            .map(|(_, path)| path)
            .unwrap_or_else(|| label.to_string())
    }

    fn code_modules(&self, root: &str, ctx: &Invocation) -> Vec<code_project::CodeModule> {
        let Some(project) = ctx.project() else {
            return Vec::new();
        };
        let mut build_files: Vec<(String, String)> = Vec::new();
        for pattern in ["**/pom.xml", "**/settings.gradle", "**/settings.gradle.kts"] {
            for path in project.find_files(pattern, 200).unwrap_or_default() {
                let rel = relative_to_root(root, &path);
                if let Ok(content) = project.read_file(&path) {
                    build_files.push((rel, content));
                }
            }
        }
        code_project::detect_modules(&build_files)
    }

    fn fetch_folders(&mut self) {
        let project_id = self.e_project.clone();
        if project_id.is_empty() {
            return;
        }
        let branch = self
            .branches
            .iter()
            .find(|b| b.label() == self.e_branch || b.name == self.e_branch)
            .map(|b| b.id);
        let module = self
            .modules
            .iter()
            .find(|m| m.name == self.e_module)
            .map(|m| m.id);
        self.endpoint_folders = load_folders(&self.server, &project_id, module, branch, false);
        self.schema_folders = load_folders(&self.server, &project_id, module, branch, true);
    }
    fn fetch_teams(&mut self) {
        if !host::secret_is_set(APIFOX_SECRET) {
            return;
        }
        if let Ok((status, text)) = call_apifox(&apifox::teams_request(), &self.server) {
            if let Ok(list) = apifox::parse_named(status, &text) {
                self.teams = list;
            }
        }
    }

    fn fetch_projects(&mut self, team_id: u64) {
        if let Ok((status, text)) =
            call_apifox(&apifox::projects_request(&team_id.to_string()), &self.server)
        {
            if let Ok(list) = apifox::parse_named(status, &text) {
                self.projects = list;
            }
        }
    }
    fn refresh_lists(&mut self, project_id: &str) {
        if project_id.is_empty() {
            self.status = Some("先填 Apifox 项目 ID".into());
            return;
        }
        if !host::secret_is_set(APIFOX_SECRET) {
            self.status = Some("先在设置页保存访问令牌".into());
            return;
        }
        let branches = match call_apifox(&apifox::branches_request(project_id), &self.server) {
            Ok((status, text)) => match apifox::parse_branches(status, &text) {
                Ok(v) => v,
                Err(e) => {
                    self.status = Some(format!("分支列表读取失败：{e}"));
                    return;
                }
            },
            Err(e) => {
                self.status = Some(format!("分支列表请求失败：{e}"));
                return;
            }
        };
        self.branches = branches;
        let wanted = self.e_branch.clone();
        let found = self
            .branches
            .iter()
            .find(|b| b.label() == wanted || b.name == wanted)
            .map(|b| (b.id, b.label()));
        let branch_id = found.as_ref().map(|(id, _)| *id);
        if let Some((_, label)) = found {
            self.e_branch = label;
        }
        self.modules.clear();
        if !self.branches.is_empty() {
            self.load_modules(project_id, branch_id);
        }
        self.status = Some(if self.branches.is_empty() {
            "未读到分支（检查项目 ID/令牌权限）".to_string()
        } else {
            format!("已加载 {} 个分支、{} 个模块", self.branches.len(), self.modules.len())
        });
    }

    fn load_modules(&mut self, project_id: &str, branch_id: Option<u64>) {
        if project_id.is_empty() {
            return;
        }
        if let Ok((status, text)) = call_apifox(&apifox::modules_request(project_id, branch_id), &self.server) {
            if let Ok(list) = apifox::parse_modules(status, &text) {
                self.modules = list;
            }
        }
    }
}

fn sync(ctx: &Invocation) -> Result<Option<String>, Error> {
    let project = ctx.project().ok_or("请先打开一个 Java 项目再运行该命令")?;
    let root = project.root().unwrap_or_default();
    let mappings = load_mappings(&root);
    let mut rows: Vec<Mapping> = mappings
        .into_iter()
        .filter(|m| m.enabled && !m.project_id.is_empty())
        .collect();
    // Right-clicking a controller/folder limits the sync to its module and,
    // below, to the selected files.
    let selected: Vec<String> = ctx
        .selected_paths()
        .iter()
        .map(|p| relative_to_root(&root, p))
        .filter(|p| !p.is_empty())
        .collect();
    if !selected.is_empty() {
        let matched: Vec<Mapping> = rows
            .iter()
            .filter(|m| !m.path.is_empty() && row_matches(&m.path, &selected))
            .cloned()
            .collect();
        if !matched.is_empty() {
            rows = matched;
        }
    }
    if rows.is_empty() {
        return Err("映射表没有可同步的行：填 Apifox 项目 ID 并勾选启用".into());
    }
    if !ensure_token()? {
        return Ok(None);
    }

    let docs = docs_for(&project, &root, &rows, &selected)?;
    // `server.servlet.context-path` from the nearest application.yml/properties.
    let base = server_base(&root);

    let task = host::begin_task("扫描模块并同步到 Apifox", false);
    let mut created = 0u64;
    let mut updated = 0u64;
    let mut failed = 0u64;
    let mut notes: Vec<String> = Vec::new();
    let mut done = 0usize;

    for (row, doc) in rows.iter().zip(docs.into_iter()) {
        if task.is_cancelled() {
            return Err(Error::Cancelled);
        }
        done += 1;
        task.report(done as u32, Some(rows.len() as u32), Some(row.name.as_str()));



        if doc.endpoints.is_empty() {
            notes.push(format!("{}：没有接口，跳过", row.name));
            continue;
        }
        let json = openapi::render(&doc);
        let opts = apifox::ImportOptions {
            branch_id: row.branch_id,
            module_id: row.module_id,
            endpoint_folder_id: row.endpoint_folder_id,
            schema_folder_id: row.schema_folder_id,
            endpoint_mode: row.endpoint_mode.clone(),
            schema_mode: row.schema_mode.clone(),
            update_folder: row.update_folder,
        };
        let request = apifox::import_request(&row.project_id, &json, &opts);
        let url = rewrite_server(&request.url, &base);
        let mut req = HttpRequest::post(&url, request.body.clone());
        for (name, value) in &request.headers {
            req = req.header(name.as_str(), value.as_str());
        }
        req.timeout_ms = Some(60_000);
        match host::fetch(&req) {
            Ok(response) => match apifox::parse_response(response.status, &response.text()) {
                Ok(out) => {
                    let _ = host::storage::set(&hash_key(&root, &row.path), &fnv1a(&json));
                    created += out.created;
                    updated += out.updated;
                    failed += out.failed;
                    if !out.errors.is_empty() {
                        notes.push(format!("{}：{}", row.name, out.errors.join("；")));
                    }
                }
                Err(e) => {
                    failed += 1;
                    notes.push(format!("{}：{e}", row.name));
                }
            },
            Err(e) => {
                failed += 1;
                notes.push(format!("{}：{e}", row.name));
            }
        }
    }

    let mut message = format!(
        "同步 {} 个模块：新建 {}，更新 {}，失败 {}",
        rows.len(),
        created,
        updated,
        failed
    );
    if !notes.is_empty() {
        message.push_str("；");
        message.push_str(&notes.join("；"));
        host::notify(NotificationLevel::Warning, TITLE, &message);
    } else {
        host::notify(NotificationLevel::Info, TITLE, &message);
    }
    Ok(Some(message))
}

fn project_document(ctx: &Invocation) -> Result<(Project, apifox_core::model::ApiDoc), Error> {
    let project = ctx.project().ok_or("请先打开一个 Java 项目再运行该命令")?;
    let root = project.root().unwrap_or_default();
    let files = project.find_files("**/*.java", MAX_FILES)?;
    let mut project_files: Vec<(String, String)> = Vec::new();
    for path in &files {
        let rel = relative_to_root(&root, path);
        if let Ok(text) = project.read_file(path) {
            project_files.push((rel, text));
        }
    }
    let doc = scan_project_files(&project_files);
    if doc.endpoints.is_empty() {
        return Err("未找到 Spring Controller".into());
    }
    Ok((project, doc))
}

fn export(ctx: &Invocation) -> Result<Option<String>, Error> {
    let (project, doc) = project_document(ctx)?;
    let json = openapi::render(&doc);
    project.write_file(OPENAPI_PATH, &json)?;
    let message = format!("已导出 {} 个接口到 {OPENAPI_PATH}", doc.endpoints.len());
    host::notify(NotificationLevel::Info, TITLE, &message);
    Ok(Some(message))
}

fn export_yaml(ctx: &Invocation) -> Result<Option<String>, Error> {
    let (project, doc) = project_document(ctx)?;
    let json: serde_json::Value = serde_json::from_str(&openapi::render(&doc))
        .map_err(|e| format!("生成 OpenAPI 失败：{e}"))?;
    let text = yaml::to_yaml(&json);
    project.write_file("docs/openapi.yaml", &text)?;
    let message = format!("已导出 {} 个接口到 docs/openapi.yaml", doc.endpoints.len());
    host::notify(NotificationLevel::Info, TITLE, &message);
    Ok(Some(message))
}

fn export_postman(ctx: &Invocation) -> Result<Option<String>, Error> {
    let (project, doc) = project_document(ctx)?;
    let text = postman::collection(&doc, "API 文档");
    project.write_file("docs/postman_collection.json", &text)?;
    let message = format!(
        "已导出 {} 个接口到 docs/postman_collection.json",
        doc.endpoints.len()
    );
    host::notify(NotificationLevel::Info, TITLE, &message);
    Ok(Some(message))
}

fn configure(ctx: &Invocation) -> Result<Option<String>, Error> {
    let project = ctx.project().ok_or("请先打开一个 Java 项目再运行该命令")?;
    let _ = project;
    if host::secret_is_set(APIFOX_SECRET) {
        if host::confirm("重新设置 Apifox 访问令牌？", "选「是」可替换或删除已保存的令牌")? {
            if let Some(raw) = host::input("输入新的访问令牌（留空 = 删除）", None)? {
                let raw = raw.trim();
                if raw.is_empty() {
                    host::secret_delete(APIFOX_SECRET)?;
                } else {
                    let token = raw.strip_prefix("Bearer ").unwrap_or(raw).trim();
                    host::secret_set(APIFOX_SECRET, token)?;
                }
            }
        }
    } else if let Some(raw) = host::input("输入 Apifox 访问令牌（留空跳过）", None)? {
        let raw = raw.trim();
        if !raw.is_empty() {
            let token = raw.strip_prefix("Bearer ").unwrap_or(raw).trim();
            host::secret_set(APIFOX_SECRET, token)?;
        }
    }
    let message = "令牌设置已更新；项目/分支/模块请在「映射」标签按代码模块配置";
    host::notify(NotificationLevel::Info, TITLE, message);
    Ok(Some(message.to_string()))
}

fn ensure_token() -> Result<bool, Error> {
    if host::secret_is_set(APIFOX_SECRET) {
        return Ok(true);
    }
    let Some(raw) = host::input(
        "首次同步需要 Apifox 访问令牌（Apifox → 账号设置 → API 访问令牌）",
        None,
    )?
    else {
        return Ok(false);
    };
    let raw = raw.trim();
    let token = raw.strip_prefix("Bearer ").unwrap_or(raw).trim();
    if token.is_empty() {
        return Ok(false);
    }
    host::secret_set(APIFOX_SECRET, token)?;
    Ok(true)
}



fn call_apifox(call: &apifox::ApiCall, base: &str) -> Result<(u16, String), Error> {
    let url = rewrite_server(&call.url, base);
    let mut req = if call.method == "GET" {
        HttpRequest::get(&url)
    } else {
        HttpRequest::post(&url, call.body.clone().unwrap_or_default())
    };
    for (name, value) in &call.headers {
        req = req.header(name.as_str(), value.as_str());
    }
    req.timeout_ms = Some(30_000);
    let response = host::fetch(&req)?;
    Ok((response.status, response.text()))
}

fn relative_to_root(root: &str, path: &str) -> String {
    let root = root.replace('\\', "/");
    let path = path.replace('\\', "/");
    let trimmed = root.trim_end_matches('/');
    let rel = if !trimmed.is_empty() {
        path.strip_prefix(trimmed).unwrap_or(&path)
    } else {
        &path
    };
    rel.trim_start_matches('/').to_string()
}

/* ---------- storage ---------- */

fn server_key(root: &str) -> String {
    format!("apifox.server:{root}")
}

fn server_base(root: &str) -> String {
    let saved = stored_value(&server_key(root));
    if saved.is_empty() {
        DEFAULT_SERVER.to_string()
    } else {
        saved
    }
}

fn rewrite_server(url: &str, base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.is_empty() || base == DEFAULT_SERVER {
        url.to_string()
    } else {
        url.replacen(DEFAULT_SERVER, base, 1)
    }
}
fn mappings_key(root: &str) -> String {
    format!("apifox.mappings:{root}")
}

fn load_mappings(root: &str) -> Vec<Mapping> {
    if let Some(raw) = host::storage::get(&mappings_key(root)).ok().flatten() {
        if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&raw) {
            return items.iter().map(mapping_from).collect();
        }
    }
    migrate_legacy(root)
}

fn mapping_from(v: &Value) -> Mapping {
    Mapping {
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        name: text(v, "name"),
        path: text(v, "path"),
        project_id: text(v, "projectId"),
        branch_id: v.get("branchId").and_then(Value::as_u64),
        branch_name: text(v, "branchName"),
        module_id: v.get("moduleId").and_then(Value::as_u64),
        module_name: text(v, "moduleName"),
        endpoint_folder_id: v.get("endpointFolderId").and_then(Value::as_u64),
        schema_folder_id: v.get("schemaFolderId").and_then(Value::as_u64),
        endpoint_mode: mode_or_default(text(v, "endpointMode")),
        schema_mode: mode_or_default(text(v, "schemaMode")),
        naming: text(v, "naming"),
        update_folder: v.get("updateFolder").and_then(Value::as_bool).unwrap_or(false),
    }
}

const NAMINGS: [(&str, &str); 7] = [
    ("保持原样", "original"),
    ("小驼峰", "camel"),
    ("大驼峰", "pascal"),
    ("下划线", "snake"),
    ("短横线", "kebab"),
    ("全小写", "flat"),
    ("全大写", "upper"),
];

fn naming_label(value: &str) -> &'static str {
    NAMINGS
        .iter()
        .find(|(_, v)| *v == value)
        .map(|(l, _)| *l)
        .unwrap_or("保持原样")
}

fn naming_value(label: &str) -> &'static str {
    NAMINGS
        .iter()
        .find(|(l, _)| *l == label)
        .map(|(_, v)| *v)
        .unwrap_or("original")
}

const DEFAULT_SERVER: &str = "https://api.apifox.com";
const MODES: [(&str, &str); 4] = [
    ("智能合并", "AUTO_MERGE"),
    ("覆盖所有字段", "OVERWRITE_EXISTING"),
    ("不导入", "KEEP_EXISTING"),
    ("保留两者", "CREATE_NEW"),
];

fn load_folders(
    server: &str,
    project_id: &str,
    module: Option<u64>,
    branch: Option<u64>,
    schema: bool,
) -> Vec<apifox::Folder> {
    match call_apifox(&apifox::folders_request(project_id, module, branch, schema), server) {
        Ok((status, text)) => apifox::parse_folders(status, &text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn folder_label_by_id(list: &[apifox::Folder], id: u64) -> String {
    list.iter()
        .find(|f| f.id == id)
        .map(|f| f.path.clone())
        .unwrap_or_else(|| format!("ID {id}"))
}

fn folder_id_by_label(list: &[apifox::Folder], label: &str) -> Option<u64> {
    let label = label.trim();
    if label.is_empty() || label == "根目录" {
        return None;
    }
    if let Some(rest) = label.strip_prefix("ID ") {
        return rest.trim().parse().ok();
    }
    list.iter().find(|f| f.path == label).map(|f| f.id)
}

fn folder_options(
    list: &[apifox::Folder],
    current: Option<u64>,
    preferred: &str,
) -> (Vec<String>, String) {
    let mut options = vec!["根目录".to_string()];
    options.extend(list.iter().map(|f| f.path.clone()));
    if !preferred.is_empty() && options.iter().any(|o| o == preferred) {
        return (options, preferred.to_string());
    }
    match current {
        None => (options, "根目录".to_string()),
        Some(id) => {
            let label = folder_label_by_id(list, id);
            if !options.iter().any(|o| o == &label) {
                options.push(label.clone());
            }
            (options, label)
        }
    }
}
fn folder_label(id: Option<u64>) -> String {
    id.map(|v| v.to_string()).unwrap_or_else(|| "根目录".into())
}


fn mode_or_default(value: String) -> String {
    if MODES.iter().any(|(_, m)| *m == value) {
        value
    } else {
        "AUTO_MERGE".into()
    }
}

fn mode_label(value: &str) -> &'static str {
    MODES
        .iter()
        .find(|(_, m)| *m == value)
        .map(|(l, _)| *l)
        .unwrap_or("智能合并")
}

fn mode_value(label: &str) -> &'static str {
    MODES
        .iter()
        .find(|(l, _)| *l == label)
        .map(|(_, m)| *m)
        .unwrap_or("AUTO_MERGE")
}
fn stored_value(key: &str) -> String {
    host::storage::get(key)
        .ok()
        .flatten()
        .unwrap_or_default()
        .trim()
        .to_string()
}
fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn save_mappings(root: &str, list: &[Mapping]) -> Result<(), Error> {
    let items: Vec<Value> = list
        .iter()
        .map(|m| {
            json!({
                "enabled": m.enabled,
                "name": m.name,
                "path": m.path,
                "projectId": m.project_id,
                "branchId": m.branch_id,
                "branchName": m.branch_name,
                "moduleId": m.module_id,
                "moduleName": m.module_name,
                "endpointFolderId": m.endpoint_folder_id,
                "schemaFolderId": m.schema_folder_id,
                "endpointMode": m.endpoint_mode,
                "schemaMode": m.schema_mode,
                "naming": m.naming,
                "updateFolder": m.update_folder,
            })
        })
        .collect();
    host::storage::set(&mappings_key(root), &Value::Array(items).to_string())
}

/// One row from the 0.3.x single-project settings, so upgrades do not lose it.
fn migrate_legacy(root: &str) -> Vec<Mapping> {
    let project = host::storage::get(&format!("apifox.project_id:{root}"))
        .ok()
        .flatten()
        .unwrap_or_default();
    let branch = host::storage::get(&format!("apifox.branch:{root}"))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let module = host::storage::get(&format!("apifox.module:{root}:0"))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    if project.trim().is_empty() {
        return Vec::new();
    }
    vec![Mapping {
        enabled: true,
        name: root.rsplit(['\\', '/']).find(|s| !s.is_empty()).unwrap_or("项目").to_string(),
        path: String::new(),
        project_id: project.trim().to_string(),
        branch_id: branch.as_ref().and_then(|v| v.get("id")).and_then(Value::as_u64),
        branch_name: branch
            .as_ref()
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        module_id: module.as_ref().and_then(|v| v.get("id")).and_then(Value::as_u64),
        module_name: module
            .as_ref()
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        ..Mapping::default()
    }]
}

/// Documents for `rows`: endpoints per module, types from the whole project,
/// with context-path and naming applied. Shared by sync and preview.
fn docs_for(
    project: &Project,
    root: &str,
    rows: &[Mapping],
    selected: &[String],
) -> Result<Vec<apifox_core::model::ApiDoc>, Error> {
    let files = project.find_files("**/*.java", MAX_FILES)?;
    let mut project_files: Vec<(String, String)> = Vec::new();
    for path in &files {
        let rel = relative_to_root(root, path);
        if let Ok(text) = project.read_file(path) {
            project_files.push((rel, text));
        }
    }
    let paths: Vec<String> = rows.iter().map(|r| r.path.clone()).collect();
    let mut docs = apifox_core::scan_modules_selected(&project_files, &paths, selected);
    let contexts = context_paths(project, root);
    for (row, doc) in rows.iter().zip(docs.iter_mut()) {
        if let Some(prefix) = context_for(&row.path, &contexts) {
            prefix_paths(doc, &prefix);
        }
        apifox_core::naming::apply(doc, apifox_core::naming::Naming::parse(&row.naming));
    }
    Ok(docs)
}

fn row_matches(module: &str, selected: &[String]) -> bool {
    let module = module.trim_end_matches('/');
    selected.iter().any(|path| {
        let path = path.trim_end_matches('/');
        path == module || path.starts_with(&format!("{module}/"))
    })
}
fn hash_key(root: &str, module: &str) -> String {
    format!("apifox.hash:{root}:{module}")
}

fn fnv1a(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}
/// `(directory, context-path)` pairs from Spring Boot config files.
fn context_paths(project: &Project, root: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for pattern in [
        "**/application.properties",
        "**/application.yml",
        "**/application.yaml",
        "**/bootstrap.properties",
        "**/bootstrap.yml",
        "**/bootstrap.yaml",
    ] {
        for path in project.find_files(pattern, 200).unwrap_or_default() {
            let Ok(text) = project.read_file(&path) else { continue };
            let Some(prefix) = read_context(&text) else { continue };
            let rel = relative_to_root(root, &path);
            let dir = rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
            if !out.iter().any(|(d, _)| d == &dir) {
                out.push((dir, prefix));
            }
        }
    }
    out
}

fn read_context(text: &str) -> Option<String> {
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with("//") {
            continue;
        }
        for key in ["server.servlet.context-path", "server.context-path"] {
            if let Some(rest) = t.strip_prefix(key) {
                if let Some(value) = rest.trim_start().strip_prefix('=') {
                    let value = clean_value(value);
                    if !value.is_empty() {
                        return Some(value);
                    }
                }
            }
        }
        if let Some(rest) = t.strip_prefix("context-path:") {
            let value = clean_value(rest);
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn clean_value(value: &str) -> String {
    value.trim().trim_matches('"').trim_matches('\'').to_string()
}

/// The config whose directory shares the most path segments with the module.
fn context_for(module: &str, contexts: &[(String, String)]) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for (dir, prefix) in contexts {
        let score = dir
            .split('/')
            .filter(|s| !s.is_empty())
            .zip(module.split('/').filter(|s| !s.is_empty()))
            .take_while(|(a, b)| a == b)
            .count();
        if best.map(|(s, _)| score > s).unwrap_or(true) {
            best = Some((score, prefix));
        }
    }
    best.map(|(_, prefix)| prefix.to_string())
}

register_plugin!(Main, views);