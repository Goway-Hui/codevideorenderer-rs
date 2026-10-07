# CodeVideoRenderer-rs 待完善清单

> **目标项目**：`CodeVideoRenderer-rs`（Rust 重写版，位于 `D:\DevCode\MyProjects\CodeVideoRenderer-rs`）
> **生成日期**：2026-10-07
> **范围**：`src/`（13 个模块，约 5,400 行）、`tests/smoke.rs`、`examples/`、`Cargo.toml`、`README.md`
> **方法**：全量代码走查 + 实际构建/测试 + 端到端渲染 + 三个临时探针程序实测
> **环境**：Windows / rustc 1.99 / edition 2024 / AMD Ryzen 9 7945HX (16C/32T)

## 0. 基线：这些已经是健康的，不要回退

在动手改任何东西之前先记录当前的健康度，它是这份清单的对照基准：

| 检查项 | 结果 |
| --- | --- |
| `cargo test --release` | 21 passed / 0 failed，另 2 个 doctest 通过 |
| `cargo clippy --all-targets`（默认 lint 集） | 零警告 |
| 端到端 CLI（480p / 15fps / PNG 输出） | 790 帧，5.04 s，产物正确 |
| 直接依赖 | 4 个：`rayon` / `thiserror` / `tiny-skia` / `ttf-parser` |
| `cargo package` | 25 个文件，13.3 MiB → 7.0 MiB 压缩后，未超 crates.io 限制 |
| `unsafe` / 全局可变状态 | 库内为零 |

架构本身是成立的：不可变 `Layout` + 烘焙的 `CameraTrack` + 纯函数 `frame(t)`，确实做到了帧级并行。下面所有条目都是在这个基础上的增量完善，**没有需要推倒重来的结构性问题**。

---

## 1. 🔴 高优先级：文档已承诺、但主路径没有兑现

这四条的共同特征是——`README.md` 或模块文档写明了目标状态，代码里也有实现，但**没有接到真正的执行路径上**。它们会直接削弱项目最核心的卖点。

### 1.1 `api::render` 没有使用 `render_into_vec`，README 的性能建议自己没遵守

- **位置**：`src/api.rs:226-242`
- **现状**：主渲染循环每帧新建一个 `Pixmap`：

  ```rust
  let rendered: Vec<Pixmap> = (start..end)
      .into_par_iter()
      .map(|index| {
          let mut pixmap = Pixmap::new(opts.width, opts.height).expect("validated resolution");
          renderer.render(timeline.time_of_frame(index), &mut pixmap);
          pixmap
      })
      .collect::<Vec<_>>();
  ```

- **对照**：`README.md` 的 "Performance notes" 明确写着：

  > **Reuse frame buffers.** Allocating a fresh 1080p buffer per frame cost about **6× more** than rasterising into it (250 → 1,500 frames/s). Use `FrameRenderer::render_into_vec`.

- **证据**：全仓 grep 显示 `render_into_vec` **只在 `examples/bench.rs:91` 被调用**。也就是说 benchmark 跑的是优化路径，真实渲染跑的是未优化路径——README 里 1,500 fps 那个数字与用户实际得到的不是同一条流水线。

- **附带问题**：`batch = threads * 2`（`src/api.rs:222`），32 线程时单批瞬时占用 32 × 8 MB ≈ **256 MB**，且批内每个 buffer 都是新分配（不是复用）。

- **建议**：批内改为复用一个 per-thread buffer 池（`Vec<Vec<u8>>`，长度为 `threads`），用 `render_into_vec` 填充；把 `batch` 从 `threads * 2` 降到 `threads` 左右。改完用 `examples/bench.rs` 与一次真实渲染对比前后耗时。

### 1.2 每次 `render()` 泄漏一份字体，并重复 13.4 MB 的 IO 与解析

