//! End-to-end check of the pure core: Java sources in, OpenAPI 3.0.3 + Apifox
//! request out. This is the layer the IDT cannot see (no PSI), so it is where
//! the real risk lives.

use apifox_core::{apifox, model::Ty, openapi, scan_sources};

fn sources() -> Vec<String> {
    [
        include_str!("fixtures/UserController.java"),
        include_str!("fixtures/UserDto.java"),
        include_str!("fixtures/Address.java"),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[test]
fn scans_spring_controller_into_endpoints() {
    let doc = scan_sources(&sources());
    let routes: Vec<(&str, &str)> = doc
        .endpoints
        .iter()
        .map(|e| (e.method.as_str(), e.path.as_str()))
        .collect();
    assert_eq!(
        routes,
        vec![
            ("POST", "/api/users"),
            ("POST", "/api/users/avatar"),
            ("GET", "/api/users/page"),
            ("DELETE", "/api/users/{id}"),
            ("GET", "/api/users/{id}"),
            ("PUT", "/api/users/{id}"),
        ]
    );
    assert!(doc.endpoints.iter().all(|e| e.tag == "UserController"));
}

#[test]
fn carries_javadoc_and_annotation_defaults() {
    let doc = scan_sources(&sources());
    let page = doc
        .endpoints
        .iter()
        .find(|e| e.path == "/api/users/page")
        .unwrap();
    assert_eq!(page.summary, "分页查询用户");
    let keyword = page.params.iter().find(|p| p.name == "keyword").unwrap();
    assert!(keyword.required);
    assert_eq!(keyword.ty, Ty::String);
    assert_eq!(keyword.description, "搜索关键字");

    let page_no = page.params.iter().find(|p| p.name == "page").unwrap();
    assert_eq!(page_no.default.as_deref(), Some("1"));
    assert!(!page_no.required, "a @RequestParam with defaultValue is optional");

    let tenant = page.params.iter().find(|p| p.name == "X-Tenant").unwrap();
    assert!(!tenant.required);
    assert_eq!(tenant.location.as_str(), "header");
}

#[test]
fn request_body_and_response_are_refs_to_dtos() {
    let doc = scan_sources(&sources());
    let create = doc
        .endpoints
        .iter()
        .find(|e| e.method == "POST" && e.path == "/api/users")
        .unwrap();
    assert_eq!(
        create.body.as_ref().map(|(mime, ty)| (mime.as_str(), ty.clone())),
        Some(("application/json", Ty::Ref("UserDto".into())))
    );
    assert_eq!(create.responses[0].code, "200");
    assert_eq!(create.responses[0].ty, Some(Ty::Ref("UserDto".into())));

    let page = doc.endpoints.iter().find(|e| e.path == "/api/users/page").unwrap();
    match page.responses[0].ty.as_ref().unwrap() {
        Ty::Object(fields) => {
            let records = fields.iter().find(|f| f.name == "records").unwrap();
            assert_eq!(records.ty, Ty::Array(Box::new(Ty::Ref("UserDto".into()))));
        }
        other => panic!("PageResult should become a page object, got {other:?}"),
    }
}

#[test]
fn builds_component_schemas_transitively() {
    let doc = scan_sources(&sources());
    let user = doc.schemas.get("UserDto").expect("UserDto schema");
    match user {
        Ty::Object(fields) => {
            let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, vec!["username", "age", "roles", "address"]);
            assert!(fields[0].required, "@NotNull marks username required");
            assert_eq!(fields[0].description, "用户名");
            assert_eq!(fields[2].ty, Ty::Array(Box::new(Ty::String)));
            assert_eq!(fields[3].ty, Ty::Ref("Address".into()));
        }
        other => panic!("unexpected UserDto {other:?}"),
    }
    assert!(doc.schemas.contains_key("Address"), "nested DTO must be collected");
}

#[test]
fn marks_deprecated_and_multipart() {
    let doc = scan_sources(&sources());
    let update = doc
        .endpoints
        .iter()
        .find(|e| e.method == "PUT")
        .unwrap();
    assert!(update.deprecated, "Javadoc @deprecated / @Deprecated :: deprecated=true");

    let upload = doc.endpoints.iter().find(|e| e.path.ends_with("/avatar")).unwrap();
    assert_eq!(upload.body.as_ref().map(|(m, _)| m.as_str()), Some("multipart/form-data"));
}

#[test]
fn renders_valid_openapi_3() {
    let doc = scan_sources(&sources());
    let json = openapi::render(&doc);
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(v["openapi"], "3.0.3");
    assert_eq!(v["info"]["title"], "API 文档");
    assert!(v["paths"]["/api/users/page"]["get"].is_object());
    assert_eq!(
        v["paths"]["/api/users"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/UserDto"
    );
    assert_eq!(
        v["paths"]["/api/users/page"]["get"]["parameters"][1]["schema"]["default"],
        1
    );
    assert_eq!(v["components"]["schemas"]["UserDto"]["properties"]["age"]["format"], "int32");
    assert_eq!(v["paths"]["/api/users"]["post"]["operationId"], "userController_create");
}

#[test]
fn apifox_request_wraps_the_document() {
    let doc = scan_sources(&sources());
    let json = openapi::render(&doc);
let req = apifox::import_request("42", &json, &apifox::ImportOptions::default());
    assert!(req.url.ends_with("/v1/projects/42/import-openapi"));
    let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["options"]["endpointOverwriteBehavior"], "AUTO_MERGE");
    assert_eq!(body["input"], json);
}
#[test]
fn partial_export_drops_unreachable_schemas() {
    let doc = scan_sources(&sources());
    let delete = doc
        .endpoints
        .iter()
        .position(|e| e.method == "DELETE")
        .expect("delete endpoint");
    let only = apifox_core::select_endpoints(&doc, &[delete]);
    assert_eq!(only.endpoints.len(), 1);
    assert!(only.schemas.is_empty(), "void response reaches no DTO");

    let detail = doc
        .endpoints
        .iter()
        .position(|e| e.method == "GET" && e.path == "/api/users/{id}")
        .expect("detail endpoint");
    let only = apifox_core::select_endpoints(&doc, &[detail]);
    assert!(only.schemas.contains_key("UserDto"));
    assert!(only.schemas.contains_key("Address"));
}

#[test]
fn module_scan_resolves_types_from_other_modules() {
    let controller = r#"
package a;
@RestController
@RequestMapping("/x")
public class C {
    @GetMapping("/list")
    public List<ExternalDto> list() { return null; }
}
"#;
    let dto = r#"
package b;
/** 外部 DTO */
public class ExternalDto {
    /** 名字 */
    private String name;
}
"#;
    let files = vec![
        ("erp/C.java".to_string(), controller.to_string()),
        ("facade/ExternalDto.java".to_string(), dto.to_string()),
    ];
    let docs = apifox_core::scan_modules(&files, &["erp".to_string()]);
    let doc = &docs[0];
    assert_eq!(doc.endpoints.len(), 1);
    match doc.endpoints[0].responses[0].ty.as_ref().expect("response") {
        Ty::Array(inner) => assert_eq!(**inner, Ty::Ref("ExternalDto".into())),
        other => panic!("unexpected response {other:?}"),
    }
    assert!(doc.schemas.contains_key("ExternalDto"));
    assert!(apifox_core::in_module("erp/C.java", "erp"));
    assert!(!apifox_core::in_module("facade/ExternalDto.java", "erp"));
}

#[test]
fn serialization_names_constraints_and_enums_reach_the_schema() {
    let controller = r#"
package a;
@RestController
@RequestMapping("/x")
public class C {
    @PostMapping("/save")
    public ResponseEntity<Payload> save(@RequestBody Payload body) { return null; }
}
"#;
    let payload = r#"
package a;
public class Payload {
    @JsonProperty("user_name")
    @Size(min = 2, max = 8)
    private String userName;
    @Min(1) @Max(9)
    private Integer level;
    @Email
    private String email;
@Pattern(regexp = "^\\d+$")
    private String code;
    @JSONField(name = "nick_name")
    private String nick;
    private Status status;
}
enum Status { ACTIVE, INACTIVE("off"); }
"#;
    let doc = apifox_core::scan_sources(&[controller.to_string(), payload.to_string()]);
    let schema = doc.schemas.get("Payload").expect("Payload schema");
    let Ty::Object(fields) = schema else { panic!("{schema:?}") };
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
assert!(names.contains(&"user_name"), "renamed property: {names:?}");
    assert!(!names.contains(&"userName"));
    assert!(
        names.contains(&"nick_name"),
        "@JSONField(name=…) renames instead of hiding: {names:?}"
    );

    let user = fields.iter().find(|f| f.name == "user_name").unwrap();
    assert_eq!(user.min_len, Some(2));
    assert_eq!(user.max_len, Some(8));

    let level = fields.iter().find(|f| f.name == "level").unwrap();
    assert_eq!(level.min, Some(1.0));
    assert_eq!(level.max, Some(9.0));

    let email = fields.iter().find(|f| f.name == "email").unwrap();
    assert_eq!(email.format.as_deref(), Some("email"));

    let code = fields.iter().find(|f| f.name == "code").unwrap();
    assert_eq!(code.pattern.as_deref(), Some("^\\d+$"));

    assert_eq!(
        doc.schemas.get("Status"),
        Some(&Ty::Enum(vec!["ACTIVE".into(), "INACTIVE".into()]))
    );

    let json = openapi::render(&doc);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let props = &v["components"]["schemas"]["Payload"]["properties"];
    assert_eq!(props["user_name"]["minLength"], 2);
    assert_eq!(props["level"]["minimum"], 1.0);
    assert_eq!(props["email"]["format"], "email");
    assert_eq!(v["components"]["schemas"]["Status"]["enum"][0], "ACTIVE");
}

#[test]
fn context_path_is_prefixed_once() {
    let mut doc = apifox_core::scan_sources(&sources());
    apifox_core::prefix_paths(&mut doc, "/api");
    assert!(doc.endpoints.iter().all(|e| e.path.starts_with("/api/")));
    apifox_core::prefix_paths(&mut doc, "api/");
    assert!(doc.endpoints.iter().all(|e| e.path.starts_with("/api/")));
    assert!(!doc.endpoints.iter().any(|e| e.path.starts_with("/api/api")));
}

#[test]
fn jaxrs_paths_params_and_verbs() {
    let src = r#"
package a;
@Path("/api")
public class J {
    /** 查询 */
    @GET
    @Path("/users/{id}")
    public UserDto get(@PathParam("id") Long id,
                       @QueryParam("q") String q,
                       @HeaderParam("X-Tenant") String tenant) { return null; }
}
class UserDto { private String name; }
"#;
    let doc = apifox_core::scan_sources(&[src.to_string()]);
    assert_eq!(doc.endpoints.len(), 1);
    let ep = &doc.endpoints[0];
    assert_eq!(ep.method, "GET");
    assert_eq!(ep.path, "/api/users/{id}");
    let by = |name: &str| ep.params.iter().find(|p| p.name == name).unwrap();
    assert_eq!(by("id").location.as_str(), "path");
    assert_eq!(by("q").location.as_str(), "query");
    assert_eq!(by("X-Tenant").location.as_str(), "header");
}

#[test]
fn feign_interfaces_become_endpoints() {
    let src = r#"
package a;
@FeignClient(name = "svc", path = "/remote")
public interface RemoteApi {
    @GetMapping("/list")
    List<UserDto> list(@RequestParam String q);
}
class UserDto { private String name; }
"#;
    let doc = apifox_core::scan_sources(&[src.to_string()]);
    assert_eq!(doc.endpoints.len(), 1);
    assert_eq!(doc.endpoints[0].path, "/remote/list");
    assert_eq!(doc.endpoints[0].method, "GET");
}

#[test]
fn swagger_annotations_override_summary_and_params() {
    let src = r#"
package a;
@RestController
public class C {
    @Operation(summary = "Swagger 摘要", description = "Swagger 描述", deprecated = true)
    @ApiResponse(description = "OK 描述")
    @GetMapping("/x")
    public String x(@Parameter(description = "用户 ID") @RequestParam String userId,
                    @Parameter(hidden = true) @RequestParam String secret) { return null; }
}
"#;
    let doc = apifox_core::scan_sources(&[src.to_string()]);
    let ep = &doc.endpoints[0];
    assert_eq!(ep.summary, "Swagger 摘要");
    assert_eq!(ep.description, "Swagger 描述");
    assert!(ep.deprecated);
    assert_eq!(ep.responses[0].description, "OK 描述");
    assert_eq!(ep.params.len(), 1, "hidden parameter dropped");
    assert_eq!(ep.params[0].description, "用户 ID");
}

#[test]
fn ignore_and_module_tags_control_export() {
    let src = r#"
package a;
/**
 * 控制器
 * @module 订单
 */
@RestController
public class C {
    /** 保留 */
    @GetMapping("/keep")
    public String keep() { return null; }

    /**
     * 不导出
     * @ignore
     */
    @GetMapping("/skip")
    public String skip() { return null; }
}
"#;
    let doc = apifox_core::scan_sources(&[src.to_string()]);
    assert_eq!(doc.endpoints.len(), 1);
    assert_eq!(doc.endpoints[0].path, "/keep");
    assert_eq!(doc.endpoints[0].tag, "订单");
}

#[test]
fn naming_conversion_renames_fields_and_params() {
    let mut doc = apifox_core::scan_sources(&sources());
    apifox_core::naming::apply(&mut doc, apifox_core::naming::Naming::Snake);
    match doc.schemas.get("UserDto") {
        Some(Ty::Object(fields)) => {
            let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            assert!(names.contains(&"username"));
            assert!(names.contains(&"roles"));
        }
        other => panic!("{other:?}"),
    }
    assert!(doc
        .endpoints
        .iter()
        .flat_map(|e| e.params.iter())
        .all(|p| !p.name.chars().any(|c| c.is_ascii_uppercase())));
}

#[test]
fn examples_only_come_from_annotations() {
    let src = r#"
package a;
@RestController
public class C {
    @GetMapping("/u") public U u() { return null; }
}
class U {
    @Schema(example = "alice")
    private String name;
    private long age;
}
"#;
    let doc = apifox_core::scan_sources(&[src.to_string()]);
    let json = openapi::render(&doc);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["components"]["schemas"]["U"]["properties"]["name"]["example"], "alice");
    assert!(v["components"]["schemas"]["U"]["properties"]["age"].get("example").is_none());
}

