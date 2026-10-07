//! 高层 API：把一段代码片段渲染成视频。
//!
//! ```no_run
//! use codevideorenderer::{render, RenderOptions, Source};
//!
//! let report = render(
//!     Source::text("def add(a, b):\n    return a + b\n"),
//!     RenderOptions {
//!         language: "python".into(),
//!         video_name: "Adder".into(),
//!         ..RenderOptions::default()
//!     },
//! )?;
//! println!("wrote {} ({} frames)", report.output, report.frames);
//! # Ok::<(), codevideorenderer::Error>(())
//! ```

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rayon::prelude::*;
use tiny_skia::Pixmap;

use crate::config::{
    DEFAULT_CAMERA_SCALE, DEFAULT_LINE_SPACING, DEFAULT_VIDEO_NAME, MAX_CRF, MAX_FPS, MAX_PIXELS,
};
use crate::encode::{EncodeOptions, FfmpegSink, FrameSink, PngSink, Progress};
use crate::error::{Error, Result};
use crate::font::Font;
use crate::layout::{self, Layout, Source};
use crate::render::{FrameRenderer, GlyphOutlines};
use crate::theme::{self, Theme};
use crate::timeline::{FrameSignature, Timeline, TimelineOptions};

/// 帧进度回调，调用时传入 `(frames_done, total_frames)`。
///
/// 克隆代价很低——只是一个 `Arc`——因此同一个回调可以同时驱动进度条、
/// 日志行和指标计数器。
#[derive(Clone)]
pub struct ProgressCallback(Arc<dyn Fn(u32, u32) + Send + Sync>);

impl ProgressCallback {
    /// 包装一个闭包。
    pub fn new(callback: impl Fn(u32, u32) + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }

    /// 报告进度。
    pub fn call(&self, done: u32, total: u32) {
        (self.0)(done, total);
    }
}

impl fmt::Debug for ProgressCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProgressCallback(..)")
    }
}

/// 控制一次渲染的全部参数。
///
/// 校验发生在 [`render`] *开始任何工作之前*，因此非法参数会立即失败，
/// 而不是在一次漫长渲染进行到一半时才报错——这是原实现最令人痛苦的
/// 行为之一。
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Pygments 语言名（例如 `"python"`）。
    pub language: String,
    /// Pygments 样式名（例如 `"github-dark"`），或内置默认样式
    /// `"midnight"`。
    pub style: String,
    /// 行与行之间的额外间距，以字体大小的比例表示。
    pub line_spacing: f32,
    /// 字符之间的最短延迟，单位秒。
    pub interval_min: f32,
    /// 字符之间的最长延迟，单位秒。
    pub interval_max: f32,
    /// 初始相机视野（越小越近）。
    pub camera_scale: f32,
    /// 输出文件主名（也是默认的 MP4 文件名）。
    pub video_name: String,
    /// 输出宽度，单位像素。
    pub width: u32,
    /// 输出高度，单位像素。
    pub height: u32,
    /// 输出帧率。
    pub fps: u32,
    /// 结束时完整代码停留的时长，单位秒。
    pub end_pause: f32,
    /// 显式字体路径；`None` 表示搜索内置字体和系统字体。
    pub font: Option<PathBuf>,
    /// MP4 的显式输出路径。
    pub output: Option<PathBuf>,
    /// 把 PNG 帧写到这里，而不是编码成视频。
    pub frames_dir: Option<PathBuf>,
    /// 抑制进度输出。
    pub quiet: bool,
    /// x264 恒定码率因子。
    pub crf: u8,
    /// x264 预设。
    pub preset: String,
    /// ffmpeg 可执行文件。
    pub ffmpeg: String,
    /// 对编码后的视频应用原实现的 glow 后处理。
    ///
    /// 默认关闭：它是一帧一帧的滤镜，会消耗编码时间。
    pub glow: bool,
    /// 打字抖动的种子。
    pub seed: u64,
    /// 让相机停留在每个关键帧上，而不是在关键帧之间平滑滑动。
    ///
    /// 更平滑的默认行为会在每一帧移动相机，因此没有两帧是相同的；而 snap
    /// 模式让相机在字符之间保持静止，使大部分帧可以被复用而不是重新绘制。
    /// 默认关闭，因为它改变了打字过程中相机的观感。
    pub snap_camera: bool,
    /// 每渲染完一帧后，以 `(frames_done, total_frames)` 调用。
    ///
    /// 内置的单行进度显示会继续与之并行工作；这正是库的调用方（或某个服务）
    /// 看到进度的方式。
    pub progress: Option<ProgressCallback>,
    /// 协作式取消：把标志设为 `true`，渲染就会在下一个批次边界处以
    /// [`Error::Cancelled`] 停止。
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            language: "python".into(),
            style: theme::DEFAULT_THEME_NAME.into(),
            line_spacing: DEFAULT_LINE_SPACING,
            interval_min: crate::config::DEFAULT_TYPE_INTERVAL,
            interval_max: crate::config::DEFAULT_TYPE_INTERVAL,
            camera_scale: DEFAULT_CAMERA_SCALE,
            video_name: DEFAULT_VIDEO_NAME.into(),
            width: 1920,
            height: 1080,
            fps: 60,
            end_pause: 1.0,
            font: None,
            output: None,
            frames_dir: None,
            quiet: false,
            crf: 18,
            preset: "veryfast".into(),
            ffmpeg: "ffmpeg".into(),
            glow: false,
            seed: 0x5eed_1234_abcd_0001,
            snap_camera: false,
            progress: None,
            cancel: None,
        }
    }
}

