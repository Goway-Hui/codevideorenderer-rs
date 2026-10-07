//! `cvr` — CodeVideoRenderer (Rust) 的命令行前端。

use std::path::PathBuf;
use std::process::ExitCode;

use codevideorenderer::api::{RenderOptions, render};
use codevideorenderer::lexer::is_supported;
use codevideorenderer::{Source, VERSION, supported_languages, theme};

/// 当既未提供 `--code` 也未提供 `--code-file` 时使用。
const EXAMPLE: &str = r#"def fibonacci(n):
    """Calculate the nth Fibonacci number."""
    if n <= 1:
        return n
    return fibonacci(n - 1) + fibonacci(n - 2)


# Example usage
result = fibonacci(10)
print(f"Fibonacci(10) = {result}")
"#;

const USAGE: &str = r#"cvr — render a "typing" code video

USAGE:
    cvr [OPTIONS]

INPUT (one of):
    --code <TEXT>            inline code to animate
    --code-file <PATH>       read code from a file (UTF-8)
                             (with neither, a built-in example is rendered)

APPEARANCE:
    --language <NAME>        language for syntax highlighting   [default: python]
    --style <NAME>           Pygments style name                 [default: midnight]
    --font <PATH>            monospace TTF/OTF to use            [default: bundled/system]
    --line-spacing <F>       extra line spacing                  [default: 0.8]

TIMING AND CAMERA:
    --interval <MIN[:MAX]>   seconds between characters          [default: 0.15]
    --camera-scale <F>       initial zoom, smaller = closer      [default: 0.5]
    --end-pause <F>          hold on the finished code, seconds  [default: 1.0]
    --seed <N>               jitter seed (deterministic output)
    --snap-camera            hold the camera still between characters
                             (frames repeat, so most are reused; the
                             camera no longer glides while typing)

OUTPUT:
    --output <PATH>          MP4 path                            [default: <name>.mp4]
    --name <STEM>            output name                         [default: CameraFollowCursorCV]
    --frames <DIR>           write PNG frames instead of a video (no ffmpeg needed)
    --resolution <SPEC>      1080p | 720p | 480p | 1920x1080     [default: 1080p]
    --width <N> --height <N> explicit pixel size
    --fps <N>                frame rate                          [default: 60]
    --crf <N>                x264 quality, lower = better        [default: 18]
    --preset <NAME>          x264 preset                         [default: veryfast]
    --glow                   add the original's glow post-process (slower encode)
    --ffmpeg <PATH>          ffmpeg executable                   [default: ffmpeg]
    --quiet                  suppress progress output

INFO:
    --list-styles            list bundled Pygments styles
    --list-languages         list languages the highlighter knows
    -h, --help               show this help
    -V, --version            show version
"#;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut opts = RenderOptions::default();
    let mut code: Option<String> = None;
    let mut code_file: Option<PathBuf> = None;
    let mut resolution: Option<String> = None;

    let mut i = 0usize;
    while i < args.len() {
        let (key, inline) = split_arg(&args[i]);
        match key {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("cvr {VERSION}");
                return Ok(());
            }
            "--list-styles" => {
                for name in theme::theme_names() {
                    println!("{name}");
                }
                return Ok(());
            }
            "--list-languages" => {
                for name in supported_languages() {
                    println!("{name}");
                }
                return Ok(());
            }
            "--code" => code = Some(take_value(args, &mut i, inline)?),
            "--code-file" => {
                code_file = Some(PathBuf::from(take_value(args, &mut i, inline)?));
            }
            "--language" => opts.language = take_value(args, &mut i, inline)?,
            "--style" => opts.style = take_value(args, &mut i, inline)?,
            "--font" => opts.font = Some(PathBuf::from(take_value(args, &mut i, inline)?)),
            "--output" => opts.output = Some(PathBuf::from(take_value(args, &mut i, inline)?)),
            "--name" => opts.video_name = take_value(args, &mut i, inline)?,
            "--frames" => opts.frames_dir = Some(PathBuf::from(take_value(args, &mut i, inline)?)),
            "--line-spacing" => {
                opts.line_spacing = number(&take_value(args, &mut i, inline)?, "--line-spacing")?
            }
            "--camera-scale" => {
                opts.camera_scale = number(&take_value(args, &mut i, inline)?, "--camera-scale")?
            }
            "--end-pause" => {
                opts.end_pause = number(&take_value(args, &mut i, inline)?, "--end-pause")?
            }
            "--interval" => {
                let raw = take_value(args, &mut i, inline)?;
                let (min, max) = parse_interval(&raw)?;
                opts.interval_min = min;
                opts.interval_max = max;
            }
            "--seed" => {
                opts.seed = take_value(args, &mut i, inline)?
                    .parse()
                    .map_err(|_| "--seed expects an integer")?
            }
            "--fps" => {
                opts.fps = take_value(args, &mut i, inline)?
                    .parse()
                    .map_err(|_| "--fps expects an integer")?
            }
            "--crf" => {
                opts.crf = take_value(args, &mut i, inline)?
                    .parse()
                    .map_err(|_| "--crf expects an integer between 0 and 51")?
            }
            "--preset" => opts.preset = take_value(args, &mut i, inline)?,
            "--ffmpeg" => opts.ffmpeg = take_value(args, &mut i, inline)?,
            "--glow" => opts.glow = true,
            "--resolution" => resolution = Some(take_value(args, &mut i, inline)?),
            "--width" => {
                opts.width = take_value(args, &mut i, inline)?
                    .parse()
                    .map_err(|_| "--width expects an integer")?
            }
            "--height" => {
                opts.height = take_value(args, &mut i, inline)?
                    .parse()
                    .map_err(|_| "--height expects an integer")?
            }
            "--quiet" => opts.quiet = true,
            "--snap-camera" => opts.snap_camera = true,
            other => return Err(format!("unknown argument '{other}' (try --help)").into()),
        }
        i += 1;
    }

    if let Some(spec) = resolution {
        let (w, h) = parse_resolution(&spec)?;
        opts.width = w;
        opts.height = h;
    }
    check_resolution(opts.width, opts.height)?;

    let source = match (code, code_file) {
        (Some(_), Some(_)) => {
            return Err("provide either --code or --code-file, not both".into());
        }
        (Some(text), None) => Source::Text(text),
        (None, Some(path)) => Source::File(path),
        (None, None) => {
            if !opts.quiet {
                eprintln!("note: no input given, rendering the built-in example");
            }
            Source::Text(EXAMPLE.to_string())
        }
    };

    if !opts.quiet {
        eprintln!(
            "cvr {VERSION}: {} · {} · {}x{} @ {}fps",
            opts.language, opts.style, opts.width, opts.height, opts.fps
        );
        if !is_supported(&opts.language) {
            // 保持宽容是有意为之——渲染仍能工作——但它不再静默发生。
            eprintln!(
                "note: no dedicated grammar for '{}'; falling back to the generic \
                 highlighter (see --list-languages)",
                opts.language
            );
        }
    }

    let report = render(source, opts)?;

    println!("  font     {}", report.font);
    println!("  theme    {}", report.theme);
    println!(
        "  layout   {} lines · {} typed characters · {} glyph outlines",
        report.lines, report.typed_chars, report.outlines
    );
    println!(
        "  frames   {} @ {}fps ({:.1}s)",
        report.frames, report.fps, report.duration
    );
    println!(
        "  render   {:.2}s ({:.0} frames/s on {} threads)",
        report.elapsed, report.frames_per_second, report.threads
    );
    if report.reused_frames > 0 {
        println!(
            "  reused   {} of {} frames repeated the previous image",
            report.reused_frames, report.frames
        );
    }
    println!("  output   {}", report.output);
    Ok(())
}