#[test]
fn selection_limits_endpoints_to_the_chosen_files() {
    let a = r#"
package a;
@RestController
@RequestMapping("/a")
public class A {
    @GetMapping("/x") public String x() { return null; }
}
"#;
    let b = r#"
package a;
@RestController
@RequestMapping("/b")
public class B {
    @GetMapping("/y") public String y() { return null; }
}
"#;
    let files = vec![
        ("erp/A.java".to_string(), a.to_string()),
        ("erp/B.java".to_string(), b.to_string()),
    ];
    let all = apifox_core::scan_modules(&files, &["erp".to_string()]);
    assert_eq!(all[0].endpoints.len(), 2);

    let only = apifox_core::scan_modules_selected(
        &files,
        &["erp".to_string()],
        &["erp/A.java".to_string()],
    );
    assert_eq!(only[0].endpoints.len(), 1);
    assert_eq!(only[0].endpoints[0].path, "/a/x");

    let folder = apifox_core::scan_modules_selected(
        &files,
        &["erp".to_string()],
        &["erp".to_string()],
    );
    assert_eq!(folder[0].endpoints.len(), 2);
}

#[test]
fn tag_name_prefers_module_then_swagger_tag_then_class_name() {
    let src = r#"
package a;
@Tag(name = "应用管理")
@RestController
public class Tagged {
    @GetMapping("/a") public String a() { return null; }
}

@Api(tags = {"旧版标签"})
@RestController
public class Legacy {
    @GetMapping("/b") public String b() { return null; }
}

/**
 * @module 覆盖标签
 */
@Tag(name = "应用管理")
@RestController
public class ModuleWins {
    @GetMapping("/c") public String c() { return null; }
}
"#;
    let doc = apifox_core::scan_sources(&[src.to_string()]);
    let tag = |path: &str| {
        doc.endpoints
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.tag.clone())
            .unwrap()
    };
    assert_eq!(tag("/a"), "应用管理");
    assert_eq!(tag("/b"), "旧版标签");
    assert_eq!(tag("/c"), "覆盖标签");
}

