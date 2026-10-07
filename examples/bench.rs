//! 光栅化器吞吐量基准测试。
//!
//! ```console
//! cargo run --release --example bench -- [frames] [width] [height]
//! ```
//!
//! 只测量**帧渲染**——不含编码、不含磁盘 I/O——因此它展示的是帧并行架构
//! 实际带来的收益。在真实渲染中，编码器是下一个瓶颈，而这正是它应有的位置。

use std::time::Instant;

use codevideorenderer::font::Font;
use codevideorenderer::layout::{self, Source, preprocess};
use codevideorenderer::render::{FrameRenderer, GlyphOutlines};
use codevideorenderer::theme;
use codevideorenderer::timeline::{Timeline, TimelineOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let total: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(600);
    let width: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1920);
    let height: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1080);

    let pre = preprocess(&Source::File("examples/fibonacci.py".into()))?;
    let font = Font::find(None)?;
    let theme = theme::default_theme();
    let layout = layout::layout(&pre, &font, theme, "python", 0.8)?;

    let timeline = Timeline::build(
        &layout,
        &TimelineOptions {
            fps: 60,
            ..TimelineOptions::default()
        },
    );

    let outlines = GlyphOutlines::build(&font, &layout);
    let renderer = FrameRenderer::new(&layout, &timeline, theme, &font, &outlines, width, height);

    let times: Vec<f32> = (0..total)
        .map(|i| timeline.time_of_frame((i as u32) % timeline.frame_count.max(1)))
        .collect();

    println!(
        "{} frames at {width}x{height} · {} code glyphs · {} outlines cached",
        total,
        layout.code_glyphs,
        outlines.len()
    );

    // 预热：首次触碰缓存的外形轮廓和刚映射的页面。
    for i in 0..8u32 {
        std::hint::black_box(renderer.render_to_rgba(timeline.time_of_frame(i)));
    }

    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let mut thread_counts = vec![1usize];
    let mut n = 2usize;
    while n <= cores {
        thread_counts.push(n);
        n *= 2;
    }
    if *thread_counts.last().unwrap() != cores {
        thread_counts.push(cores);
    }

    println!();
    println!(
        "{:>8}  {:>10}  {:>12}  {:>8}",
        "threads", "wall", "frames/s", "speedup"
    );
    let mut baseline = 0.0f32;

    for threads in thread_counts {
        let slice = total / threads;
        if slice == 0 {
            continue;
        }
        let started = Instant::now();
        let painted = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|worker| {
                    let renderer = &renderer;
                    let times = &times;
                    scope.spawn(move || {
                        let mut painted = 0usize;
                        let mut buffer: Vec<u8> = Vec::new();
                        for t in &times[worker * slice..worker * slice + slice] {
                            match renderer.render_into_vec(*t, std::mem::take(&mut buffer)) {
                                Some(bytes) => {
                                    painted += bytes.len();
                                    buffer = bytes;
                                }
                                None => break,
                            }
                        }
                        painted
                    })
                })
                .collect();
            let mut total_painted = 0usize;
            for handle in handles {
                total_painted += handle.join().unwrap_or(0);
            }
            total_painted
        });
        std::hint::black_box(painted);

        let elapsed = started.elapsed().as_secs_f32();
        let rendered = (slice * threads) as f32;
        let throughput = rendered / elapsed;
        if threads == 1 {
            baseline = throughput;
        }
        println!(
            "{threads:>8}  {:>9.0}ms  {throughput:>12.0}  {:>7.2}x",
            elapsed * 1000.0,
            if baseline > 0.0 {
                throughput / baseline
            } else {
                1.0
            }
        );
    }

    Ok(())
}