impl RenderOptions {
    /// Builder 风格的设置方法。
    ///
    /// 每个字段都是公开的、可以直接赋值；这些方法的存在是为了让链式调用
    /// 读起来像一句话，这正是批处理和服务代码想要的：
    ///
    /// ```
    /// use codevideorenderer::RenderOptions;
    ///
    /// let options = RenderOptions::default()
    ///     .with_language("rust")
    ///     .with_style("github-dark")
    ///     .with_fps(30)
    ///     .with_resolution(1280, 720)
    ///     .with_interval(0.06, 0.14)
    ///     .with_video_name("Demo")
    ///     .with_output("out/demo.mp4")
    ///     .with_quiet(true);
    ///
    /// assert_eq!(options.fps, 30);
    /// assert_eq!(options.interval_max, 0.14);
    /// ```
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    /// 设置 Pygments 样式名。
    pub fn with_style(mut self, style: impl Into<String>) -> Self {
        self.style = style.into();
        self
    }

    /// 设置额外的行间距，以字体大小的比例表示。
    pub fn with_line_spacing(mut self, spacing: f32) -> Self {
        self.line_spacing = spacing;
        self
    }

    /// 设置打字间隔范围，单位秒。
    pub fn with_interval(mut self, min: f32, max: f32) -> Self {
        self.interval_min = min;
        self.interval_max = max;
        self
    }

    /// 设置初始相机缩放（越小越近）。
    pub fn with_camera_scale(mut self, scale: f32) -> Self {
        self.camera_scale = scale;
        self
    }

    /// 设置输出名称（也是默认的 MP4 主名）。
    pub fn with_video_name(mut self, name: impl Into<String>) -> Self {
        self.video_name = name.into();
        self
    }

    /// 设置输出分辨率，单位像素。
    pub fn with_resolution(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// 设置帧率。
    pub fn with_fps(mut self, fps: u32) -> Self {
        self.fps = fps;
        self
    }

    /// 设置结束时完整代码停留的时长，单位秒。
    pub fn with_end_pause(mut self, seconds: f32) -> Self {
        self.end_pause = seconds;
        self
    }

    /// 使用指定的字体文件，而不是按搜索顺序查找。
    pub fn with_font(mut self, path: impl Into<PathBuf>) -> Self {
        self.font = Some(path.into());
        self
    }

    /// 把视频写到这里。
    pub fn with_output(mut self, path: impl Into<PathBuf>) -> Self {
        self.output = Some(path.into());
        self
    }

    /// 把 PNG 帧写到这里，而不是编码成视频。
    pub fn with_frames_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.frames_dir = Some(dir.into());
        self
    }

