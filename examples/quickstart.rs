//! 最短路径：三行代码把一段字符串渲染成 MP4。
//!
//! ```console
//! cargo run --example quickstart
//! ```
//!
//! 输出：`hello_cvr.mp4`（需要 PATH 里有 ffmpeg）。

use codevideorenderer::{RenderOptions, Source, render};

fn main() -> Result<(), codevideorenderer::Error> {
    let report = render(
        Source::text("def add(a, b):\n    return a + b\n"),
        RenderOptions::default(),
    )?;

    println!("{} frames in {:.1}s -> {}", report.frames, report.elapsed, report.output);
    Ok(())
}