- **位置**：`src/font.rs:53-58`（泄漏点）、`src/font.rs:68-99`（`find`）、`src/api.rs:164` 与 `src/api.rs:312`（调用点）
- **现状**：

  ```rust
  pub fn load(path: &std::path::Path) -> Result<Self> {
      let data = std::fs::read(path)?;
      // Leak: one font per process, and `Face` borrows the data for `'static`.
      let leaked: FontBytes = Box::leak(data.into_boxed_slice());
      Self::from_static(leaked, path.display().to_string())
  }
  ```

  注释的前提是 "one font per process"，但 `Font` 既没有缓存也没有单例化，而 `render()` 与 `render_preview()` **每次被调用都会执行 `Font::find()`**。

- **实测**（探针程序循环调用 `Font::find(None)`，用 `(Get-Process -Id $PID).PrivateMemorySize64` 读数）：

  ```
  字体文件大小: 13.4 MB
  起始私有内存:   0.8 MB
   100 次后:     44.2 MB        ← 名义应增长 1.34 GB
  对照组 read+drop 40 次: 44.2 MB（无增长）
  ```

  实测增长被压到约 0.44 MB/次，是 Windows 对相同内容页做内存合并的结果——**这是操作系统在替这个缺陷兜底**。在 Linux / macOS 上会接近 13.4 MB/次。对照组确认了增长确实来自 `Box::leak` 而非读取本身。

- **影响**：即使溢出被平台缓解，**每次渲染重新读盘 13.4 MB + 重新 `Face::parse` + 重新 `GlyphOutlines::build` 也是纯浪费**，而这恰好打在 README 声称的 "trusted in batch use" 场景上。

- **建议**：加一个进程级字体缓存（`OnceLock<Mutex<HashMap<PathBuf, &'static Font>>>`，或更简单的针对 bundled 字体的 `OnceLock<Font>`），或者把 `Font` 的持有权上移到调用方，让 `render()` 接受 `&Font`。前者改动最小。

### 1.3 `layout()` 里仍有一个真实的 O(N²) 项，与核心卖点直接冲突

- **位置**：`src/layout.rs:394-403`（定义）、`src/layout.rs:284`（每字符调用一次）
- **现状**：

  ```rust
  // src/layout.rs:284
  let color = theme.color(highlighted.kind_at(char_index(&pre.lines, line_index, col)));

  // src/layout.rs:394
  fn char_index(lines: &[String], line: usize, col: usize) -> usize {
      let mut index = 0usize;
      for (i, l) in lines.iter().enumerate() {
          if i == line { return index + col; }
          index += l.chars().count() + 1; // + newline
      }
      index + col
  }
  ```

  `char_index` 每次都要从头累加所有行的字符数，却在**逐字符的循环体内**被调用 → 总代价 O(字符数 × 行数 × 行长) = **O(N²)**。

- **对照**：`README.md` 的 "Why a rewrite" 第 1 条是「消除 O(N²)」，`src/layout.rs:1-8` 的模块文档也写着 "linear rather than quadratic in the code length"。

- **实测**（临时 crate 直接调用 `layout::layout`，5 次取均值）：

  | lines | chars | layout ms | µs/char | 相对上一档 |
  | ---: | ---: | ---: | ---: | ---: |
  | 50 | 1,970 | 0.864 | 0.438 | — |
  | 100 | 3,970 | 2.704 | 0.681 | ×1.55 |
  | 200 | 8,270 | 9.350 | 1.131 | ×1.66 |
  | 400 | 16,870 | 32.439 | 1.923 | ×1.70 |
  | 800 | 34,070 | 117.651 | 3.453 | ×1.80 |
  | 1600 | 70,270 | 438.962 | 6.247 | ×1.81 |

  字符数 ×35.7，耗时 ×508；每字符成本随规模线性上升，是二次复杂度的确定特征。

- **建议**：`layout` 的外层遍历本身就是顺序的，维护一个自增的 `global_index` 即可，`char_index` 可以整个删掉。改动很小，收益随代码规模放大。

### 1.4 荒谬分辨率会让进程 abort，而不是返回错误

