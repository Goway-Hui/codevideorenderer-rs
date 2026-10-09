//! 自定义 `FrameSink`：把帧送到内置 sink 到不了的地方。
//!
//! 这里演示一个把每帧尺寸和序号记录到文件的 `FrameSink`；同样的接口
//! 可以接 socket、对象存储或调用方自有的编码器。帧按顺序到达，且来自
//! 单一线程。
//!
//! ```console
//! cargo run --example custom-sink
//! ```

use std::fs::{File, create_dir_all};
use std::io::{BufWriter, Write};

use codevideorenderer::{FrameSink, RenderOptions, Result, Source, render_with_sink};

struct FrameLogSink {
    log: BufWriter<File>,
    frames: u32,
}

impl FrameLogSink {
    fn new(path: &str) -> Result<Self> {
        if let Some(dir) = std::path::Path::new(path).parent() {
            create_dir_all(dir)?;
        }
        Ok(Self {
            log: BufWriter::new(File::create(path)?),
            frames: 0,
        })
    }
}

impl FrameSink for FrameLogSink {
    fn write(&mut self, frame_index: u32, pixmap: &tiny_skia::Pixmap) -> Result<()> {
        writeln!(
            self.log,
            "frame {frame_index}: {}x{}",
            pixmap.width(),
            pixmap.height()
        )?;
        self.frames += 1;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<String> {
        self.log.flush()?;
        Ok(format!("frame-log ({} frames)", self.frames))
    }
}

fn main() -> codevideorenderer::Result<()> {
    // 注意：走自定义 sink 时不编码 MP4，也不需要 ffmpeg。
    let options = RenderOptions::default()
        .with_resolution(960, 540)
        .with_fps(30)
        .with_quiet(true);

    let sink = Box::new(FrameLogSink::new("out/frame-log.txt")?);
    let report = render_with_sink(
        Source::text("x = 1\ny = 2\nprint(x + y)\n"),
        options,
        sink,
    )?;

    println!("{} frames -> {}", report.frames, report.output);
    Ok(())
}