    /// 抑制内置的进度显示。
    pub fn with_quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// 设置 x264 质量（越低越好）。
    pub fn with_crf(mut self, crf: u8) -> Self {
        self.crf = crf;
        self
    }

    /// 设置 x264 预设。
    pub fn with_preset(mut self, preset: impl Into<String>) -> Self {
        self.preset = preset.into();
        self
    }

    /// 设置 ffmpeg 可执行文件。
    pub fn with_ffmpeg(mut self, ffmpeg: impl Into<String>) -> Self {
        self.ffmpeg = ffmpeg.into();
        self
    }

    /// 打开或关闭 glow 后处理。
    pub fn with_glow(mut self, glow: bool) -> Self {
        self.glow = glow;
        self
    }

    /// 设置打字抖动种子（相同种子，相同视频）。
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// 让相机停留在每个关键帧上，而不是在关键帧之间平滑滑动。
    ///
    /// 当一个字符正在被打出时帧会重复，因此大部分帧可以被复用而不是重新
    /// 绘制；不过相机不再平滑移动。
    pub fn with_snap_camera(mut self, snap: bool) -> Self {
        self.snap_camera = snap;
        self
    }

    /// 除了内置显示外，再向 `callback` 报告进度。
    pub fn with_progress(mut self, callback: ProgressCallback) -> Self {
        self.progress = Some(callback);
        self
    }

    /// 当 `token` 被设为 `true` 时停止渲染。
    pub fn with_cancel(mut self, token: Arc<AtomicBool>) -> Self {
        self.cancel = Some(token);
        self
    }
}

/// 一次渲染的产出。
#[derive(Debug, Clone)]
pub struct RenderReport {
    /// 已写入的帧数。
    pub frames: u32,
    /// 视频时长，单位秒。
    pub duration: f32,
    /// 视频渲染所用的帧率，即所请求的帧率。
    pub fps: u32,
    /// 输出最终落在哪里。
    pub output: String,
    /// 墙钟渲染耗时，单位秒。
    pub elapsed: f32,
    /// 打出的字符数。
    pub typed_chars: usize,
    /// 代码行数。
    pub lines: usize,
    /// 实际使用的字体。
    pub font: String,
    /// 实际使用的主题。
    pub theme: String,
    /// 缓存的去重后的字形轮廓数量。
    pub outlines: usize,
    /// 用于帧渲染的线程数。
    pub threads: usize,
    /// 墙钟吞吐量，单位帧/秒。
    pub frames_per_second: f32,
    /// 复用了前一帧图像、未被重新光栅化的帧。
    pub reused_frames: u32,
}

/// 渲染一个代码视频。
///
/// 这就是整条流水线：预处理 → 语法高亮 → 布局 → 构建打字 timeline →
/// 并行光栅化帧 → 编码。
pub fn render(source: Source, opts: RenderOptions) -> Result<RenderReport> {
    let sink = default_sink(&opts)?;
    render_with_sink(source, opts, sink)
}

/// 渲染到你提供的 [`FrameSink`] 中。
///
/// 内置 sink 覆盖 ffmpeg 管道和 PNG 序列；这里是其它一切的逃生通道——
/// 一个 socket、一个对象存储、一个调用方自有的编码器。
/// 该 sink 看到的帧与内置 sink 相同，顺序也相同。
pub fn render_with_sink(
    source: Source,
    opts: RenderOptions,
    mut sink: Box<dyn FrameSink>,
) -> Result<RenderReport> {
    validate(&opts)?;
    let mut report = render_frames(source, &opts, sink.as_mut())?;
    report.output = sink.finish()?;
    Ok(report)
}