- **位置**：`src/api.rs:272-277`（`validate`）、`src/api.rs:232`（`.expect(...)`）
- **现状**：`validate` 只挡 `width == 0 || height == 0`，而 `.expect("validated resolution")` 也挡不住大尺寸。

- **实测**：

  ```console
  $ cvr --code 'x = 1' --resolution 100000x100000 --frames <tmp>
  memory allocation of 40000000000 bytes failed
  skipping backtrace printing to avoid potential recursion
  退出码: 127
  ```

  进程直接 abort，没有任何可读的 `Error::Resolution` 返回。CLI 用户敲错一个数量级就会看到内存分配失败的堆栈提示。

- **建议**：在 `validate()` 里对 `width * height` 设上限（例如 `≤ 7680 × 4320`，或一个独立的像素总数上限），返回 `Error::Resolution`；CLI 侧的 `parse_resolution`（`src/main.rs:264-289`）也可以先做一次范围检查，让错误在解析参数时就暴露。

---

## 2. 🟠 正确性与健壮性

### 2.1 语法高亮跑在被改写过的文本上，已实测出误判

- **位置**：`src/layout.rs:200-208`（改写）、`src/layout.rs:234`（高亮）、`src/layout.rs:282-284`（消费）
- **现状**：`preprocess` 把行内空格替换成占位符 `(`（沿用 Python 版保列宽的做法，见 `src/layout.rs:203`），随后 `layout()` 把**这个被改写过的 `pre.text`** 交给 lexer：

  ```rust
  let highlighted: Highlighted = lexer::highlight(&pre.text, language);
  ```

  也就是说 lexer 看到的不是用户的代码——`let x = foo(1, 2);` 实际传进去的是 `let(x(=(foo(1,(2);`。

- **实测**（对比同一段代码在「原始文本」与「污染文本」两种输入下的 token kind，只比较**非空格字符**）：

  ```
  [python] 差异 0    [python] 差异 0    [bash] 差异 0    [yaml] 差异 0
  [rust]   let x = foo(1, 2); // note: a comment
           idx 4 'x': Some(Name) -> Some(NameFunction)
  合计差异: 1
  ```

  `let` 后面被塞进 `(`，让 scanner 把变量 `x` 判成了函数名。

- **影响**：多数位置 token kind 不变（行内空格对应的 glyph 本来就 `invisible: true`，不绘制），但这是**可复现的正确性偏差**，与 `README.md` 中「Pygments-compatible token kinds … behave exactly like they do in the original」的表述不完全相符。含 `(` 语义的语言（shell / markdown / 正则密集的代码）风险更高。

- **建议**：高亮改用**未被替换的原文**；行内空格单独用已有的 `pre.inner_spaces` 集合在 layout 阶段处理，不需要污染 lexer 的输入。顺带可以删掉 `config::OCCUPY_CHARACTER` 及其注释里那句关于「保持两边 glyph 网格一致」的说法——渲染器根本不画这个占位符。

### 2.2 `validate()` 不检查 `is_infinite()`

- **位置**：`src/api.rs:281-286`
- **现状**：`line_spacing` 与 `camera_scale` 只查了 `<= 0.0 || is_nan()`，漏掉 `is_infinite()`。
- **后果**：`--camera-scale inf` 能通过校验，然后在 `src/render.rs:120-129` 算出 `k = width / (REFERENCE_WIDTH * inf) = 0`，整个 `world_to_screen` 变换退化为零矩阵，画面全空且不报错。
- **建议**：两个字段都补 `!is_finite()` 检查，并复用同一个 `Error` 分支。

### 2.3 `interval_min` / `interval_max` 的 NaN 检查顺序不理想

- **位置**：`src/api.rs:287-301`
- **现状**：先做 `<` 比较（NaN 参与比较恒为 `false`，不会命中），再检查 `is_finite()`。结果是对的，但读起来像是有意绕开 NaN。
- **建议**：把 `is_finite()` 检查提到比较之前，语义更直白。

### 2.4 CLI 打印的 fps 不是用户指定的值

