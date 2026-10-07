//! 渲染器的错误类型。
//!
//! 库式错误使用 `thiserror` 并返回给调用方；二进制程序会把它们转换为一条消息
//! 加一个非零退出码。

use std::path::PathBuf;

/// 准备或渲染视频过程中可能出错的所有情况。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("code must not be empty (after trimming leading/trailing blank lines)")]
    EmptyCode,

    #[error(
        "code contains characters that cannot be rendered: {0:?} \
         (\\r, \\v and \\f break text layout — replace them with spaces or newlines)"
    )]
    InvalidCharacters(Vec<char>),

    #[error("failed to read code file '{path}': {source}")]
    CodeFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to decode '{path}' as UTF-8: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: std::str::Utf8Error,
    },

    #[error("line_spacing must be a finite number greater than 0, got {0}")]
    LineSpacing(f32),

    #[error(
        "interval_range must satisfy {min_allowed} <= min <= max (min is capped by 1/fps), \
         got min={min}, max={max}"
    )]
    IntervalRange {
        min: f32,
        max: f32,
        min_allowed: f32,
    },

    #[error("camera_scale must be a finite number greater than 0, got {0}")]
    CameraScale(f32),

    #[error("video_name must not be empty")]
    VideoName,

    #[error("unknown style '{0}'; available styles: {1}")]
    UnknownStyle(String, String),

    #[error("frame_rate must be between 1 and 1000, got {0}")]
    FrameRate(u32),

    #[error("end_pause must be a finite number of seconds, zero or more, got {0}")]
    EndPause(f32),

    #[error("crf must be between 0 and 51 (x264's range), got {0}")]
    Crf(u8),

    #[error("resolution must be greater than 0x0, got {w}x{h}")]
    Resolution { w: u32, h: u32 },

    #[error(
        "resolution {w}x{h} is too large ({pixels} pixels); the limit is {max_pixels} \
         pixels (8K). Check for a typo such as 100000x100000"
    )]
    ResolutionTooLarge {
        w: u32,
        h: u32,
        pixels: u64,
        max_pixels: u64,
    },

    #[error(
        "no usable font found; tried: {0:?}\n\
         pass --font <path.ttf> to point at a monospace font"
    )]
    FontNotFound(Vec<String>),

    #[error("failed to parse font '{path}': {reason}")]
    FontParse { path: String, reason: String },

    #[error("the font '{path}' does not contain a glyph for U+{codepoint:04X} ({ch:?})")]
    MissingGlyph {
        path: String,
        codepoint: u32,
        ch: char,
    },

    #[error(
        "ffmpeg was not found in PATH.\n\
         Install ffmpeg, or write PNG frames instead with --frames <dir>"
    )]
    FfmpegMissing,

    #[error("failed to start ffmpeg: {0}")]
    FfmpegStart(#[source] std::io::Error),

    #[error("ffmpeg exited with status {code:?}:\n{stderr}")]
    FfmpegFailed { code: Option<i32>, stderr: String },

    #[error("no frames to encode (the timeline produced 0 frames)")]
    NoFrames,

    #[error("render cancelled")]
    Cancelled,

    #[error("image error: {0}")]
    Image(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// 全 crate 通用的便捷别名。
pub type Result<T> = std::result::Result<T, Error>;
