//! 批量渲染：同进程内渲染多段代码，展示 `Source::file` 与报告解读。
//!
//! 库无全局状态，同进程多次渲染互不干扰——这正是它与原 Python 版的
//! 结构性区别之一。
//!
//! ```console
//! cargo run --example batch
//! ```

use codevideorenderer::{RenderOptions, Source, render};

fn main() -> Result<(), codevideorenderer::Error> {
    let jobs = [
        ("examples/fibonacci.py", "batch_fibonacci"),
        // 也可以直接内联源码：
        // ("inline", Source::text("...")),
    ];

    for (path, name) in jobs {
        let options = RenderOptions::default()
            .with_video_name(name)
            .with_output(format!("{}.mp4", name))
            .with_resolution(1280, 720)
            .with_quiet(true);

        let report = render(Source::file(path), options)?;

        let fps_wall = report.frames_per_second;
        println!(
            "{name}: {} lines, {} chars typed, {} frames @ {}fps \
             ({fps_wall:.0} fps wall, {} reused) -> {}",
            report.lines,
            report.typed_chars,
            report.frames,
            report.fps,
            report.reused_frames,
            report.output,
        );
    }
    Ok(())
}
