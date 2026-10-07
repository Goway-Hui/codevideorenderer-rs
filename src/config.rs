//! 默认常量，对应原项目的 `config.py`。
//!
//! 原库不得不绕过若干 Manim 行为（文本布局偏移、内部空格的占位字符、硬编码的帧宽度）。
//! 那些绕过手段在这里**并不**需要，因为这个渲染器端到端地掌控着自己的文本布局与光栅化。
//! 因此下面的常量只承载那些真正属于*设计*取舍的部分。

/// 行间额外间距，以字体大小的比例表示。
pub const DEFAULT_LINE_SPACING: f32 = 0.8;

/// 光标高度，以 Manim 世界单位计（1 单位 = 帧高度 / 8）。
pub const DEFAULT_CURSOR_HEIGHT_UNITS: f32 = 0.35;

/// 1080p 下光标插入符的像素宽度（原版用 4px 描边画了一个细线矩形，所以实际显示在屏幕上的
/// 大约是 4px）。
pub const DEFAULT_CURSOR_WIDTH_PX: f32 = 4.0;

/// 两个键入字符之间的默认间隔，单位秒。
pub const DEFAULT_TYPE_INTERVAL: f32 = 0.15;

/// “移动到下一行”相机动画的时长，单位秒。
pub const DEFAULT_LINE_BREAK_RUN_TIME: f32 = 0.4;

/// 相机入场动画时长，单位秒。
pub const DEFAULT_ENTRANCE_RUN_TIME: f32 = 1.0;

/// 相机初始视野，以默认帧宽的比例表示。
/// 值越小，放大越强。
pub const DEFAULT_CAMERA_SCALE: f32 = 0.5;

/// 展开制表符时使用的制表符宽度。
pub const DEFAULT_TAB_WIDTH: usize = 4;

/// Manim 的默认帧高度，以世界单位计。原项目隐式地依赖它（其 `14.22` 魔数恰好是
/// `8 * 16/9`）。
pub const FRAME_HEIGHT_UNITS: f32 = 8.0;

/// 每个世界单位的点数（Manim 的帧高 8 英寸，且 1 英寸 = 72pt）。
pub const POINTS_PER_UNIT: f32 = 72.0;

/// 默认代码字号，单位点——即 Manim `Paragraph` 的默认值。
pub const DEFAULT_FONT_SIZE_PT: f32 = 24.0;

/// 被原渲染器拒绝的字符，因为它们会破坏文本布局。
pub const NOT_AVAILABLE_CHARACTERS: [char; 3] = ['\r', '\x0B', '\x0C'];

/// 默认输出文件主名。
pub const DEFAULT_VIDEO_NAME: &str = "CameraFollowCursorCV";

/// 行号栏与代码之间的间隙，以格计。
pub const GUTTER_GAP_CELLS: f32 = 2.0;

/// 水平内边距，以格计，应用于内容两侧。
pub const PADDING_CELLS: f32 = 2.0;

/// 相机摆动：每次完整振荡对应的键入字符数。
pub const SWAY_CHARS_PER_WAVE: f32 = 15.0;

/// 相机摆动：振幅，以可见高度的比例表示。
pub const SWAY_AMPLITUDE_RATIO: f32 = 0.025;

/// 输出帧的像素上限（8K，7680x4320）。
///
/// 任何更大的值都是笔误，而它过去导致的失败是光栅器深处的一次分配中止，而非类型化错误。
pub const MAX_PIXELS: u64 = 7680 * 4320;

/// 帧率上限。打字视频根本用不到接近这个值的帧率；设此上限是为了防止一个笔误
/// 要求数百万帧。
pub const MAX_FPS: u32 = 1000;

/// x264 接受的最高恒定速率因子。
pub const MAX_CRF: u8 = 51;
