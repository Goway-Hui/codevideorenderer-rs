//! `Prepared`：一次布局，多次取帧——做时间轴缩略图、拖动条或测试采样时用。
//!
//! 预处理、语法高亮、布局、timeline 和字形轮廓只算一次；之后 `render_at(t)`
//! 是关于时间的纯函数，想取多少帧都行，互不干扰。
//!
//! 对比：`render_preview` 每次调用都重建这一切，对单帧没问题，
//! 对连续采样很浪费。
//!
//! ```console
//! cargo run --example prepared-samples
//! ```
//!
//! 输出：`out/sample_*.png`（9 个时间点的帧）。

use codevideorenderer::{Prepared, RenderOptions, Source};

fn main() -> Result<(), codevideorenderer::Error> {
    let options = RenderOptions::default().with_resolution(1280, 720);
    let prepared = Prepared::new(&Source::file("examples/fibonacci.py"), &options)?;

    let timeline = prepared.timeline();
    println!(
        "layout: {} lines, {} glyphs outlines, {} frames total, {:.1}s",
        prepared.layout().lines.len(),
        prepared.outlines(),
        timeline.frame_count,
        timeline.duration,
    );

    std::fs::create_dir_all("out")?;
    for i in 0..9u32 {
        let t = timeline.duration * (i as f32 + 0.5) / 9.0;
        let pixmap = prepared.render_at(t)?;
        let path = format!("out/sample_{i}.png");
        pixmap
            .save_png(&path)
            .map_err(|e| codevideorenderer::Error::Image(e.to_string()))?;
        println!("t={t:5.2}s -> {path}");
    }
    Ok(())
}
