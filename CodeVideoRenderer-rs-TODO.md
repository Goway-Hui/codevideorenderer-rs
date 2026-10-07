# CodeVideoRenderer-rs 优化清单 v2.0

> **版本**：v2.0（第二次全量走查）— **已执行完毕**，结果见 §5
> **上一版**：v1.0，2026-10-07，30 条，原文存档于 [`docs/archive/TODO-v1.0.md`](docs/archive/TODO-v1.0.md)
> **更新日期**：2026-10-07
> **范围**：`src/`（14 个模块）、`tests/smoke.rs`、`examples/`、`Cargo.toml`、`README.md`、`CHANGELOG.md`
> **方法**：v1 全量落地后重新走查 + 五个临时探针实测（layout 分解、帧成本、去重、复杂度、PNG 分解）
> **环境**：Windows / rustc 1.99 / edition 2024 / AMD Ryzen 9 7945HX (16C/32T) / PATH 中无 ffmpeg

## 0. v2.0 起点基线（实测）

| 检查项 | 结果 |
| --- | --- |
| `cargo test --release` | 39 passed / 0 failed（27 集成 + 6 encode + 3 font + 3 doctest） |
| `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` | 均零输出 |
| 端到端 CLI（480p / 15fps / PNG，790 帧） | 5.23 s（151 frames/s） |
| 端到端 CLI（1080p / 30fps，117 帧） | 2.64 s |
| bench（1080p，600 帧） | 单线程 0.33 s（~1,800 fps），4 线程 0.19 s（~3,200 fps） |
| `layout()` 1600 行 / 78,763 字形 | 19.5 ms（0.25 µs/char，与规模无关） |
| 单帧渲染 1080p（同一 layout） | 6.6 ms，**与文件大小无关**（50 行 5.79 ms → 1600 行 6.60 ms） |
| `cargo package` | 27 个文件，13.4 MiB → 7.0 MiB 压缩 |
| 直接依赖 | 4 个：`rayon` / `thiserror` / `tiny-skia` / `ttf-parser` |

---

## 1. v1.0 清单归档（全部处理完毕）

| v1 条目 | 状态 | 落点与实测 |
| --- | --- | --- |
| 1.1 未使用 `render_into_vec` | ✅ | per-worker buffer 池 + `render_into_vec`，batch 降到 `threads`；480p/15fps 5.84 s → 5.23 s |
| 1.2 每次 render 泄漏字体 | ✅ | `font.rs` 进程级 `CACHE`；单测断言两次查找共享同一 bytes |
| 1.3 `layout` 的 O(N²) | ✅ | 删 `char_index`，改自增 `text_index`；1600 行 439 ms → 16.4 ms，µs/char 持平 |
| 1.4 荒谬分辨率 abort | ✅ | `MAX_PIXELS`（8K）+ `Error::ResolutionTooLarge`；CLI 参数阶段即拒绝 |
| 2.1 高亮跑在改写文本上 | ✅ | `preprocess` 不再替换空格；像素回归证明只影响被误判的字符 |
| 2.2 `is_infinite` 漏检 | ✅ | `line_spacing` / `camera_scale` 改 `!is_finite()` |
| 2.3 NaN 检查顺序 | ✅ | finiteness 检查提到比较之前 |
| 2.4 CLI 打印反算 fps | ✅ | `RenderReport.fps`；输出 `790 @ 15fps` |
| 2.5 恒真检查 | ✅ | 已删除 |
| 2.6 字体搜索注释 | ✅ | 注释说明 `embed-font` 时系统字体不可达 |
| 2.7 每次 spawn 探测 ffmpeg | ✅ | 去掉预探测，`NotFound` → `Error::FfmpegMissing` |
| 3.1 辉光未实现 | ✅ 实现 / ⏳ 未实测 | `--glow` 单次编码滤镜链；本机无 ffmpeg，仅参数构造有测试 |
| 3.2 帧去重 | ✅（前提已修正） | 渲染侧 `FrameSignature` 去重 + `--snap-camera`；1080p 4.63×（30fps）/ 8.63×（60fps） |
| 3.3 语言降级静默 | ✅ | `lexer::is_supported` + CLI note |
| 3.4 CJK/emoji 回退 | ⏳ | 见 §3.1 |
| 3.5 CJK 列宽未记录 | ✅ | README「metric-based alignment」说明 |
| 4.1 `Sink` 封闭枚举 | ✅ | `FrameSink` trait + `render_with_sink()` |
| 4.2 无进度回调 | ✅ | `ProgressCallback` |
| 4.3 无取消机制 | ✅ | `Arc<AtomicBool>` + `Error::Cancelled` |
| 4.4 builder 半成品 | ✅ | 20 个 `with_*` |
| 5.1 不是 git 仓库 | ⏸ | 用户暂缓 |
| 5.2 无 CI | ⏸ | 用户暂缓（`fmt`/`clippy` 已可作 CI 门槛） |
| 5.3 发布元数据 | ✅ 部分 | 补 `documentation`/`keywords`/`categories`；`repository`/`homepage` 待远端地址 |
| 5.4 打包体积 | ✅ 文档 | README「Package size」 |
| 5.5 无 CHANGELOG | ✅ | `CHANGELOG.md`（Additions/Changes/Deletions） |
| 5.6 README Options 不全 | ✅ | 22 个 CLI 开关全部列出 |
| 5.7 `.gitignore` 漏 `preview.png` | ✅ | 已补 |
| 6 零引用死代码 | ✅ | 11 个符号清除，`clippy -D warnings` 零警告 |