#[test]
fn fields_with_initializers_are_kept() {
    let src = r#"
package a;
@RestController
public class C {
    @GetMapping("/r") public Resp r() { return null; }
}
class Resp {
    @Schema(description = "columns")
    private List<ProfitColumnVO> columns = new ArrayList<>();
    @Schema(description = "rows")
    private List<ProfitRowVO> rows = new ArrayList<>();
    private List<String> warnings = new ArrayList<>();
    private FeeRate feeRate;
}
class ProfitColumnVO { private String title; }
class ProfitRowVO { private String ratioType; }
class FeeRate { private String month; }
"#;
    let doc = apifox_core::scan_sources(&[src.to_string()]);
    let schema = doc.schemas.get("Resp").expect("Resp");
    let Ty::Object(fields) = schema else { panic!("{schema:?}") };
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["columns", "rows", "warnings", "feeRate"]);
    assert!(doc.schemas.contains_key("ProfitRowVO"));
    assert!(doc.schemas.contains_key("ProfitColumnVO"));
}

#[test]
fn inherited_and_static_fields_match_reference() {
    let src = r#"
package a;
@RestController
public class C {
    @GetMapping("/r") public Resp r() { return null; }
}
class Base {
    private String createdBy;
    private Long id;
}
class Resp extends Base {
    private static final long serialVersionUID = 1L;
    private String name;
    private Long id;
}
"#;
    let doc = apifox_core::scan_sources(&[src.to_string()]);
    let Ty::Object(fields) = doc.schemas.get("Resp").expect("Resp") else { panic!() };
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["createdBy", "id", "name"]);
}

