//! # CodeVideoRenderer (Rust)
//!
//! 渲染“打字”代码视频：取一段代码，逐字符地做动画，同时让相机跟随光标，
//! 并把结果编码为 MP4。
//!
//! 这是 [CodeVideoRenderer](https://github.com/ExploreMaths/CodeVideoRenderer)
//! 的从零开始的 Rust 重实现，原项目基于 Manim 构建了同样的效果。视觉语言被
//! 有意保留：等宽网格、行号、当前行高亮条、光标、相机跟随/拉远，以及手持
//! 相机的晃动。
//!
//! ## 结构上有哪些变化
//!
//! 原版对每个键入字符生成一次动画调用。这迫使动画引擎为每个字符重建其静态
//! 图层——线性输出却付出二次方的工作量——而且它永远无法使用多于一个核心，
//! 因为整条流水线是一个有状态的串行循环。
//!
//! 这里流水线被拆分为不可变的阶段：
//!
//! ```text
//! Source ──preprocess──▶ text ──lexer──▶ token kinds
//!                                   │
//!                                   ▼
//!                              layout (glyphs + positions)
//!                                   │
//!                                   ▼
//!                          timeline (reveal times + camera keys)
//!                                   │
//!                        frame(t) = pure function  ──parallel──▶ encoder
//! ```
//!
//! 由于一帧只取决于传入的 *时间*，帧可以在所有核心上渲染，并直接流入单一的
//! 编码器。库中任何地方都没有全局可变状态，因此原版的多实例与渲染中断 bug
//! 在这里根本无法表达出来。
//!
//! ## 快速开始
//!
//! ```no_run
//! use codevideorenderer::{render, RenderOptions, Source};
//!
//! let options = RenderOptions {
//!     language: "python".into(),
//!     style: "github-dark".into(),
//!     ..RenderOptions::default()
//! };
//! let report = render(Source::file("script.py"), options)?;
//! println!("{} frames in {:.1}s -> {}", report.frames, report.elapsed, report.output);
//! # Ok::<(), codevideorenderer::Error>(())
//! ```

// 高亮器与布局按索引遍历字符：它们需要前瞻和绝对位置，而迭代器链反而会让
// 这一点变得晦涩而非清晰。
#![allow(clippy::needless_range_loop)]

pub mod api;
pub mod camera;
pub mod config;
pub mod encode;
pub mod error;
pub mod font;
pub mod layout;
pub mod lexer;
pub mod render;
pub mod theme;
pub mod theme_data;
pub mod timeline;

pub use api::{
    Prepared, ProgressCallback, RenderOptions, RenderReport, render, render_preview,
    render_with_sink,
};
pub use encode::{EncodeOptions, FfmpegSink, FrameSink, PngSink, Progress};
pub use error::{Error, Result};
pub use layout::Source;
pub use lexer::supported_languages;
pub use theme::{Theme, TokenKind, theme_by_name, theme_names};

/// 重新导出，以便自定义 [`FrameSink`] 能命名其帧类型，而无需直接依赖 `tiny-skia`。
pub use tiny_skia;

/// crate 版本号。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