---

## 2. v2.0 新发现：本轮执行

### 2.1 ✅ PNG 序列输出是串行的，占端到端 83–90% 的时间

> **已修复**：`PngSink` 改为编码线程池（`PngSink::with_threads`，默认每核一个），`write` 只做一次
> 像素拷贝后入队，`finish` / `drop` 排空队列并聚合首个错误。端到端 **480p/15fps 5.23 s → 0.94 s
> （5.7×）**、**1080p/30fps 2.64 s → 0.59 s（4.5×）**；1 线程与 4 线程产出逐字节一致（有测试）。

- **位置**：`src/encode.rs` 的 `PngSink::write_frame`（`pixmap.save_png` 在主线程逐帧调用）
- **实测**（单帧，同机）：

  | 分辨率 | 渲染 | `encode_png`（内存） | `save_png`（含写盘） | 编码占比 |
  | --- | ---: | ---: | ---: | ---: |
  | 854×480 | 1.08 ms | 4.52 ms | 5.43 ms | **83%** |
  | 1920×1080 | 2.43 ms | 20.54 ms | 19.75 ms | **89%** |

- **影响**：`--frames` 路径（没有 ffmpeg 时的官方替代，也是清单附录 B 与 CI 唯一可跑的路径）把 32 核机器用成了单核。790 帧 480p 要 5.2 s，其中约 5.0 s 是串行 PNG 压缩。
- **建议**：`PngSink` 内部起一个小型编码线程池（有界队列 + `Mutex<Receiver>`），`write` 只拷贝像素后入队，`finish` 排空队列并聚合错误。trait 签名不变，自定义 sink 不受影响。代价是每帧一次 `to_vec()`（8 MB memcpy，约为压缩成本的 1/20）。
- **预期**：480p 端到端 5.2 s → 约 1.2 s；1080p 同比例。

### 2.2 ✅ 库 API 没有可复用的 pipeline，预览逐帧重建

> **已修复**：新增 `Prepared`（拥有 `Layout`/`Timeline`/`Font`/`GlyphOutlines`，借用 `&'static Theme`），
> 提供 `render_at(t)` / `renderer(w, h)` / 各字段访问器；`render_preview` 与 `render_frames` 都改为基于它，
> 两份重复的构建代码合并（`timeline_options` 亦共用）。有测试断言"一次构建渲染多帧"与 `render_preview` 逐像素一致。