- **位置**：`src/main.rs:204-209`
- **现状**：

  ```rust
  println!("  frames   {} @ {}fps ({:.1}s)",
      report.frames,
      report.frames as f32 / report.duration.max(1e-6),
      report.duration);
  ```

  第二项是 `frames / duration` 反算出的**平均帧率**。实测输出 `790 @ 15.004708fps`，而用户传的是 `--fps 15`。

- **根因**：`render(source, opts)` 消耗了 `opts`（见 `src/main.rs:192`），`main` 拿不到原始的 `fps`。
- **建议**：让 `RenderReport` 多带一个 `fps: u32` 字段（它已经有 `duration` 和 `frames`，加一个原值最省事），或者把 `render` 改成接受 `&RenderOptions`。

### 2.5 `main.rs` 里一个恒真的检查

- **位置**：`src/main.rs:194-196`
- **现状**：`if !report.elapsed.is_finite() { return Err("render produced no timing information") }`——`elapsed` 来自 `Instant` 差值，不可能非有限；而且检查发生在渲染**结束之后**，起不到任何保护作用。
- **建议**：删除。

### 2.6 字体搜索顺序的注释与实现不符

- **位置**：`src/font.rs:60-67`（文档注释列了 5 步）、`src/font.rs:85-89`（实现）
- **现状**：`#[cfg(feature = "embed-font")]` 分支是**无条件 `return`**：

  ```rust
  #[cfg(feature = "embed-font")]
  {
      let bytes: FontBytes = include_bytes!("../assets/CodeVideoRendererFont.ttf");
      return Self::from_static(bytes, "embedded CodeVideoRendererFont");
  }
  ```

  因此开启该 feature 时，注释里的第 5 步（系统字体）永远不可达。这大概是有意为之（内嵌字体必然存在），但注释没说清楚。
- **建议**：要么修正注释说明「启用 `embed-font` 后系统字体不再作为回退」，要么让内嵌分支只在真的解析失败时才走。

### 2.7 `FfmpegSink::spawn` 每次都要探测 ffmpeg

- **位置**：`src/encode.rs:96` → `Sink::ffmpeg_available`（`src/encode.rs:74-82`）
- **现状**：每次 spawn 都跑一次 `ffmpeg -version` 子进程。单次渲染无所谓，批处理时是重复开销。
- **建议**：把探测结果缓存起来（按 `ffmpeg` 路径作 key），或者干脆不预探测——直接 spawn，把 `io::ErrorKind::NotFound` 映射成 `Error::FfmpegMissing`，语义一样且少一次进程启动。

---

## 3. 🟠 功能缺口（与 Python 版的 parity）

### 3.1 辉光（glow）后处理未实现

- **现状**：README 已声明 "Not implemented yet"。这是 Python 版的标志性视觉效果（它强制开启，代价是两次编码）。
- **影响**：`README.md` 开头写的是 "The visual language is deliberately the same"，但缺了辉光，两边**输出观感并不一致**。
- **建议**：README 里已经给出了可直接用的 ffmpeg filter 链，接在 `FfmpegSink` 的 `-vf` 参数上即可，且仍是单次编码：

  ```
  -vf "split[a][b];[b]gblur=sigma=10,eq=brightness=0.06:saturation=2[a2];[a][a2]blend=all_mode=screen"
  ```

  建议做成 `--glow` 开关，默认可关（它显著增加编码时间）。

### 3.2 帧去重定义了但没有实现

- **位置**：`src/config.rs:90-93`
- **现状**：`DEFAULT_DEDUP_TOLERANCE` 定义了却**从未被引用**（全仓 grep 只有定义本身那一处）。它自己的注释写着：

  > How many identical consecutive frames may be collapsed into one encoded frame. A typed character every `interval` seconds means most rendered frames are identical, so this is a large, almost free win.

