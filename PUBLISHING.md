# 发布到 OpenIDEA 插件市场（lidengyin/openidea-apifox-sync）

## 当前状态

| 项 | 状态 |
| --- | --- |
| 插件 id | `lidengyin.apifox-sync`（已从占位 `acme` 改好） |
| 源码仓库 | https://github.com/lidengyin/openidea-apifox-sync（已存在，公开，空） |
| 本地 git | 已 `init` + 首次提交（26 个文件），remote 已指向上面的仓库 |
| 推送 | **未成功**：本机到 github.com:443 连不通，需要你在有代理的环境执行 `git push` |
| 签名密钥 | 私钥 `C:\Users\DELL\.openidea-keys\apifox-sync.key`（勿提交、自行备份） |
| 公钥 | `8oG5Be7v1eVC5djl5JahY89IyRu+PF/6OUG/Tb6E8d4=` |
| 安装包 | `dist/lidengyin.apifox-sync-0.10.1.oiplugin`（208.5 KiB，已签名） |
| sha256 | `e44c7600500b5355a12c38e045b791552e7b8c362f9615107b1909646d773ec5` |
| 市场 PR 文件 | `../openidea-plugins-pr/publishers/lidengyin.toml`、`plugins/lidengyin.apifox-sync.toml` |

## 1. 推送源码

在本机（或任意能访问 github 的机器）执行：

```sh
cd apifox-openidea
git push -u origin main
```

若提示认证：用 GitHub Personal Access Token 作密码，或 `gh auth login` 后再推。

## 2. 打 Release

仓库里创建 tag `v0.10.1` 的 Release，上传：

- `dist/lidengyin.apifox-sync-0.10.1.oiplugin`

上传后确认直链可下载（应等于 PR 文件里的 `package`）：

```
https://github.com/lidengyin/openidea-apifox-sync/releases/download/v0.10.1/lidengyin.apifox-sync-0.10.1.oiplugin
```

> Release 没建好之前，PR 的 CI 会下载 404，先建 Release 再提 PR。

## 3. 向 openidea-plugins 提 PR

Fork `https://github.com/l0ng-ai/openidea-plugins`，把准备好的两个文件放进去：

- `publishers/lidengyin.toml`
- `plugins/lidengyin.apifox-sync.toml`

内容就是 `openidea-plugins-pr/` 下的两份。PR 标题示例：`Add lidengyin.apifox-sync`。

CI 会：下载包 → 校验 sha256 → 用最新 IDE 跑 `openidea-plugin check` → 用公钥验签 → 合入后生成索引，
约一小时内出现在 OpenIDEA 的 Settings › Plugins › Available。

## 4. 后续版本

```sh
openidea-plugin build
openidea-plugin pack . --key ~/.openidea-keys/apifox-sync.key \
  --publisher lidengyin --out dist/lidengyin.apifox-sync-<ver>.oiplugin
sha256sum dist/lidengyin.apifox-sync-<ver>.oiplugin
```

在 `plugins/lidengyin.apifox-sync.toml` 里**追加**一个 `[[versions]]`（旧的保留），
把 `version` / `package` / `sha256` 换成新值，再提 PR。版本号必须与 `plugin.toml` 一致。

## 5. 规则与密钥管理

- 必须有公开源码仓库与开源协议（`LICENSE` 已加 MIT）；
- README 要说明每项能力为什么需要（`README.md` 的「能力与用途」已覆盖）；
- 不得包含非源码构建的混淆代码；
- 换密钥：先在新 `keys` 里加新公钥并保留旧公钥，用旧密钥
  `openidea-plugin endorse "<新公钥>" --key old.key --publisher lidengyin`，把输出写进 `[endorsements]`；
- 删除 `plugins/lidengyin.apifox-sync.toml` 即下架（已安装副本仍可用）；
- 对新 IDE 复检失败且 30 天不修会被下架。