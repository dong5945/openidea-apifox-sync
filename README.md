# Apifox Sync for OpenIDEA

读 Spring Controller 源码 → 生成 OpenAPI 3.0.3 → 调 Apifox `import-openapi` 导入项目。
不启动服务、不依赖 Swagger，源码改完即可同步。

方案与取舍见根目录 `Apifox插件实现方案.md`。

## 结构

```
plugin.toml       清单：能力、命令、右键菜单、密钥声明
src/lib.rs        插件壳：读文件 / 勾选 / 密钥 / 一次 fetch / 通知
core/             纯 Rust 核心（无 host 调用，独立可测）
  src/java.rs     文本扫描：类、注解、方法、字段、Javadoc 与 ///
  src/spring.rs   Spring 注解 → Endpoint / JSON Schema
  src/openapi.rs  OpenAPI 3.0.3 渲染
  src/apifox.rs   导入请求（含分支/模块）+ 分支/模块列表 + counters 解析
  src/bin/apifox_gen.rs   CLI：目录 → OpenAPI
  tests/          20 项测试
```

## 按代码模块映射（对齐 ApifoxHelper）

1. 「映射」标签点 **检测模块**：读 `pom.xml` / `settings.gradle(.kts)`，每个模块生成一行；
   也可以自己增删行。
2. **双击行**打开编辑：启用、Apifox 项目 ID、分支/版本（下拉，来自
   `/api/v1/projects/{id}/sprint-branches`）、模块（下拉，来自 `/modules?branchId=`）。
3. 表格里的「启用」单元格可直接改成 是/否；`同步全部` 只跑启用的行。
4. 每行独立导入：`options.targetBranchId` + `options.moduleId`，只扫描该模块目录下的 `.java`。

分支/模块按「行」记住；项目 ID 改动后重开该行会重新拉分支。拉不到分支列表（权限/网络）时
降级主分支并提示，不会静默导错分支。

## 界面

右侧工具窗「Apifox Sync」两个标签（一个视图 body 只能是 Form 或 Table）：

- **Apifox Sync**：服务器 / 地址 / 访问令牌（留空不变，保存进钥匙串）+ 令牌状态。
- **映射**：启用 / 代码模块 / Apifox 项目 / 分支·版本 / 模块 / 接口目录 / 模型目录 / 覆盖策略；
  工具栏 `+`、`编辑`、`−`、`清空`、`从配置导入`、`预览变更`、`刷新分支`、`同步全部`（新增/删除作用于当前选中行；编辑页里用「刷新模块」重拉源代码模块下拉，不会自动建行）。双击行是带下拉的编辑表单
  （表格单元格只能输入文本，所以下拉放在编辑表单里）。

编辑表单覆盖导入的关键项：

| 字段 | 作用 |
| --- | --- |
| 源代码模块 | 从 pom.xml / settings.gradle 检测出的模块下拉，改行绑定的目录 |
| 团队 → Apifox 项目 | 团队下拉（`/api/v1/user-teams`）→ 项目下拉（`/teams/{id}/projects`），选中即回填项目 ID |
| Apifox 项目 ID（可手填） | 不想用下拉时直接填数字 |
| 分支 / 版本、模块 | `targetBranchId` / `moduleId`（`刷新分支` 重新拉） |
| 接口导入到 / 数据模型导入到（目录 ID） | `targetEndpointFolderId` / `targetSchemaFolderId` |
| 接口 / 数据模型覆盖策略 | 智能合并 / 覆盖所有字段 / 不导入 / 保留两者 |
| 参数命名转换 | 保持原样 / 小驼峰 / 大驼峰 / 下划线 / 短横线 / 全小写 / 全大写 |
| 覆盖时同步更新接口目录 | `updateFolderOfChangedEndpoint` |

模块名取 pom **自身**的 `artifactId`（已跳过 `<parent>`；同名时下拉显示 `名称 (#序号)`，不再带路径），
编辑页「刷新模块」实测 saas-backend 得到 38 个模块供下拉选择（只填列表，不生成行）。

另外自动处理：Spring / JAX-RS / Feign 注解、Swagger 2/3 注解、`@module`/`@ignore`、
`@JsonProperty/@JSONField/@SerializedName` 改名、`@Size/@Min/@Max/@Pattern/@Email` 约束、
枚举常量、`server.servlet.context-path`、按类型生成示例值。

