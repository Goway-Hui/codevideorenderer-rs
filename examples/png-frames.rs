//! 不依赖 ffmpeg 的输出方式：把帧写成 PNG 序列。
//!
//! ```console
//! cargo run --example png-frames
//! ```
//!
//! 输出：`frames_png/` 目录下的一串 `000001.png`…（无任何外部工具依赖）。

use codevideorenderer::{RenderOptions, Source, render};

fn main() -> Result<(), codevideorenderer::Error> {
    let options = RenderOptions::default()
        .with_frames_dir("frames_png") // 设置后走 PNG 序列而不是 MP4
        .with_resolution(960, 540)
        .with_fps(30)
        .with_quiet(true);

    let report = render(
        Source::text("for i in range(10):\n    print(i * i)\n"),
        options,
    )?;

    println!(
        "{} frames written to frames_png/ ({:.1}s of video)",
        report.frames, report.duration
    );
    Ok(())
}
