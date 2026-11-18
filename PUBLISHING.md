# 发布到 OpenIDEA 插件市场

本地已验证可打包签名（0.10.1）：

```
dist/acme.apifox-sync-0.10.1.oiplugin   207.8 KiB
sha256 26f08c2f8cb93bfc1de682266d5a06dbe4e80d30dc9168a046ccb8c93606d331
签名公钥 8oG5Be7v1eVC5djl5JahY89IyRu+PF/6OUG/Tb6E8d4=
```

私钥在 `C:\Users\DELL\.openidea-keys\apifox-sync.key`，**不要提交到任何仓库**，自行备份。

## 0. 发布前必须满足

| 要求 | 现状 |
| --- | --- |
| 公开源码仓库 + 开源协议 | 需要建仓库；`LICENSE` 已放好（MIT） |
| README 说明功能与每项高危能力用途 | `README.md` 的「能力与用途」已覆盖 |
| 无混淆/压缩代码 | 纯 Rust 源码构建，满足 |
| 插件 id = `<publisher>.<name>` | 当前 `acme.apifox-sync`，`acme` 是占位名，建议改成你的 GitHub 用户名 |
| 签名包 | 已用 `pack` 生成 |

改 publisher 的做法（例：`myname`）：

1. `plugin.toml` 里 `id = "myname.apifox-sync"`；
2. 重新 `openidea-plugin build` 与 `pack --publisher myname`；
3. 市场索引里 publisher 文件也用 `myname`。

## 1. 本机打包命令

```sh
# CLI 在 WSL 里跑（Windows 无原生包）
openidea-plugin keygen ~/.openidea-keys/apifox-sync.key     # 只做一次
openidea-plugin build
openidea-plugin pack . --key ~/.openidea-keys/apifox-sync.key \
  --publisher <publisher> --out dist/<publisher>.apifox-sync-<version>.oiplugin
openidea-plugin check dist/<publisher>.apifox-sync-<version>.oiplugin
sha256sum dist/<publisher>.apifox-sync-<version>.oiplugin
```

`pack` 会先跑一遍 `check`，输出 `ok` 才算成功。

## 2. 建仓库与 Release

1. 新建**公开**仓库（建议名 `openidea-apifox-sync`），提交源码（不要包含 `dist/*.oiplugin`、`.key`）。
2. 打 tag `v0.10.1`，在该 Release 上传：
   - `<publisher>.apifox-sync-0.10.1.oiplugin`
   - 可选：同名 `.sha256`
3. 记下 Release 资源直链：
   `https://github.com/<owner>/openidea-apifox-sync/releases/download/v0.10.1/<publisher>.apifox-sync-0.10.1.oiplugin`

## 3. 向 openidea-plugins 提 PR

Fork `https://github.com/l0ng-ai/openidea-plugins`，加两个文件：

`publishers/<publisher>.toml`

```toml
name = "你的名字"
owners = ["<GitHub 账号>"]          # 允许改这些条目的人
keys = ["8oG5Be7v1eVC5djl5JahY89IyRu+PF/6OUG/Tb6E8d4="]
revoked-keys = []
```

`plugins/<publisher>.apifox-sync.toml`

```toml
repository = "https://github.com/<owner>/openidea-apifox-sync"
suggest = ["pom.xml", "build.gradle.kts", "build.gradle"]   # 最多 8 个，无通配符

[[versions]]
version = "0.10.1"
package = "https://github.com/<owner>/openidea-apifox-sync/releases/download/v0.10.1/<publisher>.apifox-sync-0.10.1.oiplugin"
sha256 = "26f08c2f8cb93bfc1de682266d5a06dbe4e80d30dc9168a046ccb8c93606d331"
```

> `sha256` 必须是**你实际打包的那个文件**的哈希；改 id/版本重打后要重新算。

## 4. CI 会做什么

- 下载 `package`，校验 `sha256`；
- 用最新 OpenIDEA 的 `openidea-plugin check` 跑一遍；
- 用 `publishers/<publisher>.toml` 的 `keys` 验签；
- 合入后生成 `index.json`，约一小时内出现在 Settings › Plugins › Available；
- 每天对最新 IDE 复检一次；30 天内不修复会被下架。

## 5. 版本更新

- 只追加 `[[versions]]`，旧版本保留（旧 IDE 会装到它支持 API 的最后版本）；
- `version` 必须与包内 `plugin.toml` 的 `version` 一致；
- 换签名密钥：先在新 `keys` 里加新公钥、保留旧公钥，用旧密钥
  `openidea-plugin endorse "<新公钥>" --key old.key --publisher <id>`，把输出写进 `[endorsements]`。

## 6. 市场规则要点

- 新 publisher / 新插件由维护者审核源码、能力申请理由，并检查是否与现有插件重复；
- 包必须能从公开仓库构建，不得包含未从该源码构建的混淆代码；
- README 必须说明为什么需要 `project.write` / `net:ask` / `secrets` 等高危能力；
- 删除 `plugins/<publisher>.<name>.toml` 即下架（已安装副本仍可用）。

## 7. 当前还差的两件事

1. 仓库尚未 `git init` / 建远程仓库；
2. `plugin.toml` 的 `id` 仍是占位的 `acme.apifox-sync`，正式发布前改成 `<你的 GitHub 用户名>.apifox-sync` 并重新打包。