//! 把单个帧渲染为 PNG——无需完整渲染即可预览，非常方便。
//!
//! ```console
//! cargo run --example preview -- examples/fibonacci.py preview.png 60
//! ```
//!
//! 参数：`<code-file> [out.png] [time-seconds] [width] [height]`

use codevideorenderer::api::{RenderOptions, render_preview};
use codevideorenderer::layout::Source;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code_file = args
        .first()
        .cloned()
        .unwrap_or_else(|| "examples/fibonacci.py".to_string());
    let out = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "preview.png".to_string());
    let time: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60.0);
    let width: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(720);

    let options = RenderOptions {
        width,
        height,
        fps: 30,
        ..RenderOptions::default()
    };

    let pixmap = render_preview(Source::File(code_file.into()), &options, time)?;
    pixmap
        .save_png(&out)
        .map_err(|e| codevideorenderer::Error::Image(e.to_string()))?;
    println!(
        "wrote {out} ({}x{}, t={time}s)",
        pixmap.width(),
        pixmap.height()
    );
    Ok(())
}