- **位置**：`src/api.rs` 的 `render_preview`（每次调用都 `preprocess → layout → Timeline::build → GlyphOutlines::build`）
- **实测**：1600 行代码每次预览要付 **19.5 ms layout + lexer + outlines**，与"换一个时间点画一帧"的真实成本（6.6 ms）同量级；2048 行的文件更贵。
- **影响**：README 把 `render_preview` 定位成"previews and tests"的入口，但任何"拖动时间轴预览"的用法都会 O(帧数) 次重建。
- **建议**：新增 `Prepared`（拥有 `Layout`/`Timeline`/`Font`/`GlyphOutlines`，借用 `&'static Theme`），提供 `render_at(t)` / `render_into(t, &mut Pixmap)`；`render_preview` 与 `render_frames` 都改为基于它，去掉两份重复的构建代码。

### 2.3 ✅ 输入校验的四个缺口已闭合

> **已修复**：新增 `config::MAX_FPS`（1000）与 `config::MAX_CRF`（51），`validate()` 现在拒绝
> `fps == 0 || fps > 1000`、`crf > 51`、`end_pause` 为负或非有限，各自返回 `Error::FrameRate` /
> `Error::Crf` / `Error::EndPause`。5 条新断言覆盖。

| 输入 | 现状 | 后果 |
| --- | --- | --- |
| `--end-pause -1` | 通过校验 | `duration` 变负 → `frame_count` 被 `.max(1)` 兜成 **1 帧**，视频静默报废 |
| `--end-pause inf` / `NaN` | 通过校验 | 帧数溢出为 `u32::MAX`，渲染永不结束 |
| `--crf 60` | 通过校验 | x264 拒绝（`CRF 60 is not within the range`），错误来自 ffmpeg 而非本程序 |
| `--fps 100000` | 通过校验 | 每秒 10 万帧 × 时长，渲染量爆炸 |

- **建议**：`end_pause` 加有限且非负校验（新增 `Error::EndPause`）；`crf` 限制在 x264 的 `0..=51`；`fps` 加上限 `1000`（打字视频远超此值无意义），复用 `Error::FrameRate`。

### 2.4 ✅ `layout()` 热路径的三处小浪费（实测合计 -33%）

> **已修复**：`space_set` 整个删除（改用与 `preprocess` 相同的算术判定）；循环里只调用一次
> `font.glyph_index`，轮廓与 advance 共用；颜色解析加 1 项 per-token 缓存。
> **实测 1600 行 19.46 ms → 13.06 ms**，µs/char 0.247 → 0.166，线性性不变。

| 位置 | 现状 | 实测/估计 | 修法 |
| --- | --- | --- | --- |
| `layout()` 内 `space_set: HashSet<(usize, usize)>` | 每字符一次哈希查询 | ~6% | 删掉集合，直接 `*ch == ' ' && col >= first && col < last_exclusive`（与 `preprocess` 的判定完全一致） |
| `advance_of(font, ch)` | 每字符再查一次 `glyph_index`（同一字符在一次循环里被查 2 次） | ~5–10% | 循环里取一次 `glyph_index`，复用它查 advance |
| `Theme::color(kind)` | 每字符走 parent 链 × `styles` 线性扫描 | 6.6%（1600 行 1.29 ms） | 加一个"上一字符颜色"的 1 项缓存：同一 token 的连续字符命中率极高 |

### 2.5 ✅ 已排除的怀疑（实测证明不需要改动）

