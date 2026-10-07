//! 相机模型——刻意设计为*时间的纯函数*。
//!
//! 帧是并行渲染的，因此 camera 绝不能跨帧携带状态。取而代之的是，整条 camera
//! 路径预先烘焙为 keyframe，并用二分查找采样。正是这一结构性差异让 Rust 渲染器
//! 能利用所有核心：原版则是在串行帧循环中移动一个有状态的 camera 对象。
//!
//! 以下三种行为是原版所实现的：
//!
//! * **entrance**——camera 落到第一行，
//! * **follow**——camera 中心跟随光标，
//! * **auto zoom-out**——视野随光标右移而加宽，使行首始终不离开画面。

use crate::config::{DEFAULT_CAMERA_SCALE, SWAY_AMPLITUDE_RATIO, SWAY_CHARS_PER_WAVE};

/// 参考帧宽度，以世界像素计（代码按 1080p 比例排版）。
pub const REFERENCE_WIDTH: f32 = 1920.0;

/// 参考帧高度，以世界像素计。
pub const REFERENCE_HEIGHT: f32 = 1080.0;

/// 一个 camera keyframe。
#[derive(Debug, Clone, Copy)]
pub struct CameraKey {
    /// 到达该关键点的时间，以秒计。
    pub time: f32,
    /// camera 中心 X，世界像素。
    pub x: f32,
    /// camera 中心 Y，世界像素。
    pub y: f32,
    /// 视野，以 [`REFERENCE_WIDTH`] 的比例表示。越小则越近。
    pub scale: f32,
}

/// 采样得到的 camera 状态。
#[derive(Debug, Clone, Copy)]
pub struct CameraState {
    /// camera 中心 X，世界像素。
    pub x: f32,
    /// camera 中心 Y，世界像素。
    pub y: f32,
    /// 视野比例。
    pub scale: f32,
}

/// 烘焙好的 camera 路径。
#[derive(Debug, Clone, Default)]
pub struct CameraTrack {
    keys: Vec<CameraKey>,
    /// 一旦设置，从该时间起，camera 会*保持*上一个 keyframe，而不是向下一个
    /// 插值。
    snap_from: Option<f32>,
}

impl CameraTrack {
    /// 由 keyframe 构建一条轨道（它们必须按时间排序）。
    pub fn new(keys: Vec<CameraKey>) -> Self {
        Self {
            keys,
            snap_from: None,
        }
    }

    /// 保持每个 keyframe 直到下一个，而不是在二者之间滑动。
    ///
    /// 滑动的 camera 每一帧都会改变画面，因此没有任何两帧相同；保持的 camera
    /// 会在键入一个字符所需的整段时间内重复同一画面，这正是渲染器得以复用帧的
    /// 原因。`from` 是 entrance 结束的时刻，因此落到第一行的动作仍然平滑。
    pub fn snapped_from(mut self, from: f32) -> Self {
        self.snap_from = Some(from);
        self
    }

    /// 当轨道保持 keyframe 而非插值时为 `true`。
    pub fn is_snapped(&self) -> bool {
        self.snap_from.is_some()
    }

    /// keyframe 的数量。
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// 当轨道没有任何 keyframe 时为 `true`。
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// 在时间 `t` 处采样 camera，在 keyframe 之间线性插值。
    pub fn at(&self, t: f32) -> CameraState {
        let fallback = CameraState {
            x: 0.0,
            y: 0.0,
            scale: DEFAULT_CAMERA_SCALE,
        };
        if self.keys.is_empty() {
            return fallback;
        }
        if let Some(from) = self.snap_from {
            if t >= from {
                let idx = self.keys.partition_point(|k| k.time <= t);
                let k = self.keys[idx.saturating_sub(1)];
                return CameraState {
                    x: k.x,
                    y: k.y,
                    scale: k.scale,
                };
            }
        }
        let idx = self.keys.partition_point(|k| k.time <= t);
        if idx == 0 {
            let k = self.keys[0];
            return CameraState {
                x: k.x,
                y: k.y,
                scale: k.scale,
            };
        }
        if idx >= self.keys.len() {
            let k = self.keys[self.keys.len() - 1];
            return CameraState {
                x: k.x,
                y: k.y,
                scale: k.scale,
            };
        }
        let a = self.keys[idx - 1];
        let b = self.keys[idx];
        let span = (b.time - a.time).max(1e-6);
        let u = ((t - a.time) / span).clamp(0.0, 1.0);
        CameraState {
            x: a.x + (b.x - a.x) * u,
            y: a.y + (b.y - a.y) * u,
            scale: a.scale + (b.scale - a.scale) * u,
        }
    }
}

/// 针对一个被键入字符，施加到 camera 目标上的垂直晃动。
///
/// 与原始实现相同的数学：一个在行两端为零的正弦包络，乘以一个频率取决于行长
/// （每 [`SWAY_CHARS_PER_WAVE`] 个字符一个完整周期）的振荡，再缩放到可见高度的
/// 一小部分。
pub fn sway_offset(index_in_line: usize, chars_in_line: usize, visible_height: f32) -> f32 {
    if chars_in_line <= 1 {
        return 0.0;
    }
    let alpha = index_in_line as f32 / (chars_in_line - 1) as f32;
    let envelope = (alpha * std::f32::consts::PI).sin();
    let wave_count = chars_in_line as f32 / SWAY_CHARS_PER_WAVE;
    let oscillation = (alpha * wave_count * 2.0 * std::f32::consts::PI).sin();
    let amplitude = visible_height * SWAY_AMPLITUDE_RATIO;
    amplitude * envelope * oscillation
}

/// 缩小决策：把视野恰好加宽到能让行号槽留在画面内。原版从不重新拉近，我们
/// 亦然。
pub fn required_scale(camera_x: f32, gutter_center_x: f32, current_scale: f32) -> f32 {
    let distance = (camera_x - gutter_center_x) / REFERENCE_WIDTH;
    if distance > current_scale {
        distance
    } else {
        current_scale
    }
}