/// 一个已经准备好但尚未开始的渲染。
///
/// 预处理、语法高亮、布局、timeline 和字形轮廓只取决于源码和选项——
/// 与时间无关——因此在这里一次性构建好。[`render_preview`] 每次调用都会
/// 重建这一切，对单帧来说没问题，但对任何要绘制很多帧的场景都很浪费：
/// 时间轴拖动条、缩略图条、采样十几次的测试。
///
/// ```no_run
/// use codevideorenderer::{Prepared, RenderOptions, Source};
///
/// let options = RenderOptions::default().with_fps(30);
/// let prepared = Prepared::new(&Source::file("script.py"), &options)?;
/// let timeline = prepared.timeline();
///
/// let middle = prepared.render_at(timeline.duration * 0.5)?;
/// println!("{} frames, mid frame {}x{}", timeline.frame_count, middle.width(), middle.height());
/// # Ok::<(), codevideorenderer::Error>(())
/// ```
pub struct Prepared {
    layout: Layout,
    timeline: Timeline,
    theme: &'static Theme,
    font: Font,
    outlines: GlyphOutlines,
    width: u32,
    height: u32,
}

impl Prepared {
    /// 为 `source` 进行预处理、语法高亮、布局并缓存字形轮廓。
    ///
    /// 这是渲染中代价高昂的一半——字体查找、布局遍历、轮廓提取——
    /// 也是与时间无关的那一半。
    pub fn new(source: &Source, opts: &RenderOptions) -> Result<Self> {
        validate(opts)?;
        let pre = layout::preprocess(source)?;
        let font = Font::find(opts.font.as_deref())?;
        let theme = theme::theme_by_name(&opts.style).ok_or_else(|| {
            Error::UnknownStyle(opts.style.clone(), theme::theme_names().join(", "))
        })?;
        let layout = layout::layout(&pre, &font, theme, &opts.language, opts.line_spacing)?;
        let timeline = Timeline::build(&layout, &timeline_options(opts));
        let outlines = GlyphOutlines::build(&font, &layout);
        Ok(Self {
            layout,
            timeline,
            theme,
            font,
            outlines,
            width: opts.width,
            height: opts.height,
        })
    }

    /// 布局后的代码：字形位置、行几何信息、颜色。
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// 显现时间、帧数和烘焙好的相机路径。
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// 正在使用的字体（进程级缓存字体的一个克隆）。
    pub fn font(&self) -> &Font {
        &self.font
    }

    /// 正在使用的主题。
    pub fn theme(&self) -> &Theme {
        self.theme
    }

    /// 缓存的字形轮廓数量。
    pub fn outlines(&self) -> usize {
        self.outlines.len()
    }

    /// 输出尺寸，取自选项。
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 用于本次已准备渲染的光栅化器，可用任意输出尺寸。
    pub fn renderer(&self, width: u32, height: u32) -> FrameRenderer<'_> {
        FrameRenderer::new(
            &self.layout,
            &self.timeline,
            self.theme,
            &self.font,
            &self.outlines,
            width,
            height,
        )
    }

    /// 渲染呈现时刻 `t` 处的帧。
    pub fn render_at(&self, t: f32) -> Result<Pixmap> {
        let mut pixmap = Pixmap::new(self.width, self.height).ok_or(Error::Resolution {
            w: self.width,
            h: self.height,
        })?;
        self.renderer(self.width, self.height)
            .render(t, &mut pixmap);
        Ok(pixmap)
    }
}

/// 一组 [`RenderOptions`] 所隐含的 timeline 设置。
fn timeline_options(opts: &RenderOptions) -> TimelineOptions {
    TimelineOptions {
        interval_min: opts.interval_min,
        interval_max: opts.interval_max,
        line_break_run_time: crate::config::DEFAULT_LINE_BREAK_RUN_TIME,
        entrance_run_time: crate::config::DEFAULT_ENTRANCE_RUN_TIME,
        end_pause: opts.end_pause,
        camera_scale: opts.camera_scale,
        fps: opts.fps,
        seed: opts.seed,
        snap_camera: opts.snap_camera,
    }
}

/// [`RenderOptions`] 所描述的 sink：当设置了 `frames_dir` 时为 PNG 帧，
/// 否则为一条通向 ffmpeg 的管道。
fn default_sink(opts: &RenderOptions) -> Result<Box<dyn FrameSink>> {
    match &opts.frames_dir {
        Some(dir) => Ok(Box::new(PngSink::new(dir)?)),
        None => {
            let out = opts
                .output
                .clone()
                .unwrap_or_else(|| PathBuf::from(format!("{}.mp4", opts.video_name)));
            Ok(Box::new(FfmpegSink::spawn(
                &out,
                &EncodeOptions {
                    fps: opts.fps,
                    width: opts.width,
                    height: opts.height,
                    crf: opts.crf,
                    preset: opts.preset.clone(),
                    codec: "libx264".into(),
                    ffmpeg: opts.ffmpeg.clone(),
                    glow: opts.glow,
                },
            )?))
        }
    }
}

