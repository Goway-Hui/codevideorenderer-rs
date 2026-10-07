//! 帧接收器（Frame sinks）：一个 `ffmpeg` 管道，或用于没有 ffmpeg 的机器的 PNG 序列。
//!
//! 原版先用 Manim 写出视频，再用 MoviePy 解码完成的 MP4，用 PIL 对每一帧做模糊后重新编码
//! ——整整两轮编码外加一轮解码。这里帧直接从内存进入单一编码器。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use tiny_skia::{IntSize, Pixmap};

use crate::error::{Error, Result};

/// 编码设置。
#[derive(Debug, Clone)]
pub struct EncodeOptions {
    /// 帧率。
    pub fps: u32,
    /// 帧宽度，单位像素。
    pub width: u32,
    /// 帧高度，单位像素。
    pub height: u32,
    /// 恒定速率因子（越低质量越好）。
    pub crf: u8,
    /// 编码器预设。
    pub preset: String,
    /// 传给 ffmpeg 的视频编码器。
    pub codec: String,
    /// ffmpeg 可执行文件名或路径。
    pub ffmpeg: String,
    /// 在输出路径上应用 glow 滤镜链。
    pub glow: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            fps: 60,
            width: 1920,
            height: 1080,
            crf: 18,
            preset: "veryfast".into(),
            codec: "libx264".into(),
            ffmpeg: "ffmpeg".into(),
            glow: false,
        }
    }
}

/// 原版的 glow 后处理，以单条 ffmpeg 滤镜链的形式实现。
///
/// Python 把它作为对已完成 MP4 的第二轮处理（解码、在 PIL 中对每一帧做模糊、再重新编码）。
/// 这里它只是输出路径上的一条滤镜：把帧一分为二，对其中一份做模糊和提亮，再把两份以
/// screen 方式合成回去 —— 观感相同，但仍只需一次编码。
///
/// 开头的 `format=gbrp` 是承重墙而非装饰：当 ffmpeg 在 `eq` 与 `blend` 之间自行协商像素
/// 格式时，结果是一帧洋红色。把两端都钉在 planar RGB 上正是避免它的办法（已通过对比渲染帧
/// 在 ffmpeg 8.0 上验证）。
pub const GLOW_FILTER: &str = concat!(
    "format=gbrp,",
    "split[a][b];",
    "[b]gblur=sigma=10,eq=brightness=0.06:saturation=2[a2];",
    "[a][a2]blend=all_mode=screen"
);

/// 传给 ffmpeg 的参数列表（除程序名外的所有内容）。
fn ffmpeg_args(opts: &EncodeOptions, output: &Path) -> Vec<std::ffi::OsString> {
    let size = format!("{}x{}", opts.width, opts.height);
    let rate = opts.fps.to_string();
    let mut args: Vec<std::ffi::OsString> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pixel_format",
        "rgba",
        "-video_size",
        &size,
        "-framerate",
        &rate,
        "-i",
        "pipe:0",
        "-an",
    ]
    .iter()
    .map(std::ffi::OsString::from)
    .collect();
    if opts.glow {
        args.push("-vf".into());
        args.push(GLOW_FILTER.into());
    }
    args.extend(
        [
            "-c:v",
            opts.codec.as_str(),
            "-preset",
            opts.preset.as_str(),
            "-crf",
        ]
        .iter()
        .map(std::ffi::OsString::from),
    );
    args.push(opts.crf.to_string().into());
    args.extend(
        ["-pix_fmt", "yuv420p", "-movflags", "+faststart"]
            .iter()
            .map(std::ffi::OsString::from),
    );
    args.push(output.into());
    args
}

/// 渲染帧的去向。
///
/// 实现它即可把帧送到内置 sink 无法到达的地方——对象存储、套接字、调用方自有的编码器——
/// 并将其交给 [`crate::api::render_with_sink`]。
pub trait FrameSink: Send {
    /// 推入一帧渲染结果。帧按顺序到达，且来自单一线程。
    fn write(&mut self, frame_index: u32, pixmap: &Pixmap) -> Result<()>;

    /// 刷新并收尾。返回的字符串会成为 [`crate::api::RenderReport::output`]。
    fn finish(self: Box<Self>) -> Result<String>;
}

/// 通过管道送入 `ffmpeg -f rawvideo` 的帧。
pub struct FfmpegSink {
    child: Child,
    stdin: Option<ChildStdin>,
    output: PathBuf,
    bytes_written: u64,
}