| 怀疑 | 测量 | 结论 |
| --- | --- | --- |
| 每帧遍历全部字形，大文件会拖慢帧 | 50 行 5.79 ms → 1600 行 6.60 ms（字形数 ×36） | 视口裁剪有效，帧成本与文件大小无关，**不改** |
| `GlyphOutlines` 的 `HashMap<u16, Path>` 查找是热点 | 每帧每可见字形一次哈希 ≈ 10–20 ns；相对 6.6 ms/帧 < 1% | 收益不抵改动面，**不改** |
| `Theme::color` 是 layout 的主要成本 | 1.29 ms / 19.46 ms = 6.6% | 只做 1 项缓存，不做查表重构 |

---

## 3. v2.0 遗留与后续候选（本轮不执行）

1. **CJK/emoji 字形回退**（v1 的 3.4）：需要多 `Face` 回退链 + glyph→face 归属；Windows 的 Segoe UI Emoji 是 COLR 彩色字体，轮廓路径跨平台不确定。当前行为已在 README 写明，缺字形会报出码点。
2. **Git 仓库与 CI**（v1 的 5.1/5.2）：用户明确暂缓。`cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test` 已可直接作为 CI 门槛。
3. ~~**辉光的真实观感验证**（v1 的 3.1）~~ —— **已完成（2026-10-07）**。本机 winget 包目录里其实装了
   ffmpeg 8.0（`%LOCALAPPDATA%\Microsoft\WinGet\Packages\Gyan.FFmpeg_*`，只是不在 PATH）。端到端验证
   发现原滤镜链会把整帧打成洋红（`eq` 与 `blend` 之间的像素格式协商问题），已修为在链首固定
   `format=gbrp`，并加了回归测试 `the_glow_filter_pins_its_pixel_format`。
4. **`repository` / `homepage`**：需要远端地址。
5. **`theme_data.rs` 生成器**：2136 行从 Pygments 导出的数据只有结果没有过程，重新生成需要重写导出脚本。
6. **criterion 基准**：当前用 `examples/bench.rs` 与临时探针，没有统计意义的回归防护。
7. **MSRV 1.85 实测**：本机只有 1.99，`rust-version = "1.85"` 未在真机上验证过。

---

## 4. 执行顺序（v2.0 本轮）

| # | 条目 | 成本 | 收益 | 状态 |
| --- | --- | --- | --- | --- |
| 1 | 2.3 输入校验补全 | 小 | 消除静默报废与 ffmpeg 报错 | ✅ 4 个新校验 + 5 条断言 |
| 2 | 2.4 layout 热路径三处 | 小 | layout 快 ~15% | ✅ 实测 **-33%**（19.46 → 13.06 ms / 1600 行） |
| 3 | 2.1 `PngSink` 并行编码 | 中 | **端到端 4–5×**（PNG 路径） | ✅ 实测 **5.7× / 4.5×**（480p / 1080p） |
| 4 | 2.2 `Prepared` 复用 pipeline | 中 | 预览从 O(帧数×重建) 降到 O(1) 重建 | ✅ `Prepared` + 共用构建路径 + 测试 |
| 5 | 验证与文档收尾 | 小 | fmt/clippy/test + 端到端复测 + CHANGELOG | ✅ 44 个测试全绿，CHANGELOG/README 已同步 |

> 状态图例：⏳ 待执行 · 🔄 进行中 · ✅ 已完成（附实测）

## 5. v2.0 执行结果（2026-10-07）

| 指标 | v2.0 起点 | 执行后 |
| --- | --- | --- |
| `cargo test --release` | 39 passed | **44 passed**（29 集成 + 8 encode + 3 font + 4 doctest） |
| 端到端 480p/15fps → PNG（790 帧） | 5.23 s（151 fps） | **0.94 s（836 fps）** |
| 端到端 1080p/30fps → PNG（117 帧） | 2.64 s（44 fps） | **0.55 s（213 fps）** |
| `layout()` 1600 行 / 78,763 字形 | 19.46 ms（0.25 µs/char） | **13.06 ms（0.17 µs/char）** |
| 单帧渲染 1080p | 6.6 ms | 6.6 ms（未变，不需要改） |
| `cargo fmt --check` / `clippy -D warnings` | 干净 | 干净 |

