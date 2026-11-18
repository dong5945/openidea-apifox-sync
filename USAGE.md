# Apifox Sync 使用说明（发给同事版）

把 Spring Controller 源码直接生成 OpenAPI 3.0.3 并导入 Apifox：
不启动服务、不依赖 Swagger，改完源码即可同步。

- 插件 id：`lidengyin.apifox-sync`
- 版本：`0.11.1`
- 要求：OpenIDEA **0.1.46 及以上**（`plugin.toml` 声明 `engines.openidea = "^1.9"`）
- 安装包：`dist/lidengyin.apifox-sync-0.11.1.oiplugin`（约 229 KiB，已签名，内含 wasm）
- sha256：`8c201c51a981d6d121a52eb833220dac5cf3bd09d07ba38618d5a21daa4bc0f0`

用的人**不需要** Rust、Node、WSL 环境——插件是编译好的 wasm，装完即用。

---

## 一、安装（约 2 分钟）

1. 打开 OpenIDEA（0.1.46+）。
2. `Settings › Plugins` → 齿轮菜单 → **从插件包文件安装…** → 选 `lidengyin.apifox-sync-0.11.1.oiplugin`。
   （不要选「从磁盘安装插件…」，那是文件夹选择器，只认含 `plugin.toml` 的目录。）
3. 弹出权限清单时同意：

   | 能力 | 用途 |
   | --- | --- |
   | `project.read` | 扫 `.java` 源码 |
   | `project.write` | 只用于导出 `docs/openapi.yaml/json`、Postman Collection |
   | `storage` | 每个项目记住自己的映射配置 |
   | `secrets` | 令牌存系统钥匙串；插件只判断"是否已填"，请求头由宿主替换 |
   | `net:api.apifox.com` | 同步时发一次导入请求（私有化部署另加自定义域名授权） |
   | `net:ask` | 首次连非白名单地址（如私有化部署）时弹一次授权 |

4. 装完右侧出现 **Apifox Sync** 工具窗；没出现就重启 IDE。

> 目前未上架插件市场，安装包从 Release 下载：
> https://github.com/lidengyin/openidea-apifox-sync/releases/download/v0.11.1/lidengyin.apifox-sync-0.11.1.oiplugin
> 发布链路见 `PUBLISHING.md`。

---

## 二、首次配置

### 1. 申请 Apifox 访问令牌

Apifox → 头像 → 账号设置 → API 访问令牌 → 新建并复制。

### 2. 填服务器和令牌

右侧 **Apifox Sync** 标签：

- 「服务器」选 **公网 SaaS**（`https://api.apifox.com`）或 **私有化部署** 并填自己的地址；
  自定义域名首次请求会触发宿主 `net:ask` 授权。
- 粘贴令牌 → 保存（进系统钥匙串，插件本身读不到明文；留空 = 保持不变）。
- 点 **测试连接**：用当前服务器 + 令牌读一次分支列表，验证令牌与网络。

### 3. 建映射行（模块 → 项目/分支/模块）

切到 **「映射」** 标签：

1. 点 **检测模块**：读 `pom.xml` / `settings.gradle(.kts)`，得到候选模块下拉（只填下拉，不自动建行）。
2. 点 `+` 新建行，或**双击行**（表格不可编辑）打开编辑表单，填：

   | 字段 | 说明 |
   | --- | --- |
   | 源代码模块 | 检测出的模块下拉，决定扫描哪个目录 |
   | 团队 → Apifox 项目 | 下拉联动，选中即回填项目 ID |
   | Apifox 项目 ID | 不想用下拉时手填数字（必填） |
   | 分支 / 版本、模块 | 对应 `targetBranchId` / `moduleId`，`刷新分支` 重拉 |
   | 接口导入到 / 数据模型导入到 | `targetEndpointFolderId` / `targetSchemaFolderId`，默认根目录 |
   | 覆盖策略 | 智能合并 / 覆盖所有字段 / 不导入 / 保留两者 |
   | 参数命名转换 | 保持原样 / 小驼峰 / 大驼峰 / 下划线 / 短横线 / 全小写 / 全大写 |
   | 覆盖时同步更新接口目录 | `updateFolderOfChangedEndpoint` |

3. 表格「启用」列可直接改成 是/否；`同步全部` 只跑启用的行。
4. 多个模块就多建几行（一个代码模块只映射到一个「项目 + 分支 + 模块」组合）。

### 4. 配置即代码（可选）

项目根放 `.apifox-sync.json`，映射页点 **从配置导入** 整体覆盖：

```json
{"mappings":[{"name":"saas-erp","path":"saas-erp","projectId":"8957701","branchId":1,"moduleId":2}]}
```

---

## 三、日常使用