impl FfmpegSink {
    /// 启动 ffmpeg 并打开 raw-video 管道。
    ///
    /// 没有前置的 `ffmpeg -version` 探测：启动本身就是检查，缺失的二进制会被映射为下方的
    /// [`Error::FfmpegMissing`]。这样每次渲染省下一次进程启动，在循环渲染时尤其重要。
    pub fn spawn(output: &Path, opts: &EncodeOptions) -> Result<Self> {
        if let Some(parent) = output.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut child = Command::new(&opts.ffmpeg)
            .args(ffmpeg_args(opts, output))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => Error::FfmpegMissing,
                _ => Error::FfmpegStart(e),
            })?;

        let stdin = child.stdin.take().ok_or(Error::FfmpegMissing)?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            output: output.to_path_buf(),
            bytes_written: 0,
        })
    }

    fn write_frame(&mut self, pixmap: &Pixmap) -> Result<()> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(Error::NoFrames);
        };
        stdin.write_all(pixmap.data())?;
        self.bytes_written += pixmap.data().len() as u64;
        Ok(())
    }

    fn finalize(mut self) -> Result<String> {
        drop(self.stdin.take());
        let output = self.child.wait_with_output().map_err(Error::FfmpegStart)?;
        if !output.status.success() {
            return Err(Error::FfmpegFailed {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(self.output.display().to_string())
    }
}

impl FrameSink for FfmpegSink {
    fn write(&mut self, _frame_index: u32, pixmap: &Pixmap) -> Result<()> {
        self.write_frame(pixmap)
    }

    fn finish(self: Box<Self>) -> Result<String> {
        self.finalize()
    }
}

/// 以 PNG 序列写出的帧（无需外部工具）。
///
/// PNG 压缩是这条路径上最昂贵的一环——实测占端到端运行的 83–89%——而且它是天然并行的，
/// 因此帧被送往一个小型的编码器线程池：`write` 拷贝像素并入队，`finish` 排空队列。这次拷贝
/// 正是让渲染器的缓冲池持续运转的关键，因为它把同一块缓冲区交还用于下一帧。
pub struct PngSink {
    dir: PathBuf,
    /// 一旦 sink 被 finish 或 drop，即为 `None`。
    queue: Option<SyncSender<Job>>,
    workers: Vec<JoinHandle<()>>,
    failure: Arc<Mutex<Option<Error>>>,
    threads: usize,
}

/// 等待编码器池处理的一帧。
struct Job {
    index: u32,
    size: IntSize,
    data: Vec<u8>,
}

impl PngSink {
    /// 创建输出目录，并为每个核心创建一个编码器线程。
    pub fn new(dir: &Path) -> Result<Self> {
        Self::with_threads(dir, encoder_threads())
    }

    /// 以显式的编码器线程数创建 sink。
    ///
    /// 单线程复现原版的串行行为。更多线程以内存换取吞吐：队列最多容纳 `threads` 帧未压缩的
    /// 帧，在 1080p 下每帧 8 MB。
    pub fn with_threads(dir: &Path, threads: usize) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let threads = threads.max(1);
        let (queue, receiver) = sync_channel::<Job>(threads);
        // std 的 receiver 是单消费者的，但 worker 只在取任务时持锁；
        // 压缩本身在锁外进行。
        let receiver = Arc::new(Mutex::new(receiver));
        let failure: Arc<Mutex<Option<Error>>> = Arc::new(Mutex::new(None));
        let mut workers = Vec::with_capacity(threads);

        for _ in 0..threads {
            let receiver = Arc::clone(&receiver);
            let failure = Arc::clone(&failure);
            let dir = dir.to_path_buf();
            workers.push(std::thread::spawn(move || {
                loop {
                    let job = match receiver.lock().expect("PNG queue poisoned").recv() {
                        Ok(job) => job,
                        Err(_) => break, // sink 已被 finish
                    };
                    if failure.lock().expect("PNG error slot poisoned").is_some() {
                        // 已在失败中：只排空队列，不再做更多工作。
                        continue;
                    }
                    let path = dir.join(format!("frame_{:05}.png", job.index));
                    let Some(pixmap) = Pixmap::from_vec(job.data, job.size) else {
                        record_failure(
                            &failure,
                            Error::Image(format!("{}: bad frame buffer", path.display())),
                        );
                        continue;
                    };
                    if let Err(error) = pixmap.save_png(&path) {
                        record_failure(
                            &failure,
                            Error::Image(format!("failed to write {}: {error}", path.display())),
                        );
                    }
                }
            }));
        }

        Ok(Self {
            dir: dir.to_path_buf(),
            queue: Some(queue),
            workers,
            failure,
            threads,
        })
    }

    fn write_frame(&mut self, frame_index: u32, pixmap: &Pixmap) -> Result<()> {
        if let Some(failure) = self.failure.lock().expect("PNG error slot poisoned").take() {
            return Err(failure);
        }
        let job = Job {
            index: frame_index,
            size: IntSize::from_wh(pixmap.width(), pixmap.height())
                .ok_or_else(|| Error::Image("frame has a zero side".into()))?,
            data: pixmap.data().to_vec(),
        };
        match self.queue.as_ref().expect("PNG queue open").send(job) {
            Ok(()) => Ok(()),
            // worker 已消失，这种情况只会在某个 worker 记录了原因之后发生。
            Err(_) => Err(self
                .failure
                .lock()
                .expect("PNG error slot poisoned")
                .take()
                .unwrap_or(Error::NoFrames)),
        }
    }

    /// 关闭队列并等待编码器排空它。
    fn drain(&mut self) -> Result<()> {
        drop(self.queue.take());
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        match self.failure.lock().expect("PNG error slot poisoned").take() {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }

    /// 该 sink 运行的编码器线程数。
    pub fn threads(&self) -> usize {
        self.threads
    }
}

impl Drop for PngSink {
    fn drop(&mut self) {
        // 一个未调用 `finish` 就被 drop 的 sink（比如一次被取消的渲染）仍然必须
        // 停掉它的线程并关闭队列。
        let _ = self.drain();
    }
}

impl FrameSink for PngSink {
    fn write(&mut self, frame_index: u32, pixmap: &Pixmap) -> Result<()> {
        self.write_frame(frame_index, pixmap)
    }

    fn finish(mut self: Box<Self>) -> Result<String> {
        self.drain()?;
        Ok(format!(
            "{} (PNG sequence, {} encoder threads)",
            self.dir.display(),
            self.threads
        ))
    }
}

/// 只保留第一个失败；后续都是同一个磁盘问题在重复。
fn record_failure(slot: &Mutex<Option<Error>>, error: Error) {
    let mut slot = slot.lock().expect("PNG error slot poisoned");
    if slot.is_none() {
        *slot = Some(error);
    }
}

/// 每个核心一个编码器线程，PNG 压缩正应在此处进行。
fn encoder_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// 极简的单行进度报告器。
pub struct Progress {
    enabled: bool,
    total: u32,
    done: u32,
    last_percent: u32,
    last_len: usize,
}

impl Progress {
    /// 为 `total` 帧创建报告器；`enabled == false` 时它什么都不做。
    pub fn new(total: u32, enabled: bool) -> Self {
        Self {
            enabled,
            total,
            done: 0,
            last_percent: u32::MAX,
            last_len: 0,
        }
    }

    /// 报告一步进度。
    pub fn tick(&mut self) {
        self.done += 1;
        if !self.enabled || self.total == 0 {
            return;
        }
        let percent = (self.done as u64 * 100 / self.total as u64) as u32;
        if percent == self.last_percent {
            return;
        }
        self.last_percent = percent;
        let text = format!(
            "  rendering {percent:>3}%  ({} / {})",
            self.done, self.total
        );
        let pad = " ".repeat(self.last_len.saturating_sub(text.len()));
        print!("\r{text}{pad}");
        let _ = std::io::stdout().flush();
        self.last_len = text.len();
    }

    /// 清除进度行。
    pub fn clear(&mut self) {
        if !self.enabled {
            return;
        }
        if self.last_len > 0 {
            print!("\r{}\r", " ".repeat(self.last_len));
            let _ = std::io::stdout().flush();
            self.last_len = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(opts: &EncodeOptions) -> Vec<String> {
        ffmpeg_args(opts, Path::new("out.mp4"))
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_png_sink_reports_its_encoder_threads() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("cvr-png-threads-{}", std::process::id()));
        assert_eq!(PngSink::with_threads(&dir, 3).expect("sink").threads(), 3);
        assert_eq!(
            PngSink::with_threads(&dir, 0).expect("sink").threads(),
            1,
            "zero threads means one"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// drop 一个未 finish 的 sink 仍必须停掉其 worker。
    #[test]
    fn dropping_a_png_sink_joins_its_workers() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("cvr-png-drop-{}", std::process::id()));
        let sink = PngSink::with_threads(&dir, 2).expect("sink");
        drop(sink);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_glow_filter_is_handed_to_ffmpeg() {
        let args = args_of(&EncodeOptions {
            glow: true,
            ..EncodeOptions::default()
        });
        let flag = args.iter().position(|a| a == "-vf").expect("-vf");
        assert_eq!(args[flag + 1], GLOW_FILTER);
        // `-vf` 是输出选项：它必须位于输入之后。
        let input = args.iter().position(|a| a == "-i").expect("-i");
        assert!(flag > input);
    }

    /// 回归防护：不固定像素格式时，ffmpeg 在 `eq` 与 `blend` 之间自行协商会让整帧变成
    /// 洋红色（在 8.0 上复现）。
    #[test]
    fn the_glow_filter_pins_its_pixel_format() {
        assert!(
            GLOW_FILTER.starts_with("format="),
            "the chain must pin its pixel format, got {GLOW_FILTER}"
        );
    }

    #[test]
    fn without_glow_there_is_no_filter_chain() {
        let args = args_of(&EncodeOptions::default());
        assert!(!args.iter().any(|a| a == "-vf"));
    }

    #[test]
    fn the_encoder_settings_reach_the_command_line() {
        let args = args_of(&EncodeOptions {
            fps: 30,
            width: 1280,
            height: 720,
            crf: 23,
            preset: "slow".into(),
            ..EncodeOptions::default()
        });
        let value_after = |flag: &str| {
            let at = args.iter().position(|a| a == flag).expect(flag);
            args[at + 1].clone()
        };
        assert_eq!(value_after("-video_size"), "1280x720");
        assert_eq!(value_after("-framerate"), "30");
        assert_eq!(value_after("-crf"), "23");
        assert_eq!(value_after("-preset"), "slow");
        assert_eq!(args.last().expect("output"), "out.mp4");
    }
}
