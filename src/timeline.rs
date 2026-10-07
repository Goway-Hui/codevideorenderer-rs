//! 打字时间轴。
//!
//! 原始实现每输入一个字符就调用一次 `scene.play()`，导致动画库把整个静态层重建了
//! N 次：线性的输出却做了平方级的工作。这里改为一次性把输入顺序解析成一张出现时间
//! 表，因此渲染第 *n* 帧只是一次查表，每一帧彼此独立。

use crate::camera::{self, CameraKey, CameraTrack, REFERENCE_HEIGHT};
use crate::config::{
    DEFAULT_CAMERA_SCALE, DEFAULT_ENTRANCE_RUN_TIME, DEFAULT_LINE_BREAK_RUN_TIME,
    DEFAULT_TYPE_INTERVAL,
};
use crate::layout::Layout;

/// 打字时间轴的调优参数。
#[derive(Debug, Clone)]
pub struct TimelineOptions {
    /// 两个字符之间的最短延迟，秒。
    pub interval_min: f32,
    /// 两个字符之间的最长延迟，秒。
    pub interval_max: f32,
    /// 相机移动到新行所需的时间，秒。
    pub line_break_run_time: f32,
    /// 相机入场动画时长，秒。
    pub entrance_run_time: f32,
    /// 代码输入完成后保持停留的时长，秒。
    pub end_pause: f32,
    /// 初始相机视野比例。
    pub camera_scale: f32,
    /// 输出帧率。
    pub fps: u32,
    /// 逐字符间隔抖动的种子（保证输出确定性）。
    pub seed: u64,
    /// 让相机停留在每个关键帧上，而不是在关键帧之间平滑滑动。
    ///
    /// 渲染更省力、画面也更稳定，但打字过程中相机不再平滑移动，因此默认关闭。
    pub snap_camera: bool,
}

impl Default for TimelineOptions {
    fn default() -> Self {
        Self {
            interval_min: DEFAULT_TYPE_INTERVAL,
            interval_max: DEFAULT_TYPE_INTERVAL,
            line_break_run_time: DEFAULT_LINE_BREAK_RUN_TIME,
            entrance_run_time: DEFAULT_ENTRANCE_RUN_TIME,
            end_pause: 1.0,
            camera_scale: DEFAULT_CAMERA_SCALE,
            fps: 60,
            seed: 0x5eed_1234_abcd_0001,
            snap_camera: false,
        }
    }
}

/// 构建完成的打字时间轴。
#[derive(Debug, Clone)]
pub struct Timeline {
    /// 每个已输入字符的出现时间，按输入顺序索引。
    pub reveal_times: Vec<f32>,
    /// 每个已输入字符所在行的索引。
    pub line_of_char: Vec<u32>,
    /// 视频总时长，秒（含结尾停顿）。
    pub duration: f32,
    /// 帧率。
    pub fps: u32,
    /// 需要渲染的帧数。
    pub frame_count: u32,
    /// 烘焙好的相机路径。
    pub camera: CameraTrack,
    /// 结尾停顿开始的时间。
    pub typing_end: f32,
}

