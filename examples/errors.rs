//! 错误处理：所有失败模式都是强类型错误，且在渲染开始*之前*校验。
//!
//! 非法参数立即失败，而不是在一次漫长渲染进行到一半时才报错。
//!
//! ```console
//! cargo run --example errors
//! ```

use codevideorenderer::{Error, RenderOptions, Source, render};

fn main() {
    // 1. 参数校验错误：在开始任何工作之前就返回。
    let bad = RenderOptions::default().with_line_spacing(-1.0);
    match render(Source::text("print(1)\n"), bad) {
        Err(e @ Error::LineSpacing(_)) => println!("参数校验拦截: {e}"),
        other => println!("意外结果: {other:?}"),
    }

    // 2. 未知主题：错误里带着输入的名字和可用建议。
    let bad = RenderOptions::default().with_style("no-such-theme");
    match render(Source::text("print(1)\n"), bad) {
        Err(e @ Error::UnknownStyle(..)) => println!("主题错误: {e}"),
        other => println!("意外结果: {other:?}"),
    }

    // 3. 空代码。
    match render(Source::text(""), RenderOptions::default().with_quiet(true)) {
        Err(e @ Error::EmptyCode) => println!("空代码: {e}"),
        other => println!("意外结果: {other:?}"),
    }

    // 4. 读不存在的文件。
    match render(Source::file("no/such/file.py"), RenderOptions::default().with_quiet(true)) {
        Err(e @ Error::CodeFile { .. }) => println!("文件错误: {e}"),
        other => println!("意外结果: {other:?}"),
    }

    // 5. ffmpeg 缺失：MP4 输出在启动时即检测。
    let opts = RenderOptions::default().with_ffmpeg("definitely-not-ffmpeg-xyz");
    match render(Source::text("print(1)\n"), opts) {
        Err(e @ Error::FfmpegMissing | e @ Error::FfmpegStart(_)) => println!("ffmpeg: {e}"),
        other => println!("意外结果: {other:?}"),
    }
}