- **影响**：每字符 0.15 s 在 60 fps 下意味着**每个字符产生 9 帧完全相同的画面**，全部原样喂给 x264。这是当前最大的剩余优化空间——它比 1.1 的 buffer 复用收益更大，因为减少的是**编码器**的工作量，而编码已经是这条流水线的真正瓶颈。
- **建议**：`Timeline` 已经持有完整的 `reveal_times` 数组，因此不必逐帧比对像素——直接算出每个字符对应的重复帧区间，重复帧只编码一次（配合 `-vsync cfr` 或 `-r` 维持时长）。若不想改编码器交互，退一步也可以先在 `Sink` 层做重复帧比对。

### 3.3 语言覆盖 19 vs Pygments 600+，且降级时无任何提示

- **现状**：README 已声明 19 种手写语法 + generic fallback。列表见 `src/lexer.rs:59-64`。
- **问题**：`--language haskell` 不会报错，会**静默**退化成 generic scanner（`src/lexer.rs:51-56`），用户无从得知自己的代码只是"降级高亮"。
- **建议**：在 `highlight()` 或 CLI 层判断语言不在 `supported_languages()` 中时，向 stderr 输出一行 note（`--quiet` 时抑制）。行为保持宽容，但不再沉默。

### 3.4 CJK / emoji 字形回退未实现

- **现状**：README 已声明。实测 `examples/diag.rs` 输出：

  ```
  'M'  -> glyph 48      advance_em = 0.5498
  '中' -> glyph 9549    advance_em = 1.0000
  '演' -> glyph 17972   advance_em = 1.0000
  '🚀' -> missing
  ```

  内置字体覆盖 CJK，但 emoji 缺失 → `layout()` 返回 `Error::MissingGlyph` 直接失败。
- **建议**：要么接入系统字体回退（按 codepoint 分派到多个 `Face`），要么在 README 的错误说明里明确「emoji 不受支持，请预处理移除」。目前错误信息本身是可读的，属于「可接受但未完成」。

### 3.5 CJK 列宽不是 ASCII 的整数倍（未记录）

- **位置**：`src/layout.rs:326-331`（逐字符累加 advance）
- **现状**：ASCII 字符 advance = 0.5498 em，CJK 字符 = 1.0000 em，比值 1.82 而非 2。光标位置靠逐字符累加所以是正确的，但**含中文的行与纯 ASCII 行的列不会对齐**。
- **影响**：README 与 `src/layout.rs` 都强调 "monospace grid"，而这条在混合中英文时不成立。Python 版有同样的行为（都按字体实际 advance 排版），所以不是回归，但值得在 README 里补一句说明。

---

## 4. 🟡 库 API 的扩展点不足

如果这个 crate 打算被当作库用于批处理 / 服务化（README 的 "trusted in batch use" 指向的正是这个场景），以下几处会很快成为阻塞点。

### 4.1 `Sink` 是封闭枚举

- **位置**：`src/encode.rs:49-54`
- **现状**：`pub enum Sink { Ffmpeg(Box<FfmpegSink>), PngSequence(PngSink) }`——调用方无法注入自定义输出目标（推流、写内存、上传对象存储）。
- **建议**：抽一个 `trait FrameSink { fn write(&mut self, index: u32, pixmap: &Pixmap) -> Result<()>; fn finish(self: Box<Self>) -> Result<String>; }`，内置两个实现，`RenderOptions` 增加 `sink: Option<Box<dyn FrameSink>>` 或提供 `render_with_sink()`。

### 4.2 没有进度回调

- **位置**：`src/encode.rs:200-248`（`Progress`）、`src/api.rs:223`（唯一使用点）
- **现状**：`Progress` 只往 stdout 打印，库调用方拿不到任何进度信息，长渲染期间无法给用户反馈。
- **建议**：`RenderOptions` 加一个 `progress: Option<Arc<dyn Fn(u32, u32) + Send + Sync>>`，内部同时驱动它和内置 `Progress`。

### 4.3 没有取消 / 中止机制