/// 共享流水线：预处理 → 语法高亮 → 布局 → timeline → 帧。
fn render_frames(
    source: Source,
    opts: &RenderOptions,
    sink: &mut dyn FrameSink,
) -> Result<RenderReport> {
    let started = Instant::now();
    let prepared = Prepared::new(&source, opts)?;
    let Prepared {
        layout,
        timeline,
        theme,
        font,
        outlines,
        ..
    } = &prepared;
    let renderer = FrameRenderer::new(
        layout,
        timeline,
        theme,
        font,
        outlines,
        opts.width,
        opts.height,
    );

    // 有界的批次让内存保持平稳，同时仍然占满每个核心：帧之间相互独立，
    // 所以这是原实现无法与之竞争的唯一一处。
    //
    // 每个 worker 一个 RGBA 缓冲，跨批次循环复用。为每一帧分配一个全新的
    // 1080p 缓冲的代价约为光栅化进它的 6 倍（见 README 中的性能说明），
    // 所以是缓冲池——而不是分批——让流水线保持高速。
    let threads = rayon::current_num_threads().max(1);
    let batch = threads;
    let size = tiny_skia::IntSize::from_wh(opts.width, opts.height).ok_or(Error::Resolution {
        w: opts.width,
        h: opts.height,
    })?;
    let pool: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::with_capacity(batch));
    let mut progress = Progress::new(timeline.frame_count, !opts.quiet);
    let mut written = 0u32;
    let mut reused = 0u32;
    // 最后绘制的那一帧，以及它所属的签名，这样跨批次边界重复的帧
    // 仍然有图像可以复用。
    let mut previous: Option<FrameSignature> = None;
    let mut carry: Option<Pixmap> = None;

    for start in (0..timeline.frame_count).step_by(batch) {
        // 批次边界是天然的检查点：每批次只加载一次标志，
        // 取消后最多浪费一个批次的工作。
        if is_cancelled(opts.cancel.as_ref()) {
            return Err(Error::Cancelled);
        }
        let end = (start + batch as u32).min(timeline.frame_count);
        let signatures: Vec<FrameSignature> = (start..end)
            .map(|index| timeline.signature_of_frame(index))
            .collect();

        // 签名与前一帧相同的帧会光栅化出相同的字节，因此每一段重复运行
        // 中只有第一帧真正被绘制。
        let fresh: Vec<u32> = (0..signatures.len() as u32)
            .filter(|offset| {
                let before = if *offset == 0 {
                    previous
                } else {
                    Some(signatures[*offset as usize - 1])
                };
                before != Some(signatures[*offset as usize])
            })
            .collect();

        let drawn: Vec<Pixmap> = fresh
            .par_iter()
            .map(|offset| {
                let buffer = pool
                    .lock()
                    .expect("frame buffer pool poisoned")
                    .pop()
                    .unwrap_or_default();
                let bytes = renderer
                    .render_into_vec(timeline.time_of_frame(start + offset), buffer)
                    .expect("validated resolution");
                Pixmap::from_vec(bytes, size).expect("validated resolution")
            })
            .collect();

        let mut next = 0usize;
        for offset in 0..signatures.len() {
            let pixmap: &Pixmap = if fresh.get(next) == Some(&(offset as u32)) {
                let pixmap = &drawn[next];
                next += 1;
                pixmap
            } else if next == 0 {
                // 仍处于上一个批次结束时的那一帧内。
                reused += 1;
                carry.as_ref().expect("a repeat has a frame to repeat")
            } else {
                // 复用本批次中更早绘制的帧。
                reused += 1;
                &drawn[next - 1]
            };
            sink.write(start + offset as u32, pixmap)?;
            progress.tick();
            written += 1;
            if let Some(callback) = &opts.progress {
                callback.call(written, timeline.frame_count);
            }
        }

        // 为下一批次保留最后绘制的帧；其它缓冲则回收复用。
        let mut pool = pool.lock().expect("frame buffer pool poisoned");
        let mut drained = drawn.into_iter().peekable();
        while let Some(pixmap) = drained.next() {
            if drained.peek().is_none() {
                carry = Some(pixmap);
            } else {
                pool.push(pixmap.take());
            }
        }
        drop(pool);
        previous = signatures.last().copied();
    }
    progress.clear();

    let elapsed = started.elapsed().as_secs_f32();

    Ok(RenderReport {
        frames: written,
        duration: timeline.duration,
        fps: opts.fps,
        // 由 `render_with_sink` 在 sink 完成时填入。
        output: String::new(),
        elapsed,
        typed_chars: layout.typed_chars,
        lines: layout.lines.len(),
        font: font.origin().to_string(),
        theme: theme.name.to_string(),
        outlines: outlines.len(),
        threads,
        frames_per_second: if elapsed > 0.0 {
            written as f32 / elapsed
        } else {
            0.0
        },
        reused_frames: reused,
    })
}