#[test]
fn scoped_scan_uses_annotations_and_mapped_modules_only() {
    let sources: Vec<(String, String)> = vec![
        (
            "shop/src/main/java/a/AnyName.java".into(),
            r#"
package a;
@RestController
public class AnyName {
    @GetMapping("/x") public Resp x() { return null; }
}
"#
            .into(),
        ),
        (
            "shop/src/main/java/a/Resp.java".into(),
            r#"
package a;
public class Resp { private Shared shared; }
"#
            .into(),
        ),
        (
            "common/src/main/java/b/Shared.java".into(),
            r#"
package b;
public class Shared { private String name; }
"#
            .into(),
        ),
        (
            "other/src/main/java/c/Hidden.java".into(),
            r#"
package c;
@RestController
public class Hidden {
    @GetMapping("/hidden") public String h() { return null; }
}
"#
            .into(),
        ),
    ];
    let paths: Vec<String> = sources.iter().map(|(p, _)| p.clone()).collect();
    let docs = apifox_core::scan_modules_scoped(
        &paths,
        |path| {
            sources
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, text)| text.clone())
        },
        &["shop".to_string()],
        &[],
    );
    assert_eq!(docs[0].endpoints.len(), 1);
    assert_eq!(docs[0].endpoints[0].path, "/x");
    assert!(docs[0].schemas.contains_key("Shared"));
}