- **现状**：`render()` 一旦开始就只能等它结束，批处理里无法中断一个跑偏的任务。
- **建议**：传入一个 `Arc<AtomicBool>` 令牌，在批循环边界检查（`src/api.rs:226` 的 `for start in ...` 每轮检查一次，粒度足够）。

### 4.4 `RenderOptions` 的 builder 覆盖不全

- **位置**：`src/api.rs:106-126`
- **现状**：只有 `with_fps` / `with_resolution` / `with_interval` 三个，且**全仓无人使用**（包括 README 示例和测试）。其余十几个字段只能靠结构体更新语法设置。
- **建议**：要么补齐成完整 builder，要么删掉这三个避免半成品 API；当前 `RenderOptions { ..Default::default() }` 已经够用，保留三个不完整的便利方法反而增加维护面。

---

## 5. 🟡 工程化与发布

### 5.1 不是 git 仓库

- **现状**：目录里没有 `.git`。有 README、LICENSE、测试、示例，唯独没有版本历史。
- **建议**：`git init` + 一次初始提交。5.2 的 CI 也依赖它。

### 5.2 没有 CI

- **现状**：没有 `.github/`。对比：Python 版有覆盖 7 个 Python 版本 × pytest 的 workflow，Rust 版连 `cargo test` 的自动化都没有。
- **建议**：最小可用流水线 = `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`，矩阵跑 `stable` 与 `1.85`（`rust-version` 声明的 MSRV）。当前 clippy 零警告，正好可以直接上 `-D warnings`。

### 5.3 `Cargo.toml` 缺少发布元数据

- **现状**：`cargo package` 明确警告：

  ```
  warning: manifest has no documentation, homepage or repository
  ```

  另外 `keywords` / `categories` 也缺失。
- **建议**：补 `repository`、`homepage`、`keywords = ["video", "code", "animation", "render", "syntax-highlighting"]`、`categories = ["multimedia::video", "command-line-utilities"]`。

### 5.4 打包体积已接近 crates.io 限制

- **实测**：`cargo package` 产出 25 个文件，13.3 MiB 未压缩 → **7.0 MiB 压缩后**（`codevideorenderer-rs-0.1.0.crate`，7,365,628 字节）。字体 `assets/CodeVideoRendererFont.ttf` 占了约 98%。
- **结论**：**未超**限制，可以发布，但余量不大。
- **建议**：评估是否把字体拆成独立的 `codevideorenderer-fonts` crate，或用 `exclude` 排除、由用户在运行时提供。若维持现状，至少在 README 里说明体积构成。

### 5.5 没有 CHANGELOG

- **现状**：`Cargo.toml` 版本还是 `0.1.0`，没有 `CHANGELOG.md`。
- **参考**：Python 版的 `docs/changelog.rst` 是它文档体系里做得最好的部分之一（按 Additions / Changes / Deletions 分类），值得沿用同一格式。

### 5.6 README 的 Options 表不全

- **现状**：CLI 实际支持 `--code` / `--code-file` / `--name` / `--output` / `--width` / `--height` / `--ffmpeg` / `--list-styles` / `--list-languages`，但 README 的 "Options" 表格里都没有（正文 Usage 段只出现了一部分）。
- **建议**：至少补上 `--width` / `--height` / `--output`，或直接把该表换成 `cvr --help` 的输出。

### 5.7 `.gitignore` 漏了示例输出

- **现状**：忽略了 `*.mp4` / `/frames/` / `/frames_test/` / `/frame_*.png`，但 `examples/preview.rs` 的默认输出是当前目录下的 `preview.png`（`examples/preview.rs:18`），未被忽略。
- **建议**：补一行 `preview.png`，或让示例默认输出到临时位置。

---

## 6. 🟡 死代码 / "计划了但没接线"的痕迹

以下符号经全仓 grep（含 `examples/` 与 `tests/`）确认**除定义外零引用**。不是错误，但说明有几条线拉了一半，会让后来者误以为这些功能已经生效：