fn split_arg(arg: &str) -> (&str, Option<&str>) {
    match arg.split_once('=') {
        Some((key, value)) if key.starts_with('-') => (key, Some(value)),
        _ => (arg, None),
    }
}

fn take_value(
    args: &[String],
    index: &mut usize,
    inline: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(value) = inline {
        return Ok(value.to_string());
    }
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{} expects a value", args[*index - 1]).into())
}

fn number(raw: &str, what: &str) -> Result<f32, Box<dyn std::error::Error>> {
    raw.parse::<f32>()
        .map_err(|_| format!("{what} expects a number, got '{raw}'").into())
}

fn parse_interval(raw: &str) -> Result<(f32, f32), Box<dyn std::error::Error>> {
    match raw.split_once(':') {
        Some((min, max)) => {
            let min = min
                .parse::<f32>()
                .map_err(|_| format!("--interval: '{min}' is not a number"))?;
            let max = max
                .parse::<f32>()
                .map_err(|_| format!("--interval: '{max}' is not a number"))?;
            Ok((min, max))
        }
        None => {
            let value = raw
                .parse::<f32>()
                .map_err(|_| format!("--interval expects <MIN[:MAX]>, got '{raw}'"))?;
            Ok((value, value))
        }
    }
}

fn parse_resolution(spec: &str) -> Result<(u32, u32), Box<dyn std::error::Error>> {
    let lower = spec.to_ascii_lowercase();
    let preset = match lower.as_str() {
        "1080p" | "fhd" => Some((1920, 1080)),
        "720p" | "hd" => Some((1280, 720)),
        "480p" => Some((854, 480)),
        "1440p" | "2k" => Some((2560, 1440)),
        "2160p" | "4k" => Some((3840, 2160)),
        _ => None,
    };
    if let Some((w, h)) = preset {
        return check_resolution(w, h).map(|()| (w, h));
    }
    if let Some((w, h)) = lower.split_once('x') {
        let w: u32 = w
            .trim()
            .parse()
            .map_err(|_| format!("--resolution: '{spec}' has a bad width"))?;
        let h: u32 = h
            .trim()
            .parse()
            .map_err(|_| format!("--resolution: '{spec}' has a bad height"))?;
        check_resolution(w, h)?;
        return Ok((w, h));
    }
    Err(format!("--resolution expects 1080p, 720p, 480p or WxH, got '{spec}'").into())
}

/// 在参数解析阶段就对帧尺寸做范围检查，这样像 `100000x100000` 这种笔误会被
/// 当作错误参数报告，而不是在若干阶段之后才因分配失败而暴露。
fn check_resolution(w: u32, h: u32) -> Result<(), Box<dyn std::error::Error>> {
    let max = codevideorenderer::config::MAX_PIXELS;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > max {
        return Err(format!(
            "resolution {w}x{h} is out of range: both sides must be non-zero and the frame \
             must not exceed {max} pixels (8K)"
        )
        .into());
    }
    Ok(())
}