#[test]
fn scoped_scan_falls_back_when_the_selection_has_no_controller() {
    let files = vec![
        (
            "erp/src/A.java".to_string(),
            r#"
package a;
@RestController
@RequestMapping("/a")
public class A {
    @GetMapping("/x") public String x() { return null; }
}
"#
            .to_string(),
        ),
        (
            "erp/src/Dto.java".to_string(),
            "package a;\npublic class Dto { private String name; }\n".to_string(),
        ),
    ];
    let paths: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
    let scan = apifox_core::scan_modules_scoped_report(
        &paths,
        |path| files.iter().find(|(p, _)| p == path).map(|(_, t)| t.clone()),
        &["erp".to_string()],
        &["erp/src/Dto.java".to_string()],
    );
    assert!(scan.selection_ignored, "a DTO selection must not hide the module");
    assert_eq!(scan.module_files, 2);
    assert_eq!(scan.docs[0].endpoints.len(), 1);
    assert_eq!(scan.docs[0].endpoints[0].path, "/a/x");
}

#[test]
fn scoped_scan_keeps_a_controller_selection() {
    let files = vec![
        (
            "erp/src/A.java".to_string(),
            r#"
package a;
@RestController
public class A {
    @GetMapping("/a") public String a() { return null; }
}
"#
            .to_string(),
        ),
        (
            "erp/src/B.java".to_string(),
            r#"
package a;
@RestController
public class B {
    @GetMapping("/b") public String b() { return null; }
}
"#
            .to_string(),
        ),
    ];
    let paths: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
    let scan = apifox_core::scan_modules_scoped_report(
        &paths,
        |path| files.iter().find(|(p, _)| p == path).map(|(_, t)| t.clone()),
        &["erp".to_string()],
        &["erp/src/A.java".to_string()],
    );
    assert!(!scan.selection_ignored);
    assert_eq!(scan.docs[0].endpoints.len(), 1);
    assert_eq!(scan.docs[0].endpoints[0].path, "/a");
}