界面与命令共用同一份 storage（映射数组），改哪边都生效；旧版单项目配置会自动迁移成一行。

### 项目 ID 填在哪里

1. 右侧「Apifox Sync」→「映射」标签；
2. **双击任意一行**（0.10.1 起表格不可编辑，双击/回车直接进编辑页）；
3. 表单第 5 项「**Apifox 项目 ID（必填，数字）**」就是填写处；
   也可以先在第 3、4 项选「团队 → Apifox 项目」，会自动回填 ID；
4. 同一页继续选分支/版本、模块，点「保存」。

## 同步范围与目录

- **只同步选中的 Controller/目录**：在项目视图右键某个 `.java` 或目录 → `同步全部`，
  只会用包含该路径的模块行，并且只导出这次选中文件里的接口（类型仍按全项目解析）。
- **tag 名称**：方法 `@module` → 类 `@module` → Swagger `@Tag(name=…)` / `@Api(tags=…)` → Controller 类名；tag 用于接口分组，不决定目录。
- **目录与参考实现一致**：不写 `x-apifox-folder`、不按 Controller 自动建目录；接口进映射里「接口导入到」选的目录，
  默认根目录。目录下拉来自 `GET /api/v1/projects/{id}/api-detail-folders?moduleId=`（模型目录用
  `api-schema-detail-folders`），编辑页「刷新目录」重新拉取；切分支/模块会自动重拉。
- 已经导入到根目录的接口要挪进目录：编辑该行，选好目录并勾选「覆盖时同步更新接口目录」再同步一次
  （对应 `updateFolderOfChangedEndpoint`）。
## 服务器、导出与配置

- **服务器地址**：设置页可在「公网 SaaS / 私有化部署」间切换并填地址；保存后所有请求改走该地址。
  自定义域名首次请求会触发宿主 `net:ask` 授权。`测试连接` 会读一次分支列表验证令牌与网络。
- **导出**：`导出 OpenAPI YAML` → `docs/openapi.yaml`；`导出 Postman Collection` →
  `docs/postman_collection.json`（命令面板或项目视图右键）。
- **配置即代码**：项目根放 `.apifox-sync.json`：
  `{"mappings":[{"name":"saas-erp","path":"saas-erp","projectId":"8957701","branchId":1,"moduleId":2}]}`
  ，映射页点「从配置导入」整体覆盖。
- **预览变更**：用上次同步时保存的内容哈希逐模块比较，报告「N 个模块有变更」；不访问网络。
- **导入结果**（0.11.0）：同步一开始就打开「导入结果」标签，每传完一个模块刷新一次——按模块列出
  项目 ID/名称、分支、模块、接口目录、数据模型目录、接口清单，以及 Endpoints/Schemas 的
  Created/Updated/Ignored/Failed 计数表（与参考插件控制台输出同一份信息）；失败的模块显示原因。
- **测试连接**：设置页按钮，用当前服务器 + 令牌读映射里的第一个项目。

## 已验证

```sh
cd core
cargo test                     # 22 单测 + 19 端到端，全绿
cargo run -- tests/fixtures --json out/openapi.json --project 3760990 --branch 4321 --module 77
```

`out/openapi.json` 是可直接导入 Apifox 的文档；`--project/--branch/--module` 会打印将要发送的请求
（含 `Bearer {{secret:apifox_token}}` 占位符，不会包含真实令牌）。

## 已验证：编译 + check

```text
$ rustup target add wasm32-wasip2          # 本机已装
$ cargo build --release --target wasm32-wasip2
   Finished `release` profile
$ cp target/wasm32-wasip2/release/openidea_plugin_lidengyin_apifox_sync.wasm dist/plugin.wasm
$ openidea-plugin check .
lidengyin.apifox-sync 0.11.1 — Apifox Sync
  code: dist/plugin.wasm (600.4 KiB, plugin API 1.9)
  categories: frameworks, tools
  contributes 5 × commands
  contributes 5 × menus
  contributes 1 × secrets
  contributes 3 × views
  capability project.write (required, High risk): Create and change files in the open project
  capability net:api.apifox.com (required, Medium risk): Connect to api.apifox.com (from the machine the project is on)
  capability net:ask (required, Medium risk): Ask to connect to a host that is not on the list the first time it tries; you allow or refuse each host, and can revoke the allowed ones
  capability secrets (required, Medium risk): Store secrets in the system keychain
  capability project.read (required, Low risk): Read files in the open project
  capability storage (required, Low risk): Store its own data
ok
```