新增测试覆盖：输入校验 5 条断言、PNG 单线程/多线程产出逐字节等价、sink 线程数与 Drop 收尾、
`Prepared` 一次构建多帧渲染并与 `render_preview` 逐像素一致。

---

## 6. v2.1：配色重设计（2026-10-07）

**起因**：默认主题 `material` 的画面被认为不好看。

**诊断**（对比渲染同一帧后确认）：material（以及 one-dark 等一批编辑器主题）把**标点与运算符**
画成亮青色 `#89ddff`，于是括号、逗号、冒号与关键字抢视线，一行代码看着像撒了彩色纸屑。另一个问题是
`CURRENT_LINE_HIGHLIGHT`（`#333333`）、`LINE_NUMBER_COLOR`（`#808080`）、`CURSOR_COLOR`
（`#ffffff`）与主题无关地硬编码 —— 浅色主题下那条深灰高亮条根本不能用。

**做法**

1. 手写主题 **`midnight`**（定义在 `src/theme.rs`，不写进生成的 `theme_data.rs`）并设为默认：

   | 用途 | 色值 | 说明 |
   | --- | --- | --- |
   | 背景 / 前景 | `#10141F` / `#E6ECF7` | 深靛 + 雾蓝白，比 `#263238`/`#eeffff` 更沉、不刺眼 |
   | 关键字 | `#C4A7F0` | 紫 |
   | 函数名 | `#9CC7FF` | 蓝 |
   | 字符串 | `#A8E6B8` | 绿 |
   | 内置/类型/转义 | `#96E2E8` | 青 |
   | 类名 | `#F5D48E` | 金 |
   | 数字/常量 | `#FFC98A` | 橙 |
   | 错误 | `#FF8A90` | 只在出错时出现 |
   | 注释 | `#6E7A96` | 明显退后 |
   | **标点 / 运算符** | **`#98A3BC` / `#A5B0C8`** | **贴近前景，退到代码后面**（核心改动） |

   > **v2.1.1（同日）**：首版整体偏暗，反馈"文字亮度有点低"后把整套色值上调约 15–20%
   > （前景 `#C7D0E0` → `#E6ECF7`，画面最亮像素 194 → 218），色相与层次关系不变。

2. 行号、当前行高亮条、光标改为**从主题派生**（`Theme::line_highlight()` /
   `line_number()` / `line_number_active()` / `cursor()`，在背景与前景之间线性插值）：
   深色主题得到"略微提亮"的条，浅色主题自动变成"略微压暗"的条 —— 43 个 Pygments 主题一起受益。
3. 删除 4 个硬编码颜色常量（`config::CURRENT_LINE_HIGHLIGHT` / `LINE_NUMBER_COLOR` /
   `LINE_NUMBER_ACTIVE_COLOR` / `CURSOR_COLOR`）。

**验证**：`theme.rs` 新增 3 个单元测试（默认主题可被查找、UI 颜色随背景走且保持微妙、
标点的对比度低于关键字）；并排渲染 material 与 midnight 的同一帧人工比对（`frame_theme_compare.png`）。

**回退方式**：`--style material`（或任何 Pygments 主题）仍然是原来那套颜色。

---

## 7. v2.2：源码注释中文化（2026-10-07）

按需求把 `src/`、`tests/`、`examples/` 下 18 个 Rust 文件的**全部注释**（884 行：`//`、`///`、`//!`）
翻译成简体中文，代码与标识符保持原样。

**约束**（这些属于程序输出或可执行示例，一律不改）：

- doctest 围栏代码块（`api.rs` / `lib.rs` / `theme.rs` 等，会被 `cargo test` 执行）；
- `src/error.rs` 的 `#[error("...")]` 文案 —— 那是给用户看的错误信息；
- `src/main.rs` 的 `USAGE` / `EXAMPLE`；`src/encode.rs` 的 `GLOW_FILTER` 滤镜链；
- 技术名词保留英文（FrameSink、Pygments、ffmpeg、tiny-skia、HashMap、layout、timeline 等）。

