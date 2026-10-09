//! 进度回调与协作式取消——把库嵌入服务时的接法。
//!
//! ```console
//! cargo run --example progress
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use codevideorenderer::{ProgressCallback, RenderOptions, Source, render};

fn main() -> Result<(), codevideorenderer::Error> {
    // 每渲染完一帧回调一次 (frames_done, total_frames)。
    let last_percent = Arc::new(AtomicU64::new(0));
    let progress = {
        let last = Arc::clone(&last_percent);
        ProgressCallback::new(move |done, total| {
            let percent = (done as u64 * 100) / total.max(1) as u64;
            // 避免刷屏：只在跨过 10% 的整数倍时打印。
            if percent / 10 > last.swap(percent, Ordering::Relaxed) / 10 {
                println!("progress: {done}/{total} ({percent}%)");
            }
        })
    };

    // 协作式取消：置 true 后渲染在下一个批次边界以 Error::Cancelled 停止。
    let cancel = Arc::new(AtomicBool::new(false));
    // 想测试取消就放开下一行（例如由信号处理器 / HTTP 端点触发）：
    // cancel.store(true, Ordering::Relaxed);

    let options = RenderOptions::default()
        .with_resolution(1280, 720)
        .with_quiet(true)
        .with_progress(progress)
        .with_cancel(cancel);

    match render(Source::file("examples/fibonacci.py"), options) {
        Ok(report) => println!("done: {} frames -> {}", report.frames, report.output),
        Err(codevideorenderer::Error::Cancelled) => println!("render cancelled"),
        Err(e) => return Err(e),
    }
    Ok(())
}