| 操作 | 入口 | 说明 |
| --- | --- | --- |
| 同步全部 | 映射页工具栏 / 项目视图右键 | 只处理启用的行；右键某个 `.java` 或目录时只导出选中范围内的接口（类型仍按全项目解析） |
| 导入结果 | 自动打开 | 按模块列出项目/分支/模块/目录、接口清单与 Created/Updated/Ignored/Failed 计数；失败模块显示原因 |
| 预览变更 | 映射页工具栏 | 用上次同步的本地哈希比较，报告「N 个模块有变更」，不访问网络 |
| 导出 OpenAPI YAML | 命令面板 / 右键 | 写 `docs/openapi.yaml` |
| 导出 OpenAPI JSON | 命令面板 / 右键 | 写 `docs/openapi.json` |
| 导出 Postman Collection | 命令面板 / 右键 | 写 `docs/postman_collection.json` |
| 刷新分支 / 刷新模块 / 刷新目录 | 编辑表单 | 重新拉远端下拉，不自动建行 |

同步规则要点：

- **tag 名称**（接口分组）：方法 `@module` → 类 `@module` → Swagger `@Tag(name=)` / `@Api(tags=)` → Controller 类名。
- **目录**：不写 `x-apifox-folder`、不按 Controller 自动建目录；接口进映射里选的目录，默认根目录。
  已导到根目录的接口要搬进目录：编辑行选目录 + 勾「覆盖时同步更新接口目录」再同步一次。

支持范围：Spring MVC / JAX-RS / Feign、Swagger 2/3 注解、`@module`/`@ignore`、
`@JsonProperty/@JSONField/@SerializedName` 改名、`@Size/@Min/@Max/@Pattern/@Email` 约束、枚举常量、
`server.servlet.context-path`、按类型生成示例值。

---

## 四、想自己编译（开发者）

```sh
rustup target add wasm32-wasip2
cd apifox-openidea
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/openidea_plugin_acme_apifox_sync.wasm dist/plugin.wasm
sha256sum dist/plugin.wasm    # 与 target 产物一致才算对
```

自检（官方 CLI 只发 Linux 版，Windows 走 WSL）：

```sh
wsl -d Ubuntu-22.04
gunzip -kf openidea-plugin.gz && chmod +x openidea-plugin
cd /mnt/c/.../apifox-openidea && ./openidea-plugin check .
```

本地调试用 **Linked for development**：把插件目录写进
`%APPDATA%\ai.l0ng.openidea\plugins\links.json`：

```json
{ "paths": ["C:\\Users\\DELL\\Documents\\self\\openIdea_plugin\\apifox-openidea"] }
```

改完代码 **Reload** 才生效。若 `github.com` 不通，用 codeload 源码包 + path patch 指向本地 SDK：

```sh
curl -L https://codeload.github.com/l0ng-ai/openidea/tar.gz/refs/tags/v0.1.46 | tar -xz -C vendor
cargo build --release --target wasm32-wasip2 \
  --config 'patch."https://github.com/l0ng-ai/openidea".openidea-plugin.path="vendor/openidea-0.1.46/sdk/rust/openidea-plugin"'
```

测试：

```sh
cd core && cargo test --release    # 22 单测 + 19 端到端
```

---

## 五、已知限制

- 仅支持 Java（Spring MVC / JAX-RS / Feign）；**Kotlin/KDoc 不支持**。
- 类型解析基于**项目源码**：跨模块 DTO 可以，JAR 里的类退化成 `{}`。
- 远端已删除的接口不会被删（`deleteUnmatchedResources` 故意没开）。
- 目录填的是 Apifox 目录 ID，没有目录树选择器；团队/项目已是下拉。
- `context-path` 只读项目内 `application.*` / `bootstrap.*`，Nacos/Apollo 里的读不到。
- 分支/模块列表走 Apifox 客户端内部 API（与官方 CLI 相同），不在开放 API 文档内；
  拉不到时降级主分支并提示，不会静默导错分支。
- 一个代码模块只对应一个「项目 + 分支 + 模块」。
- 未做：Actuator、YApi 导出、接口调试、Groovy/JS 规则引擎、上传前拉远端最新数据、真正的增量上传（只有哈希预览）。

---

## 六、常见问题

**Apifox 里显示 `No schema defined` / 示例是 null？**

按顺序排查：

1. 返回类型在**项目源码**里吗？跨模块可以，JAR 里的不行。
2. 泛型外层是不是自定义包装类？`R<T>` / `Result<T>` / `ApiResponse<T>` 能自动解包，
   其他命名要加进 `spring.rs` 的 `WRAPPERS`。
3. 用 CLI 定位：
   ```sh
   cargo run -- <项目根> --code-module <模块路径> --json out.json
   ```
   看 `paths.<path>.post.responses.200.content.application/json.schema` 是 `$ref` 还是 `{}`。

**提示 400105 / Client version too low？**
插件已带正确请求头（`X-Apifox-Cli-Version: 2.2.12` + `X-Apifox-Api-Version: 2026-05-28`）；
若仍出现，多为令牌无效（鉴权先于版本校验，先确认令牌）。

**同步后分支不对？**
编辑行点 `刷新分支` 重新拉；拉不到列表会降级主分支并在导入结果里提示。

**改了代码但行为没变？**
Reload 插件（Linked for development 场景），或确认 `dist/plugin.wasm` 是最新构建（sha256 对比）。
