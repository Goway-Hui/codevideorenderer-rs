# 发布手册（crates.io）

本项目同时发布**库**（`codevideorenderer`）与**二进制**（`cvr`）。
本文件是内部流程文档，已在 `Cargo.toml` 的 `exclude` 里，不会随包发布。

---

## 0. 现状与待办

| 项 | 状态 |
|---|---|
| `cargo publish --dry-run` | ✅ 通过（打包 + 编译验证） |
| 打包体积 | ✅ 27 files / 13.4 MiB → **7.1 MiB 压缩**（crates.io 上限 10 MiB） |
| 名称可用性 | ✅ `codevideorenderer` 系列未被占用 |
| `license` / `description` / `keywords` / `categories` / `documentation` | ✅ 已填 |
| `repository` / `homepage` | ✅ `https://github.com/Goway-Hui/codevideorenderer-rs` |
| 版本号 | `0.1.0`（首次发布建议保持） |
| git 仓库 | ✅ 已初始化并推送到 GitHub（`main`），打包自动遵守 `.gitignore` |

---

## 1. 一次性准备

```console
# 1) 用 GitHub 账号登录 https://crates.io 并接受使用条款
# 2) 在 https://crates.io/settings/tokens 生成一个 API token（scope 选 publish-new + publish-update）
# 3) 本地登录（token 会存到 ~/.cargo/credentials.toml）
$ cargo login <你的-token>
```

补充 `Cargo.toml` 的仓库地址（推荐，会显示在 crates.io 侧栏与 docs.rs）：

```toml
repository = "https://github.com/<你>/CodeVideoRenderer-rs"
homepage = "https://github.com/<你>/CodeVideoRenderer-rs"
```

---

## 2. 发布前检查清单

每次发布前跑一遍，全绿再发：

```console
$ cargo fmt --check
$ cargo clippy --all-targets -- -D warnings
$ cargo test --release
$ cargo publish --dry-run
```

`--dry-run` 会真的打包并编译验证，只是不上传。**这一步能拦住绝大多数发布事故**
（体积超限、文件缺失、编译失败、元数据非法）。

再确认一次打包内容与体积：

```console
$ cargo package --list          # 应该只有 src/ tests/ examples/ assets/ 与几份 md
$ cargo package                 # 看 “Packaged N files, X MiB (Y MiB compressed)”
```

---

## 3. 发布

```console
$ cargo publish
```

工作区必须干净（所有改动都已提交）。若 cargo 报 `working directory is dirty`，先提交：
`git add -A && git commit -m "..."`。

发布完成后：

```console
# 等几十秒让 crates.io 索引更新，然后验证“别人装得上”
$ cargo install codevideorenderer --force      # 会构建并安装 cvr 二进制
$ cvr --version
$ cvr --list-styles | head

# 库文档
# https://docs.rs/codevideorenderer 会自动构建
```

---

## 4. git 仓库（已完成）

仓库已初始化并推送到 <https://github.com/Goway-Hui/codevideorenderer-rs>，默认分支 `main`。
由此带来三点变化：

- `cargo publish` 不再需要 `--allow-dirty`，只要求工作区干净；
- cargo 打包时会遵守 `.gitignore`（`*.mp4`、`frame_*.png`、`/target/`、`/.reasonix/` 等），
  `Cargo.toml` 里的 `exclude` 仍然保留 —— 两者叠加，多一层保险；
- 行尾由 `.gitattributes`（`* text=auto eol=lf`）钉死为 LF，避免 Windows 的 `core.autocrlf`
  在检出时改写工作区文件。

日常流程：

```console
$ git add -A && git commit -m "描述这次改动"
$ git push
```

---

## 5. 发布是不可逆的

三件事一旦发生就无法撤销，务必在 `cargo publish` 前确认：

1. **crate 名是永久的**。发布即占名，不能改名、不能删除（只能 `cargo yank` 撤销某个版本，
   但名字仍属于你）。
2. **版本号不能复用**。`0.1.0` 发布后就不能再发一次 `0.1.0`，只能发 `0.1.1`。
   所以发布前把该改的都改好。
3. **上传的内容永久留档**。源码、README、打包进去的资源都会公开且不可删除。

---

## 6. 体积预算（重要）

包体 98% 是内置字体 `assets/CodeVideoRendererFont.ttf`（约 13 MiB）。当前压缩后
**7.1 MiB**，离 crates.io 的 **10 MiB** 上限只剩约 30% 余量。

因此：

- **不要**把生成的视频、截图、帧序列提交进仓库根目录 —— 它们会被打包（本目录不是 git
  仓库时尤其要注意，`.gitignore` 不起作用）。`Cargo.toml` 里的 `exclude` 已经挡住了
  `*.mp4` / `frame_*.png` / `preview.png` / `frames/`，新增其它大文件时要同步补进去。
- 想进一步瘦身，可以把字体拆成独立的 `codevideorenderer-fonts` crate，运行时按需下载；
  代价是「开箱即用」这一点没了。
- 用 `cargo package` 的输出核对实际体积，不要凭感觉估。

---

## 7. 后续版本

```console
$ # 1) 改 Cargo.toml 的 version，并在 CHANGELOG.md 里把 Unreleased 段落固化成该版本
$ # 2) 跑 §2 的检查清单
$ cargo publish --allow-dirty
```

版本号遵循语义化版本：内部重构与性能优化走 `0.1.x`，新增 API 或改变默认行为走 `0.2.0`
（例如把默认主题从 `material` 换成 `midnight` 这类改动就属于后者）。

---

## 8. 常见失败与处理

| 报错 | 原因 | 处理 |
|---|---|---|
| `file size ... exceeds the maximum` | 打包体积超过 10 MiB | 检查是否混入了视频/图片，补进 `Cargo.toml` 的 `exclude` |
| `crate version 0.1.0 is already uploaded` | 版本号已用过 | 提升 `version` |
| `crate name ... is already taken` | 名字被占 | 换包名（`[lib] name` 可以保持 `codevideorenderer` 不变） |
| `working directory is dirty` | 有未提交的改动 | 先 `git add -A && git commit` |
| `failed to verify package tarball` | 打包后的代码编译不过 | 通常是文件被 `exclude` 漏掉（例如新增了被引用的资源）；看 `cargo package --list` |
| `API token not found` | 没登录 | `cargo login <token>` |