`dist/` 不入库（`.gitignore`）；分发用签名包 `dist/lidengyin.apifox-sync-<ver>.oiplugin`，
安装时齿轮菜单选「从插件包文件安装…」（「从磁盘安装插件…」是文件夹选择器，只认含 `plugin.toml` 的目录）。

### 本机是怎么跑的（Windows 无官方 CLI）

release 只提供 `openidea-plugin-linux-x86_64.gz`，所以用 WSL 跑：

```sh
# WSL 内
gunzip -kf openidea-plugin.gz && chmod +x openidea-plugin
./openidea-plugin --version        # openidea-plugin 0.1.46 (plugin API 1.9.0)
cd /mnt/c/.../apifox-openidea && ./openidea-plugin check .
```

### SDK 依赖

`Cargo.toml` 里是 git tag `v0.1.46`。本机 `git clone github.com` 不通，验证时用源码包 + path patch：

```sh
curl -L https://codeload.github.com/l0ng-ai/openidea/tar.gz/refs/tags/v0.1.46 | tar -xz -C vendor
cargo build --release --target wasm32-wasip2 \
  --config 'patch."https://github.com/l0ng-ai/openidea".openidea-plugin.path="vendor/openidea-0.1.46/sdk/rust/openidea-plugin"'
```

`core/` 有自己的 `[workspace]`，插件根用 `exclude = ["core"]` 隔开——核心可在没有 SDK 的情况下单独 `cargo test`。

## 能力与用途

| 能力 | 用途 |
| --- | --- |
| `project.read` | 扫 `.java` 源码 |
| `project.write` | 只用于导出 `docs/openapi.json` |
| `storage` | 每个 IDE 项目记住自己的 Apifox 项目 ID |
| `secrets` | 令牌存系统钥匙串；插件只判断"是否已填"，请求头由宿主替换 |
| `net:api.apifox.com` | 仅在执行同步时发一次导入请求 |

## 已知限制

- 支持 Java（Spring MVC / JAX-RS / Feign 接口）与 Swagger 2/3 注解；**Kotlin/KDoc 未支持**（需独立的 Kotlin 解析器）。
- 类型解析基于**项目源码**：跨模块 DTO 已支持（全项目建索引，即使只同步一个模块）；
  项目外（JAR 依赖里的类）仍退化成 `{}`，需要精确模式（ASM 辅助进程）。
- 只删除不了的远端接口：覆盖策略选「保留两者/不导入」时本来就不删；要同步删除需 `deleteUnmatchedResources`，故意没开。
- 目录导入填的是 Apifox 目录 ID（界面里复制），没有目录树选择器；团队/项目树选择也未做。
- `context-path` 只读项目内的 `application.*` / `bootstrap.*`；Nacos/Apollo 等配置中心里的值读不到。
- 分支/模块列表用的是 Apifox 客户端内部 API（与官方 CLI 相同），不在开放 API 文档内；
  拿不到时降级主分支并提示，不会静默导错分支。
- 一个代码模块只映射到一个「Apifox 项目 + 分支 + 模块」组合。
- 未做：Actuator 端点、YApi 导出、接口调试（Fast Request）、Groovy/JS 规则引擎、上传前拉取远端最新数据、
  代理/日志级别/字符集设置（代理由 IDE 统一处理）、真正的增量上传（只有哈希预览）。
- 「预览变更」只比较本地哈希，不会拉远端；刷新分支/模块后才反映远端状态。

## 响应显示 `No schema defined` / 示例 null？

说明该接口的返回类型没有被解析成 schema，Apifox 只能显示空对象。按顺序排查：

1. 返回类型是否在**项目源码**里：跨模块可以，JAR 里的类不行；
2. 泛型外层是不是项目自定义包装类：`R<T>/Result<T>/ApiResponse<T>` 等能自动解包，
   其他命名需要加进 `spring.rs` 的 `WRAPPERS`；
3. 用 CLI 单独验证：
   `cargo run -- <项目根> --code-module <模块路径> --json out.json`，
   看 `paths.<path>.post.responses.200.content.application/json.schema` 是 `$ref` 还是 `{}`。