#[test]
fn module_scope_accepts_a_project_root_that_is_the_module() {
    let paths = vec!["src/main/java/a/A.java".to_string()];
    assert_eq!(apifox_core::module_scope(&paths, "", "saas-backend"), "");
    assert_eq!(apifox_core::module_scope(&paths, "saas-erp", "saas-erp"), "");
    assert_eq!(apifox_core::module_scope(&paths, "saas-erp", "SAAS-ERP"), "");
    assert_eq!(
        apifox_core::module_scope(&paths, "saas-erp", "saas-backend"),
        "saas-erp"
    );
    let inside = vec!["saas-erp/src/main/java/a/A.java".to_string()];
    assert_eq!(apifox_core::module_scope(&inside, "saas-erp", "saas-backend"), "saas-erp");
}
#[test]
fn responses_keep_declared_codes_and_media_types() {
    let src = r#"
package a;
@RestController
public class C {
    @ApiResponses({
        @ApiResponse(code = 200, message = "OK"),
        @ApiResponse(code = 401, message = "未登录")
    })
    @GetMapping(value = "/x", produces = "application/xml")
    public U x() { return null; }
}
class U { private String name; }
"#;
    let doc = apifox_core::scan_sources(&[src.to_string()]);
    let ep = &doc.endpoints[0];
    let codes: Vec<&str> = ep.responses.iter().map(|r| r.code.as_str()).collect();
    assert_eq!(codes, vec!["200", "401"]);
    let ok = &ep.responses[0];
    assert_eq!(ok.media, "application/xml");
    assert_eq!(ok.description, "OK");
    assert_eq!(ok.ty, Some(Ty::Ref("U".into())));
    let unauthorized = &ep.responses[1];
    assert_eq!(unauthorized.ty, None);
    assert_eq!(unauthorized.description, "No Content");
}