| 符号 | 位置 | 说明 |
| --- | --- | --- |
| `DEFAULT_DEDUP_TOLERANCE` | `src/config.rs:93` | 帧去重，见 3.2——**有明确收益，建议实现而不是删除** |
| `CODE_BACKGROUND` | `src/config.rs:64` | 渲染实际用的是 `theme.background`（`src/render.rs:111`），这个常量永远不会生效 |
| `Font::bytes()` | `src/font.rs:146` | — |
| `Timeline::seconds_per_frame()` | `src/timeline.rs:186` | — |
| `Timeline::reference_width()` | `src/timeline.rs:191` | — |
| `Theme::is_dark()` | `src/theme.rs:29` | — |
| `FrameRenderer::glyph_count()` | `src/render.rs:291` | `RenderReport.outlines` 用的是 `GlyphOutlines::len()` |
| `RenderOptions::with_fps` / `with_resolution` / `with_interval` | `src/api.rs:108/114/121` | 见 4.4 |

---

## 7. 建议的修复顺序

按「收益 / 成本」排序。前四条都是小改动且收益直接，建议作为第一批：

| # | 条目 | 预估成本 | 收益 |
| --- | --- | --- | --- |
| 1 | 1.1 接入 `render_into_vec` | 小 | 直接、可测量 |
| 2 | 1.2 `Font` 进程级缓存 | 小 | 批处理场景的 IO 与内存 |
| 3 | 1.3 移除 `char_index` 二次项 | 小 | 大规模代码的 layout 耗时 |
| 4 | 1.4 + 2.2 分辨率上限与 `is_finite` 校验 | 小 | 消除进程 abort |
| 5 | 3.2 帧去重 | 中 | **当前最大的剩余性能优化** |
| 6 | 2.1 高亮改用未污染文本 | 中 | 正确性 |
| 7 | 4.1–4.3 进度回调 / 自定义 Sink / 取消 | 中 | 决定它能否当库用 |
| 8 | 5.1 + 5.2 `git init` + CI | 小 | 长期可维护性 |
| 9 | 3.1 辉光后处理 | 中 | 与 Python 版的视觉 parity |
| 10 | 3.3 / 5.3 / 5.5 / 5.6 文档与元数据 | 小 | 使用体验 |

---

## 附录 A：如何复现本文的实测结论

### A.1 基线

```console
cargo test --release
cargo clippy --all-targets
./target/release/cvr.exe --code-file examples/fibonacci.py --frames <tmp>/frames \
    --resolution 480p --fps 15 --quiet
```

### A.2 分辨率 abort（对应 1.4）

```console
./target/release/cvr.exe --code 'x = 1' --resolution 100000x100000 --frames <tmp> --quiet
# 期望：进程 abort，退出码 127
```

### A.3 `layout()` 的二次增长（对应 1.3）

建一个临时 crate，`Cargo.toml` 里 `codevideorenderer = { package = "codevideorenderer-rs", path = "<repo>" }`，然后用递增行数调用 `layout::layout` 并计时。判据是 **µs/char 随规模单调上升**——修复后它应当基本持平。

### A.4 高亮污染（对应 2.1）

同一段代码分别用原文和 `preprocess(...).text` 喂给 `lexer::highlight`，逐字符比较非空格位置的 `TokenKind`。当前 `let x = foo(1, 2);` 会在 `x` 处产生 `Name -> NameFunction` 的差异。

### A.5 字体泄漏（对应 1.2）

循环调用 `Font::find(None)`，用 `(Get-Process -Id $PID).PrivateMemorySize64` 观察私有内存。对照组：循环 `std::fs::read` 同一个字体文件并 drop，应当**无增长**。

---

## 附录 B：环境观察

- 分析所在 shell 的 `PATH` 里**没有 `ffmpeg`**，因此 README 默认的 MP4 路径未能验证，上述端到端测试走的是 `--frames` PNG 分支。若在交互式 shell 中 ffmpeg 可见，建议补一次 MP4 路径的实测。
- 本文所有结论基于当前工作区快照。该目录**无版本历史**，因此无法标注对应的 commit。