/// 对非法参数快速失败。
fn validate(opts: &RenderOptions) -> Result<()> {
    if opts.video_name.trim().is_empty() {
        return Err(Error::VideoName);
    }
    if opts.width == 0 || opts.height == 0 {
        return Err(Error::Resolution {
            w: opts.width,
            h: opts.height,
        });
    }
    // 对任意 `u32` 组合，`width * height` 都能放进 `u64`，因此这个检查本身
    // 不会溢出；它防止的是稍后分配时的中止。
    let pixels = u64::from(opts.width) * u64::from(opts.height);
    if pixels > MAX_PIXELS {
        return Err(Error::ResolutionTooLarge {
            w: opts.width,
            h: opts.height,
            pixels,
            max_pixels: MAX_PIXELS,
        });
    }
    if opts.fps == 0 || opts.fps > MAX_FPS {
        return Err(Error::FrameRate(opts.fps));
    }
    if opts.crf > MAX_CRF {
        return Err(Error::Crf(opts.crf));
    }
    // 负的停留时长会把视频缩短到只剩 `.max(1)` 的安全帧，
    // 无穷大则会要求 `u32::MAX` 帧。
    if !opts.end_pause.is_finite() || opts.end_pause < 0.0 {
        return Err(Error::EndPause(opts.end_pause));
    }
    if !opts.line_spacing.is_finite() || opts.line_spacing <= 0.0 {
        return Err(Error::LineSpacing(opts.line_spacing));
    }
    if !opts.camera_scale.is_finite() || opts.camera_scale <= 0.0 {
        return Err(Error::CameraScale(opts.camera_scale));
    }
    let min_allowed = 1.0 / opts.fps as f32;
    // 先检查有限性：与 NaN 的比较都为 false，所以 NaN 否则会
    // 溜过下面的范围检查。
    if !opts.interval_min.is_finite() || !opts.interval_max.is_finite() {
        return Err(Error::IntervalRange {
            min: opts.interval_min,
            max: opts.interval_max,
            min_allowed,
        });
    }
    if opts.interval_min < min_allowed - 1e-9 || opts.interval_max < opts.interval_min {
        return Err(Error::IntervalRange {
            min: opts.interval_min,
            max: opts.interval_max,
            min_allowed,
        });
    }
    Ok(())
}

/// 当调用方的取消令牌已被设置时为 `true`。
fn is_cancelled(token: Option<&Arc<AtomicBool>>) -> bool {
    token.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

/// 把单帧渲染到一张 pixmap——便于预览和测试。
///
/// 每次调用都会重建布局和轮廓缓存。当你需要从同一源码得到多于一帧——
/// 拖动条、缩略图条——请构建一次 [`Prepared`]，然后改调
/// [`Prepared::render_at`]。
pub fn render_preview(source: Source, opts: &RenderOptions, t: f32) -> Result<Pixmap> {
    Prepared::new(&source, opts)?.render_at(t)
}
