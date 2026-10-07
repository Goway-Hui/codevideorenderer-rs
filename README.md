# CodeVideoRenderer-rs

用 Rust 渲染**打字代码视频**：给一段代码，逐字符地动画呈现，相机跟随光标移动，
最后编码成 MP4。

本项目是 [CodeVideoRenderer](https://github.com/ExploreMaths/CodeVideoRenderer)（Python + Manim）
的从零重写的 Rust 版本。视觉语言刻意保持一致 —— 等宽字符网格、行号、当前行高亮条、光标、
相机跟随与自动拉远、手持相机晃动 —— 但底层流水线做了重构，因此它能够
**（a）用满所有 CPU 核心，（b）在批处理场景下放心使用**。

> English documentation: [README.en.md](README.en.md)

```console
$ cvr --code-file examples/fibonacci.py --style midnight
cvr 0.1.0: python · midnight · 1920x1080 @ 60fps
  font     assets/CodeVideoRendererFont.ttf
  theme    midnight
  layout   16 lines · 311 typed characters · 52 glyph outlines
  frames   3160 @ 60fps (52.7s)
  render   24.34s (130 frames/s on 32 threads)
  reused   59 of 3160 frames repeated the previous image
  output   CameraFollowCursorCV.mp4
```

---

## 特性一览

- **帧级并行**：一帧只取决于时间 `t`，因此所有帧在所有核心上并行渲染。
- **单次编码**：帧直接流入一个 ffmpeg 进程，没有中间 MP4、没有解码、没有二次处理。
- **重复帧零成本**：与上一帧画面完全相同的帧不会被重新光栅化。
- **纯函数管线**：布局只算一次，帧是查表而不是重建 —— 复杂度对代码长度线性。
- **无全局状态**：所有参数都在 `RenderOptions` 里，同进程内多次渲染互不干扰。
- **两种输出**：MP4（需要 ffmpeg）或 PNG 序列（不需要任何外部工具）。
- **可编程**：`FrameSink` trait 可以把帧送到任何地方，支持进度回调与协作式取消。
- **手写默认配色**：`midnight` 主题为「屏幕上只有代码」这一场景设计。

---

## 为什么重写

Python 版能跑，但有四个**结构性**（而非表面）的问题，都已在它源码里确认过：

1. **二次方渲染**。它为每个键入字符调用一次 `scene.play()`，而 Manim 每次都会重建整个
   「静态层」；又因为相机在动、场景里的对象是静态的，每一帧都要把**此前所有字符**重新
   光栅化一遍。N 个字符的总工作量是 O(N²) —— 500 个字符约 125,000 次字形绘制。
2. **串行帧**。Manim 的帧循环完全没有多线程，每个字符对应的约 9 帧是一帧一帧渲染并写出的。
3. **两轮编码**。Manim 先写出 MP4，MoviePy 再解码、用 PIL 对每帧做模糊、然后重新编码；
   中途还会产生数百个分片视频文件（每个 `play()` 一个）。
4. **全局状态**。参数存在模块级类里，于是第二个 `CameraFollowCursorCV` 实例会篡改第一个
   实例的参数；Manim 的全局配置在构造时被改写、只有渲染成功才恢复；输出路径用硬编码的
   Windows 分隔符拼接。

本 crate 把这四点全部结构性消除，**没有一行是逐行移植**。

## 架构

```text
Source ──preprocess──▶ text ──lexer──▶ 逐字符 token kind
                                    │
                                    ▼
                              layout：字形 + 位置（只算一次）
                                    │
                                    ▼
                        timeline：揭示时刻 + 烘焙好的相机关键帧
                                    │
                      frame(t) = 关于时间的纯函数
                                    │
                    rayon ──▶ 有界批次 ──▶ 单个 ffmpeg 管道
```

- **一次布局**。字形位置、颜色、揭示顺序只计算一次；渲染一帧是查表而不是重建，
  O(N²) 项被彻底移除。
- **纯函数帧**。相机被烘焙成关键帧、用二分查找采样，因此 `render_frame(t)` 不触碰任何共享
  可变状态，帧可以并行渲染；帧按有界批次产出，内存占用保持平稳。
- **轮廓按字体单位缓存**。字形轮廓与缩放无关，只提取一次，然后按当前相机倍率重新填充 ——
  相机拉远时文字依然锐利（原版靠 Cairo 拿到的同一条保证）。
- **单次编码**。帧流入一个 `ffmpeg` 进程，没有中间 MP4、没有解码、没有逐帧 PIL 处理。
- **没有全局变量**。所有状态都在 `RenderOptions` / `Layout` / `Timeline` 里，同进程的两次
  渲染不可能互相干扰。

| 模块 | 职责 |
|---|---|
| `config` | 与原项目常量对应的默认值 |
| `error` | 所有失败模式，均为强类型错误 |
| `font` | 字体查找、度量、轮廓提取（`ttf-parser`） |
| `lexer` | 手写扫描器 → Pygments 兼容的 token kind |
| `theme` / `theme_data` | 43 套 Pygments 风格 + 手写的 `midnight` 默认主题 |
| `layout` | 预处理 + 等宽网格排版 |
| `camera` | 关键帧相机：入场、跟随、自动拉远、晃动 |
| `timeline` | 揭示时刻、换行、总帧数 |
| `render` | 光栅化（`tiny-skia`） |
| `encode` | ffmpeg 管道或 PNG 序列，以及进度显示 |
| `api` | `render(Source, RenderOptions) -> RenderReport` |

---

## 安装

### 前置要求

- **Rust 1.85+**（使用 edition 2024）
- **ffmpeg**（可放在 `PATH` 里，或通过 `--ffmpeg <路径>` 指定）
  - 不想装 ffmpeg 也可以：用 `--frames <目录>` 直接输出 PNG 序列，**无需任何外部工具**

### 从源码构建

```console
$ cargo build --release
$ ./target/release/cvr --help
```

把内置字体编进二进制（单文件分发）：

```console
$ cargo build --release --features embed-font
```

### ffmpeg 不在 PATH 里怎么办

程序报 `ffmpeg was not found in PATH` 时，按优先级有三种做法：

1. **装到 PATH**：

   ```console
   # Windows（winget / scoop / choco 任选）
   $ winget install Gyan.FFmpeg
   # macOS
   $ brew install ffmpeg
   # Debian / Ubuntu
   $ sudo apt install ffmpeg
   ```

2. **直接指定路径**（推荐给「装了但没进 PATH」的情况）：

   ```console
   $ cvr --code-file script.py --ffmpeg "C:\path\to\ffmpeg.exe"
   ```

   > Windows 上用 winget 安装的 ffmpeg 常常落在
   > `%LOCALAPPDATA%\Microsoft\WinGet\Packages\Gyan.FFmpeg_*` 下而没有自动加入 `PATH`，
   > 这时用 `--ffmpeg` 指过去即可，不必改系统环境变量。

3. **完全绕开 ffmpeg**：

   ```console
   $ cvr --code-file script.py --frames frames/out
   ```

### 体积说明

发布到 crates.io 的包约 13 MiB（压缩后约 7 MiB），其中内置字体
`assets/CodeVideoRendererFont.ttf` 占了约 98%。这是「自带字体」的代价：
中文代码开箱即用，只有想换字体时才需要 `--font`。

---

## 快速开始

```console
# 1. 用内置示例渲染一段视频（默认 1080p / 60fps / MP4）
$ cvr

# 2. 渲染你的一段代码
$ cvr --code 'print("hello, world")'

# 3. 从文件读，换主题和语言
$ cvr --code-file script.py --language python --style github-dark --name Demo
```

输出解读：

| 行 | 含义 |
|---|---|
| `font` / `theme` | 实际使用的字体与主题 |
| `layout` | 行数、参与打字的字符数、缓存的字形轮廓数 |
| `frames` | 总帧数与帧率、视频时长 |
| `render` | 墙钟耗时与吞吐（帧/秒）、使用的线程数 |
| `reused` | 与上一帧画面相同、因而未重新光栅化的帧数 |
| `output` | 产物路径 |

---

## 命令行用法

### 按场景的示例

```console
# —— 最简 ——
$ cvr --code 'print("hello")' --name Hello

# —— 从文件读，指定语言 ——
$ cvr --code-file algorithm.py --language python --name Algorithm

# —— 换主题（44 种可选，见 --list-styles）——
$ cvr --code-file script.py --style dracula
$ cvr --code-file script.py --style nord

# —— 打字节奏：快一点、并带随机抖动 ——
$ cvr --code-file script.py --interval 0.04:0.09

# —— 大段代码：相机拉远一些，结尾多停一会儿 ——
$ cvr --code-file big.py --camera-scale 0.35 --end-pause 3 --name BigDemo

# —— 输出规格 ——
$ cvr --code-file script.py --resolution 720p --fps 30
$ cvr --code-file script.py --resolution 2560x1440 --fps 60
$ cvr --code-file script.py --width 1600 --height 900

# —— 只需要每一帧图片（不需要 ffmpeg）——
$ cvr --code-file script.py --frames frames/out

# —— 编码参数 ——
$ cvr --code-file script.py --crf 14 --preset slow --output final.mp4

# —— 加上原版的辉光后期（编码会慢一些）——
$ cvr --code-file script.py --glow

# —— 更快的渲染：相机在字符之间静止，大部分帧可复用 ——
$ cvr --code-file script.py --snap-camera

# —— 可复现：同一个 seed 得到同一个视频 ——
$ cvr --code-file script.py --seed 42

# —— ffmpeg 不在 PATH 时 ——
$ cvr --code-file script.py --ffmpeg "C:\tools\ffmpeg\bin\ffmpeg.exe"

# —— 查看可选值 ——
$ cvr --list-styles
$ cvr --list-languages
$ cvr --help
```

### 参数详解

#### 输入（二者择一）

| 参数 | 默认 | 说明 |
|---|---|---|
| `--code <TEXT>` | 内置示例 | 直接传入代码文本；注意加引号，含换行时用 shell 的引号语法 |
| `--code-file <PATH>` | — | 从文件读取（必须是 UTF-8）|

两者同时给出会报错；都不给则渲染内置示例。读取的代码会被预处理：
展开 tab（宽度 4）、去掉首尾空白行、保留中间空行，并拒绝 `\r`、`\v`、`\f`
（它们会破坏文本排版，请先替换成空格或换行）。

#### 外观

| 参数 | 默认 | 说明 |
|---|---|---|
| `--language <NAME>` | `python` | 语法高亮语言。支持 19 种（见下），支持别名如 `py` / `rs` / `js` / `c#`；**不在列表里不会报错**，会退化成通用高亮并在 stderr 提示 |
| `--style <NAME>` | `midnight` | 配色主题：内置的 `midnight`，或 43 套 Pygments 风格（如 `github-dark`、`monokai`、`dracula`、`nord`、`one-dark`、`solarized-dark`、`gruvbox-dark`…）。名称大小写不敏感，`_` 与 `-` 等价 |
| `--font <PATH>` | 内置 → 系统 | 指定等宽 TTF/OTF。不指定时的查找顺序：`--font` → 可执行文件旁的 `assets/CodeVideoRendererFont.ttf` → 当前目录的同上 → （启用 `embed-font` 时）内置字体 → 常见系统等宽字体 |
| `--line-spacing <F>` | `0.8` | 行间距，单位是「字号的倍数」。调大可让画面更舒展，调小更紧凑 |

支持的语言（19 种）：

```text
python  javascript  typescript  rust  go  c  cpp  csharp  java
kotlin  swift  php  ruby  bash  json  yaml  toml  markdown  sql
```

#### 时间线与相机

| 参数 | 默认 | 说明 |
|---|---|---|
| `--interval <MIN[:MAX]>` | `0.15` | 相邻两个字符的间隔秒数。写单个值则无抖动；写 `MIN:MAX` 则在区间内随机取值（由 `--seed` 决定）。**下限是 `1/fps`**，见下文 |
| `--camera-scale <F>` | `0.5` | 初始视野大小，**值越小画面越近**。相机只会自动拉远（跟随光标右移），永不推近 |
| `--snap-camera` | 关 | 相机在字符之间保持静止，而不是平滑滑动。牺牲一点顺滑感，换来大部分帧可直接复用（实测光栅化快 4.6–8.6 倍）|
| `--end-pause <F>` | `1.0` | 代码打完后画面停留的秒数（必须 ≥ 0 且有限）|
| `--seed <N>` | 固定值 | 字符间隔抖动的随机种子。同一个种子 + 同样的参数 = 逐字节相同的视频 |

**视频时长怎么算**

```text
时长 ≈ 入场动画(1.0s) + Σ(各字符间隔) + 0.4s × 换行次数 + 结尾停留(end_pause)
帧数 = ceil(时长 × fps)
```

例：311 个字符、默认 `--interval 0.15`、16 行，实测时长约 52.7 秒。想缩短视频，
优先调小 `--interval`，其次调小 `--end-pause`。

**`--interval` 的下限**

一个字符不可能比一帧还快，所以最小值是 `1/fps`：60fps 时是 16.7 ms。校验会在渲染前
完成并明确报出约束（`interval_range must satisfy 0.0167 <= min <= max ...`），
而不是渲染到一半才失败。

#### 输出

| 参数 | 默认 | 说明 |
|---|---|---|
| `--output <PATH>` | `<name>.mp4` | MP4 的完整路径。父目录不存在会自动创建 |
| `--name <STEM>` | `CameraFollowCursorCV` | 输出名（也用作默认的 MP4 文件名） |
| `--frames <DIR>` | — | 改为输出 PNG 序列 `frame_00000.png`… 到该目录，**不需要 ffmpeg**。给了这个参数时 `--output` 被忽略 |
| `--resolution <SPEC>` | `1080p` | `1080p` / `720p` / `480p` / `1440p`(2k) / `2160p`(4k)，或显式 `WxH`，如 `1920x1080`。上限 8K（33177600 像素），越界会在参数解析阶段报错 |
| `--width <N>` `--height <N>` | — | 显式指定像素尺寸，等价于 `--resolution WxH` |
| `--fps <N>` | `60` | 帧率，范围 `1..=1000` |

> `--resolution` 与 `--width`/`--height` 同时给出时，后者先被设置、前者再覆盖它。

#### 编码（MP4 输出）

| 参数 | 默认 | 说明 |
|---|---|---|
| `--crf <N>` | `18` | x264 质量，**值越小画质越好、文件越大**。范围 `0..=51`，常用 `18`（视觉无损）～`23`（体积小） |
| `--preset <NAME>` | `veryfast` | x264 预设：`ultrafast` / `superfast` / `veryfast` / `faster` / `fast` / `medium` / `slow` / `slower` / `veryslow`。越慢压缩率越高 |
| `--glow` | 关 | 加上原版的辉光后期效果。它是一条 ffmpeg 滤镜链，仍然**只编码一次**；因为逐帧处理，编码会明显变慢（实测约 1.8 倍）。默认关闭 |
| `--ffmpeg <PATH>` | `ffmpeg` | ffmpeg 可执行文件名或路径 |

#### 信息

| 参数 | 说明 |
|---|---|
| `--list-styles` | 列出全部 44 套配色主题名 |
| `--list-languages` | 列出 19 种内置语法 |
| `-h`, `--help` | 显示帮助 |
| `-V`, `--version` | 显示版本 |
| `--quiet` | 关闭进度条与提示信息（`stdout` 的最终报告仍会打印）|

### 完整速查表

| 参数 | 默认值 |
|---|---|
| `--code <TEXT>` / `--code-file <PATH>` | 内置示例 |
| `--language <NAME>` | `python` |
| `--style <NAME>` | `midnight` |
| `--font <PATH>` | 内置 → 系统 |
| `--line-spacing <F>` | `0.8` |
| `--interval <MIN[:MAX]>` | `0.15` |
| `--camera-scale <F>` | `0.5` |
| `--snap-camera` | 关 |
| `--end-pause <F>` | `1.0` |
| `--seed <N>` | 固定值 |
| `--output <PATH>` | `<name>.mp4` |
| `--name <STEM>` | `CameraFollowCursorCV` |
| `--frames <DIR>` | — |
| `--resolution <SPEC>` | `1080p` |
| `--width <N>` / `--height <N>` | — |
| `--fps <N>` | `60` |
| `--crf <N>` | `18` |
| `--preset <NAME>` | `veryfast` |
| `--glow` | 关 |
| `--ffmpeg <PATH>` | `ffmpeg` |
| `--quiet` | — |
| `--list-styles` / `--list-languages` / `-h` / `-V` | — |

---

## 库用法

把它当库用（`codevideorenderer`）：

```toml
[dependencies]
codevideorenderer = "0.1"
```

### 渲染一个视频

```rust
use codevideorenderer::{render, RenderOptions, Source};

// builder 风格与直接设字段都行
let options = RenderOptions::default()
    .with_language("rust")
    .with_style("github-dark")
    .with_fps(30)
    .with_interval(0.08, 0.20);

let report = render(Source::file("script.rs"), options)?;
println!("{} 帧，耗时 {:.1}s → {}", report.frames, report.elapsed, report.output);
# Ok::<(), codevideorenderer::Error>(())
```

`RenderReport` 会告诉你：`frames`、`duration`、`fps`、`output`、`elapsed`、
`typed_chars`、`lines`、`font`、`theme`、`outlines`、`threads`、
`frames_per_second`、`reused_frames`。

### 只画一帧（不编码）

```rust
use codevideorenderer::{render_preview, RenderOptions, Source};

let options = RenderOptions::default();
let pixmap = render_preview(Source::text("x = 1"), &options, 2.0)?;
pixmap.save_png("preview.png").unwrap();
# Ok::<(), codevideorenderer::Error>(())
```

### 一次构建、多帧渲染

`render_preview` 每次调用都会重建布局与轮廓缓存。如果要连续取很多帧（时间轴预览、
批量缩略图、逐帧测试），用 `Prepared` 只构建一次：

```rust
use codevideorenderer::{Prepared, RenderOptions, Source};

let options = RenderOptions::default().with_fps(30);
let prepared = Prepared::new(&Source::file("script.py"), &options)?;
let timeline = prepared.timeline();

for frame in 0..timeline.frame_count {
    let t = timeline.time_of_frame(frame);
    let pixmap = prepared.render_at(t)?;
    // ……保存、比对、显示
    let _ = pixmap;
}
# Ok::<(), codevideorenderer::Error>(())
```

`Prepared` 还提供 `layout()`、`font()`、`theme()`、`outlines()`、`size()`
与 `renderer(w, h)`（可以换一个输出尺寸光栅化）。

### 自定义输出目标、进度与取消

`FrameSink` 是「把帧送去哪里」的抽象。内置实现有 ffmpeg 管道与 PNG 序列，
你也可以自己实现，把帧送到对象存储、socket、或者自己的编码器：

```rust
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use codevideorenderer::tiny_skia::Pixmap;
use codevideorenderer::{
    Error, FrameSink, ProgressCallback, RenderOptions, Source, render_with_sink,
};

/// sink 只需要按顺序接收帧，并负责收尾。
struct Uploader { bytes: usize }

impl FrameSink for Uploader {
    fn write(&mut self, _index: u32, pixmap: &Pixmap) -> Result<(), Error> {
        self.bytes += pixmap.data().len();
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<String, Error> {
        Ok(format!("uploaded {} bytes", self.bytes))
    }
}

let cancel = Arc::new(AtomicBool::new(false));

let options = RenderOptions::default()
    .with_progress(ProgressCallback::new(|done, total| {
        eprintln!("{done}/{total} 帧");
    }))
    .with_cancel(Arc::clone(&cancel));

// 在另一个线程里把 cancel 置为 true，渲染会在下一个批次边界停止，
// 返回 Error::Cancelled 而不是报告。
let report = render_with_sink(Source::file("script.py"), options, Box::new(Uploader { bytes: 0 }))?;
println!("{}", report.output);
# Ok::<(), codevideorenderer::Error>(())
```

### 主要类型

| 类型 | 用途 |
|---|---|
| `Source` | 输入：`Source::text(..)` 或 `Source::file(..)` |
| `RenderOptions` | 全部渲染参数（20 个 `with_*` builder 方法）|
| `RenderReport` | 渲染结果与统计 |
| `Prepared` | 构建一次、多帧复用的渲染器 |
| `FrameSink` | 自定义输出目标的 trait |
| `ProgressCallback` | 逐帧进度回调 |
| `Error` | 全部失败模式（强类型）|

---

## 配色

默认主题 `midnight` 是**手写**的，不从 Pygments 导出。理由是：编辑器配色要在工具栏、
标签页、行号槽旁边工作，而代码视频的屏幕上除了代码什么都没有，二者的取舍不同。

设计取舍：

- **标点与运算符贴近前景色**，让括号、逗号不再与代码抢视线。Material、One Dark 把标点涂成
  亮青色，一行代码看着像撒了彩纸屑 —— 这是 `midnight` 与原版配色最大的观感差异。
- **六种克制的色相，各司其职**：紫色关键字、蓝色函数名、绿色字符串、青色内建与类型、
  琥珀色类名与数字，以及只在出错时出现的柔和红色。
- **没有颜色完全饱和，背景压得很深**，因此较亮的颜色过一遍视频压缩后依然清晰，而不会发光。
- **行号、当前行高亮条、光标由主题推导**（在背景与前景之间插值），不是硬编码 ——
  这样它们在所有主题里都协调，包括浅色主题（那里固定灰色横条会完全不可见）。

```console
$ cvr --list-styles          # 查看全部 44 套
$ cvr --code-file script.py --style monokai
```

---

## 已知限制

- **没有 CJK/emoji 字形回退**。内置字体覆盖中日韩文字；系统字体不一定。遇到字体里没有的
  字符会返回明确的错误并报出码点（而不是静默画成空白方框）。emoji 通常不被支持。
- **列对齐是按字体度量算的，不是按单元格**。中文（内置字体里 1.0 em）不是 ASCII 单元格
  （0.55 em）的整数倍，因此中英文混排的行不会逐列对齐。游标位置仍然正确，因为它累加的是
  同一套 advance。
- **只有 19 种语法**。其它语言不会报错，而是退化成通用高亮，并在 stderr 打印一行提示。
- **辉光的实际观感未做主观校验**。滤镜链已在 ffmpeg 8.0 上验证过颜色正确，但效果强度
  是沿用原版的参数。

---

## 性能

渲染成本正比于「帧数 × 可见字形数」，而不是代码长度的平方：

| | Python + Manim（原版实测行为）| 本 crate |
|---|---|---|
| 500 字符的字形光栅化次数 | ≈ 125,000（O(N²) 静态层重建） | ≈ 500 次逐字揭示 + 每帧视口裁剪后的绘制 |
| 帧渲染 | 1 核，串行 | 全部核心，有界批次 |
| 编码轮次 | 2（Manim + MoviePy）+ 1 次解码 | 1 |
| 分片视频文件 | 每个字符一个 | 无 |

### 实测数据

开发机：AMD Ryzen 9 7945HX (16C/32T)，Windows。

**纯光栅化** —— `cargo run --release --example bench`（1080p，不含编码与磁盘 IO，
缓冲区复用）：

| 线程数 | 墙钟（600 帧） | 吞吐 |
|---|---|---|
| 1 | 0.33 s | ~1,800 frames/s |
| 4 | 0.19 s | ~3,200 frames/s |
| 32 | 0.51 s | ~1,100 frames/s |

单线程就已经是 60fps 实时速度的约 30 倍 —— **渲染不再是瓶颈**。剩下的是编码，
而 ffmpeg 内部已经多线程了。

**重复帧是免费的**。一帧的 `FrameSignature`（相机位置、当前行、已显示字符数）若与上一帧
相同，就不会重新光栅化。默认的平滑相机只在结尾停留时命中；`--snap-camera` 让相机在字符
之间静止，命中率大幅提高：

| 相机 | fps | 不同帧数 | 全渲染 | 只渲染不同帧 | 加速 |
|---|---|---|---|---|---|
| 平滑（默认） | 30 | 1551 / 1580 | 2.14 s | 2.13 s | 1.00× |
| 平滑（默认） | 60 | 3101 / 3160 | 4.30 s | 4.36 s | 0.99× |
| `--snap-camera` | 30 | 342 / 1580 | 2.23 s | 0.48 s | **4.63×** |
| `--snap-camera` | 60 | 372 / 3160 | 4.46 s | 0.52 s | **8.63×** |

**整条流水线** —— 16 行代码（311 个字符，790 帧），PNG 序列输出（最慢的出路，
因为每帧还要 PNG 编码）：

| 场景 | 墙钟 | 吞吐 |
|---|---|---|
| 854×480 @ 15fps → PNG 序列 | 0.9 s | 836 frames/s |
| 1920×1080 @ 30fps → PNG 序列 | 0.6 s | 213 frames/s |

PNG 压缩是这条路上最贵的一步（占 83–89%），所以它跑在编码线程池上而不是渲染线程上；
队列深度上限是每线程一帧，内存因此有界。

四条来自实测的实践建议：

- **一定用 `--release` 构建**：`tiny-skia` 的光栅化器在未优化构建下慢约 20 倍。
- **复用帧缓冲**：每帧新分配一个 1080p 缓冲比直接往复用缓冲里光栅化贵约 6 倍
  （250 → 1,500 frames/s），因此渲染器为每个 worker 循环使用一个缓冲。
- **帧并行只在光栅化是瓶颈时才有收益**：1080p 下单是帧缓冲就有 8 MB，单线程已经吃掉了
  相当一部分内存带宽；超过约 4 线程后基准测试不再上升，在这台 CPU 上甚至回落。并行的意义
  是让渲染永远不成为限制因素。
- **布局复杂度对代码长度线性**：1600 行（73,470 字符）约 16 ms 完成排版，每字符成本在
  35 倍规模区间内保持平稳（约 0.22 µs）。

---

## 常见问题

**Q：报错 `ffmpeg was not found in PATH`。**
装到 PATH，或者用 `--ffmpeg <路径>` 指定，或者改用 `--frames <目录>` 输出 PNG 序列。

**Q：中文能显示，emoji 不行。**
内置字体包含中日韩字形，但不含 emoji。当前没有字形回退；遇到缺字会明确报错并给出码点。
需要的话可以在渲染前把 emoji 从代码里去掉。

**Q：`--language haskell` 没报错，但高亮很朴素。**
只有 19 种语言有专用语法；其余会退化成通用高亮（字符串、数字、行注释）。CLI 会打印一行
`note: no dedicated grammar for 'haskell'...` 提示。`cvr --list-languages` 可以看到全部。

**Q：视频比我预期的长。**
时长主要由「字符数 × `--interval`」决定，另有入场 1 秒与结尾 `--end-pause` 1 秒。
调小 `--interval`（注意下限是 `1/fps`）或 `--end-pause`。

**Q：想让输出完全可复现。**
`--seed` 固定住字符间隔的抖动；相同参数 + 相同种子 = 逐字节相同的视频。

**Q：内存占用大概多少。**
渲染侧是「线程数 × 单帧字节数」（1080p 约 8 MB/帧），PNG 输出再加「编码线程数 × 单帧」。
1080p、32 线程下峰值大约几百 MB 量级；要降低可以用 `--frames` 的并行度或减少线程数
（例如通过环境变量限制 rayon 线程池）。

**Q：批量渲染很多视频。**
用库 API：在同一个进程里循环调用 `render`，字体只会加载一次（有进程级缓存），
互不干扰。想中途取消就传 `with_cancel`。

---

## 与 Python 版的对照

| 特性 | Python + Manim | 本 crate |
|---|---|---|
| 从字符串/文件取代码 | ✅ | ✅ |
| tab 展开、首尾空行处理 | ✅ | ✅ |
| 非法字符拒绝 | ✅ | ✅ |
| 行号 + 当前行 | ✅ | ✅ |
| 当前行高亮条 | ✅ | ✅ |
| 光标 | ✅ | ✅ |
| 打字间隔范围 | ✅ | ✅（可用 `--seed` 复现） |
| 相机入场 / 跟随 / 自动拉远 | ✅ | ✅ |
| 手持相机晃动 | ✅ | ✅ |
| 语法高亮 | Pygments，600+ 词法器 | 19 种手写语法 + 通用回退 |
| 配色主题 | 60+ Pygments 风格 | 43 套 Pygments + 1 套手写默认 |
| 进度显示 | rich | 内置单行进度条 |
| 分辨率 / 帧率控制 | 只能改 Manim 全局配置 | ✅ 一等参数 |
| 帧并行渲染 | ❌ 串行 | ✅ rayon |
| 批处理安全（同进程多次渲染） | ❌ 全局参数互相污染 | ✅ |
| Linux/macOS 输出路径 | ❌ 硬编码 Windows 分隔符 | ✅ |
| 空代码 | 渲染时才崩 | 构造阶段就拒绝 |
| 辉光后期 | ✅（强制，两轮编码） | ✅ `--glow`（单次编码，默认关闭） |
| OpenGL 渲染器 | ✅（作者注明更慢） | 不适用 |
| 运行时依赖 | manim, pygments, moviepy, PIL, numpy, rich, ffmpeg | tiny-skia, ttf-parser, rayon, ffmpeg |

---

## 开发

```console
# 测试（含文档测试）
$ cargo test

# 静态检查
$ cargo fmt --check
$ cargo clippy --all-targets -- -D warnings

# 光栅化基准
$ cargo run --release --example bench

# 字形/字体诊断
$ cargo run --release --example diag
```

测试覆盖：预处理边界（空代码、空行、行内空格、tab、非法字符）、高亮器不变量
（每种语言下「一个字符一个 token kind」，以及预处理不会改写高亮器看到的文本）、
排版几何、相机拉远、时间线顺序、帧签名（相同签名必须光栅化出相同像素）、
自定义 sink、进度回调、取消、ffmpeg 命令行构造，以及端到端的 PNG 序列渲染。

## License

MIT，与原项目一致。