impl Timeline {
    /// 为某个布局构建时间轴。
    pub fn build(layout: &Layout, opts: &TimelineOptions) -> Self {
        let mut rng = Lcg::new(opts.seed);
        let mut keys: Vec<CameraKey> = Vec::with_capacity(layout.typed_chars + 2);
        let mut reveal_times: Vec<f32> = Vec::with_capacity(layout.typed_chars);
        let mut line_of_char: Vec<u32> = Vec::with_capacity(layout.typed_chars);

        let gutter_center = (layout.bar_left + layout.code_left) * 0.5;
        let start_x = layout.cursor_start_x;
        let first_line_y = layout.lines.first().map(|l| l.baseline_y).unwrap_or(0.0);
        let mut cam_scale = opts.camera_scale;

        // 入场：从上方落到第一行。
        keys.push(CameraKey {
            time: 0.0,
            x: start_x,
            y: first_line_y - 3.0 * layout.cell_h,
            scale: cam_scale,
        });
        keys.push(CameraKey {
            time: opts.entrance_run_time,
            x: start_x,
            y: first_line_y,
            scale: cam_scale,
        });

        let mut t = opts.entrance_run_time;

        for line in &layout.lines {
            let Some(first) = line.first_typed_index else {
                continue;
            };
            let end = line.end_typed_index;
            let chars_in_line = end.saturating_sub(first);
            if chars_in_line == 0 {
                continue;
            }

            // 换到新行时配有一次独立的相机移动。
            if line.index != 0 {
                let x = keys.last().map(|k| k.x).unwrap_or(start_x);
                keys.push(CameraKey {
                    time: t,
                    x,
                    y: line.baseline_y,
                    scale: cam_scale,
                });
                t += opts.line_break_run_time;
            }

            for (offset, index) in (first..end).enumerate() {
                t += rng.range(opts.interval_min, opts.interval_max);
                reveal_times.push(t);
                line_of_char.push(line.index as u32);

                let cursor_x = layout.cursor_x.get(index).copied().unwrap_or(start_x);
                let visible_height = REFERENCE_HEIGHT * cam_scale;
                let sway = camera::sway_offset(offset, chars_in_line, visible_height);
                // 这里 Y 轴向下增长，而原始实现中的 `UP` 为负值。
                let target_y = line.baseline_y - sway;
                cam_scale = camera::required_scale(cursor_x, gutter_center, cam_scale);
                keys.push(CameraKey {
                    time: t,
                    x: cursor_x,
                    y: target_y,
                    scale: cam_scale,
                });
            }
        }

        let typing_end = t;
        let duration = typing_end + opts.end_pause;
        let frame_count = ((duration * opts.fps as f32).ceil() as u32).max(1);
        let camera = if opts.snap_camera {
            CameraTrack::new(keys).snapped_from(opts.entrance_run_time)
        } else {
            CameraTrack::new(keys)
        };

        Self {
            reveal_times,
            line_of_char,
            duration,
            fps: opts.fps,
            frame_count,
            camera,
            typing_end,
        }
    }

    /// 某帧索引的呈现时间。
    pub fn time_of_frame(&self, frame: u32) -> f32 {
        frame as f32 / self.fps as f32
    }

    /// 到时间 `t` 为止已经输入了多少个字符。
    pub fn typed_count_at(&self, t: f32) -> usize {
        self.reveal_times.partition_point(|rt| *rt <= t)
    }

    /// 当给定输入索引对应的字符可见时返回 `true`。
    pub fn is_visible(&self, index: usize, t: f32) -> bool {
        self.reveal_times
            .get(index)
            .is_some_and(|reveal| *reveal <= t)
    }

    /// 时间 `t` 时正在输入的行索引（输入开始前为 0）。
    pub fn current_line_at(&self, t: f32) -> u32 {
        let typed = self.typed_count_at(t);
        if typed == 0 {
            return 0;
        }
        self.line_of_char
            .get(typed - 1)
            .copied()
            .unwrap_or_else(|| self.line_of_char.last().copied().unwrap_or(0))
    }

    /// 光栅化器在时间 `t` 从时间轴读取的全部状态。
    ///
    /// 一帧是该状态的纯函数，因此签名相等的两帧会光栅化出*完全相同*的像素——
    /// 这正是渲染器可以只画一帧、并在另一帧上复用的安全性所在。
    pub fn signature_at(&self, t: f32) -> FrameSignature {
        let cam = self.camera.at(t);
        FrameSignature {
            // 用原始位表示：相机每帧移动的幅度不到一个像素，因此基于容差的比较
            // 会悄然复用了错误的帧。
            camera: (cam.x.to_bits(), cam.y.to_bits(), cam.scale.to_bits()),
            current_line: self.current_line_at(t),
            typed: self.typed_count_at(t),
        }
    }

    /// 某帧索引对应的 [`Timeline::signature_at`]。
    pub fn signature_of_frame(&self, frame: u32) -> FrameSignature {
        self.signature_at(self.time_of_frame(frame))
    }
}

/// 一帧渲染的完整输入状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSignature {
    /// 相机中心 X、中心 Y 与视野，以 `f32` 的原始位表示。
    pub camera: (u32, u32, u32),
    /// 光标所在行（驱动高亮条和当前行的行号）。
    pub current_line: u32,
    /// 目前已揭示的字符数（驱动字形与光标）。
    pub typed: usize,
}

/// 一个小型的确定性随机数生成器，使同一种子总是产出相同的视频。
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u32(&mut self) -> u32 {
        // 类似 SplitMix64；对间隔抖动来说已经够用。
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }

    fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        if hi <= lo {
            return lo;
        }
        lo + (hi - lo) * self.next_f32()
    }
}