**验证**：

- 翻译前后剥离注释后逐行对比 18 个文件 —— **所有非注释行逐字节一致**；
- `cargo fmt --check` / `cargo clippy --all-targets -D warnings` / `cargo test --release`
  （45 个测试：12 theme + 29 集成 + 4 doctest）全绿；
- 全文 grep 复查：无英文注释残留。

**未改动**：`README.md`、`CHANGELOG.md`、本清单等文档仍是英文（需求限定的是"代码里的注释"）。

---

## 8. v2.3：中文 README + 用法/参数补齐（2026-10-07）

`README.md` 重写为中文，并把用法与参数补全到可以直接当手册用：

- **参数详解按功能分组**（输入 / 外观 / 时间线相机 / 输出 / 编码 / 信息），每个参数给出默认值、
  取值范围、作用与副作用；末尾另附一张与 `--help` 对齐的完整速查表。
- **新增的实用内容**：ffmpeg 不在 PATH 时的三种解法（含 winget 装了但没进 PATH 的常见情况）、
  视频时长计算公式、`--interval` 下限 `1/fps` 的说明、渲染报告逐行解读、四个库用法配方
  （`render` / `render_preview` / `Prepared` / 自定义 `FrameSink` + 进度 + 取消）、
  常见问题 FAQ、已知限制、开发命令。
- **英文原版完整保留**为 `README.en.md`（工作区无版本控制，避免不可逆丢失），两份文档互相链接。
- **验证**：文中所有命令示例都实际执行过一遍 —— PNG 路径（`--width/--height`、`--style`、
  `--language c#`、`--interval 0.04:0.09`、`--snap-camera --seed`）与 MP4 路径
  （`--crf/--preset`、`--glow`）全部通过；并用脚本比对 README 提到的参数名与 `cvr --help` 完全一致。

> 说明：`README.md` 里的 Rust 示例不会被当作 doctest 编译（`lib.rs` 没有 `include_str!`），
> 但它们都是按实际 API 写的，参数与类型均已核对。

---

## 9. v2.4：发布准备与包名调整（2026-10-07）

**发现并修掉的发布阻塞问题**：`cargo publish --dry-run` 报出打包 **34 个文件 / 28.6 MiB
（压缩 22.0 MiB）** —— 演示产物 `demo-*.mp4` 与 `frame_*.png` 被一起打了进去，而 crates.io 的
压缩包上限是 **10 MiB**，上传必被拒。根因是**本目录不是 git 仓库，cargo 不读 `.gitignore`**。

修法：`Cargo.toml` 显式排除：

```toml
exclude = ["*.mp4", "frame_*.png", "preview.png", "frames/",
           "CodeVideoRenderer-rs-TODO.md", "docs/"]
```

打包回到 **27 files / 13.4 MiB（压缩 7.1 MiB）**。

**包名**：从 `codevideorenderer-rs` 改为 **`codevideorenderer`**（已用 `cargo info` 确认 crates.io
上未占用）。因此依赖声明简化为 `codevideorenderer = "0.1"`，不再需要 `package = ...` 重命名；
库名与二进制名不变（`codevideorenderer` / `cvr`）。**仓库/项目名保持 `CodeVideoRenderer-rs`**，
用来标识它是 Rust 移植版。

**新增**：`docs/RELEASING.md` —— 发布手册（检查清单、一次性准备、发布与发布后验证、体积预算、
不可逆性说明、常见报错对照表）。`docs/` 已在 `exclude` 内，不会随包发布。

**发布就绪状态**：`cargo publish --dry-run` 通过、名称可用、体积 7.1 MiB（余量约 30%）。
仍需外部输入：crates.io API token、`repository` / `homepage` 地址。
