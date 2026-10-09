//! Builder 风格选项全览：语言、主题、分辨率、帧率、打字节奏、相机、
//! 编码参数、glow 后处理与随机种子。
//!
//! ```console
//! cargo run --example styled
//! ```
//!
//! 输出：`styled_demo.mp4`。

use codevideorenderer::{RenderOptions, Source, render};

fn main() -> Result<(), codevideorenderer::Error> {
    let options = RenderOptions::default()
        // 内容侧
        .with_language("python") // 任意 Pygments 语言名
        .with_style("midnight") // 内置默认主题；也可用任意 Pygments 风格名
        .with_line_spacing(1.6) // 行距（字体大小的倍数）
        .with_interval(0.04, 0.12) // 打字节奏：字符间延迟的下限/上限（秒）
        .with_seed(42) // 打字抖动种子；同一种子 = 同一节奏
        // 相机侧
        .with_camera_scale(8.0) // 初始视野，越小越近
        .with_snap_camera(true) // 相机逐关键帧跳变（更多帧可复用，渲染更快）
        .with_end_pause(3.0) // 结尾完整代码停留时长（秒）
        // 输出侧
        .with_video_name("StyledDemo")
        .with_resolution(1280, 720)
        .with_fps(30)
        .with_crf(20) // x264 恒定质量，越大文件越小
        .with_preset("medium") // x264 预设：越慢压缩越好
        .with_glow(true); // 原版同款辉光后处理（逐帧滤镜，较慢）

    let report = render(Source::text("print('hello, styled world')\n"), options)?;
    println!(
        "{}x{} @ {}fps -> {} ({:.1}s, reused {} frames)",
        1280, 30, 30, report.output, report.duration, report.reused_frames
    );
    Ok(())
}
