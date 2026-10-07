//! 字体加载、字形轮廓与度量信息。
//!
//! 轮廓只提取一次，以 *font units*（字体单位）表示，并翻转 Y 轴，使渲染器能在
//! 普通的“Y 轴向下增长”坐标系中工作。由于轮廓与缩放无关，每个 glyph 只需精确
//! 计算一次，之后在每一帧、每一种相机缩放级别下复用——这正是让每帧开销随 glyph
//! 数量呈线性增长（而非二次增长）的原因。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use tiny_skia::{Path, PathBuilder};
use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::error::{Error, Result};

/// 原始字体字节。故意泄漏：`Face` 需要一个 `'static` 借用，而字体本来就随进程
/// 存活到结束（见下方缓存）。
type FontBytes = &'static [u8];

/// 进程级字体缓存：解析字体过去在 *每次* `render()` 时都要从磁盘读取约 13 MB
/// 并重新解析，在批量使用中纯属浪费。
///
/// `Face<'static>` 实现了 `Clone`（即解析后表格的浅拷贝），因此调用方保留它们
/// 一直拿到的 `Font` 值，而可读字节与解析后的表格在进程内每次渲染间共享。
static CACHE: OnceLock<Mutex<HashMap<PathBuf, &'static Font>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<PathBuf, &'static Font>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 编译进二进制（`embed-font`）的字体所用的缓存键，它在磁盘上没有路径。
/// 这不是合法的 Windows 路径，因此不会与真实路径冲突。
#[cfg(feature = "embed-font")]
const EMBEDDED_KEY: &str = "<embedded CodeVideoRendererFont>";

/// 已解析的字体 face 及其度量信息。
#[derive(Clone)]
pub struct Font {
    face: Face<'static>,
    /// 每个 em 对应的字体设计单位数。
    upem: f32,
    origin: String,
}

impl Font {
    /// 从存活时间超过进程的内存缓冲区中解析字体。
    pub fn from_static(bytes: FontBytes, origin: impl Into<String>) -> Result<Self> {
        let origin = origin.into();
        let face = Face::parse(bytes, 0).map_err(|e| Error::FontParse {
            path: origin.clone(),
            reason: e.to_string(),
        })?;
        let upem = face.units_per_em() as f32;
        if upem <= 0.0 {
            return Err(Error::FontParse {
                path: origin,
                reason: "units_per_em is zero".into(),
            });
        }
        Ok(Self { face, upem, origin })
    }

    /// 从磁盘加载字体，尽可能复用进程级缓存。
    ///
    /// 对某个路径的第一次调用会读取并解析文件；之后的调用只克隆已解析的
    /// face，不再触碰磁盘。
    pub fn load(path: &std::path::Path) -> Result<Self> {
        Ok((*Self::load_cached(path)?).clone())
    }

    fn load_cached(path: &std::path::Path) -> Result<&'static Font> {
        // 规范化路径可让 `--font ./x.ttf` 与 `--font x.ttf` 命中同一条目。
        // 若规范化失败，说明字体本来就缺失，下方的读取会报告该错误。
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut cache = cache().lock().expect("font cache poisoned");
        if let Some(font) = cache.get(&key) {
            return Ok(font);
        }

        let data = std::fs::read(path)?;
        let boxed = data.into_boxed_slice();
        // 泄漏前先校验：解析失败的字体不能白白占用与自身大小相当的常驻内存。
        if let Err(reason) = Face::parse(&boxed, 0) {
            return Err(Error::FontParse {
                path: path.display().to_string(),
                reason: reason.to_string(),
            });
        }
        let leaked: FontBytes = Box::leak(boxed);
        let font: &'static Font = Box::leak(Box::new(Self::from_static(
            leaked,
            path.display().to_string(),
        )?));
        cache.insert(key, font);
        Ok(font)
    }

    /// 编译进二进制（`embed-font`）的字体，与其他字体一样被缓存。
    #[cfg(feature = "embed-font")]
    fn embedded() -> Result<&'static Font> {
        let mut cache = cache().lock().expect("font cache poisoned");
        let key = PathBuf::from(EMBEDDED_KEY);
        if let Some(font) = cache.get(&key) {
            return Ok(font);
        }
        let bytes: FontBytes = include_bytes!("../assets/CodeVideoRendererFont.ttf");
        let font: &'static Font = Box::leak(Box::new(Self::from_static(
            bytes,
            "embedded CodeVideoRendererFont",
        )?));
        cache.insert(key, font);
        Ok(font)
    }

    /// 定位一个可用的等宽字体，并复用进程级缓存。
    ///
    /// 查找顺序：
    /// 1. `explicit`（即 `--font` 参数）
    /// 2. 可执行文件旁的 `assets/CodeVideoRendererFont.ttf`
    /// 3. 相对当前目录的 `assets/CodeVideoRendererFont.ttf`
    /// 4. 内置副本——启用 `embed-font` 特性时该项胜出，步骤 2、3、5 都不会
    ///    被执行，因此损坏的内置字体会被如实报告，而不会被系统字体静默替换
    /// 5. 若干常见的系统等宽字体
    pub fn find(explicit: Option<&std::path::Path>) -> Result<Self> {
        Ok((*Self::find_cached(explicit)?).clone())
    }

    fn find_cached(explicit: Option<&std::path::Path>) -> Result<&'static Font> {
        let mut tried: Vec<String> = Vec::new();

        if let Some(p) = explicit {
            if p.is_file() {
                return Self::load_cached(p);
            }
            tried.push(format!("{} (from --font)", p.display()));
        }

        for candidate in asset_candidates() {
            if candidate.is_file() {
                return Self::load_cached(&candidate);
            }
            tried.push(candidate.display().to_string());
        }

        #[cfg(feature = "embed-font")]
        {
            return Self::embedded();
        }

        #[cfg(not(feature = "embed-font"))]
        {
            for candidate in system_font_candidates() {
                if candidate.is_file() {
                    return Self::load_cached(&candidate);
                }
                tried.push(candidate.display().to_string());
            }

            Err(Error::FontNotFound(tried))
        }
    }

    /// 将字符解析为 glyph id。
    pub fn glyph_index(&self, ch: char) -> Option<GlyphId> {
        self.face.glyph_index(ch)
    }

    /// 某个 glyph 的水平步进宽度，以 em 为单位。
    pub fn advance_em(&self, glyph: GlyphId) -> f32 {
        self.face.glyph_hor_advance(glyph).unwrap_or(0) as f32 / self.upem
    }

    /// 每个 em 对应的设计单位数（缩放轮廓时需要：`px = units * size / upem`）。
    pub fn units_per_em(&self) -> f32 {
        self.upem
    }

    /// 以 em 为单位的 ascender，从基线向上度量（为正）。
    pub fn ascender_em(&self) -> f32 {
        self.face.ascender() as f32 / self.upem
    }

    /// 以 em 为单位的 descender，从基线向下度量（为正）。
    pub fn descender_em(&self) -> f32 {
        -(self.face.descender() as f32) / self.upem
    }

    /// 预期行高（ascender + descender + gap），以 em 为单位。
    pub fn line_height_em(&self) -> f32 {
        let gap = self.face.line_gap() as f32 / self.upem;
        self.ascender_em() + self.descender_em() + gap
    }

    /// 以字体单位表示的 glyph 轮廓，Y 轴已翻转（Y 向下增长）。
    /// 对空格这类空白 glyph 返回 `None`。
    pub fn outline(&self, glyph: GlyphId) -> Option<Path> {
        let mut sink = OutlineSink::default();
        self.face.outline_glyph(glyph, &mut sink)?;
        sink.finish()
    }

    /// 该字体的可读来源（路径或 "embedded"）。
    pub fn origin(&self) -> &str {
        &self.origin
    }
}

fn asset_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    const NAME: &str = "assets/CodeVideoRendererFont.ttf";
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join(NAME));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push(cwd.join(NAME));
    }
    out
}

fn system_font_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        for name in [
            "consola.ttf",
            "CascadiaMono.ttf",
            "CascadiaCode.ttf",
            "lucon.ttf",
        ] {
            out.push(PathBuf::from(&root).join("Fonts").join(name));
        }
    }
    #[cfg(target_os = "macos")]
    {
        for p in [
            "/System/Library/Fonts/Menlo.ttc",
            "/System/Library/Fonts/Monaco.ttf",
            "/System/Library/Fonts/SFNSMono.ttf",
        ] {
            out.push(PathBuf::from(p));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for p in [
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        ] {
            out.push(PathBuf::from(p));
        }
    }
    out
}

/// 将 `ttf-parser` 的轮廓指令收集进一个 `tiny-skia` 路径。
#[derive(Default)]
struct OutlineSink {
    builder: PathBuilder,
    open: bool,
}

impl OutlineSink {
    fn finish(mut self) -> Option<Path> {
        if self.open {
            self.builder.close();
        }
        self.builder.finish()
    }
}

impl OutlineBuilder for OutlineSink {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.move_to(x, -y);
        self.open = true;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.builder.line_to(x, -y);
        }
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        if self.open {
            self.builder.quad_to(x1, -y1, x, -y);
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        if self.open {
            self.builder.cubic_to(x1, -y1, x2, -y2, x, -y);
        }
    }

    fn close(&mut self) {
        if self.open {
            self.builder.close();
        }
    }
}

impl std::fmt::Debug for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Font")
            .field("origin", &self.origin)
            .field("upem", &self.upem)
            .field("ascender_em", &self.ascender_em())
            .field("descender_em", &self.descender_em())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缓存的存在是为了让同一进程内的第二次渲染不必再次读取并解析字体。
    /// 共享同一个泄漏的字节缓冲区正是这个目的。
    #[test]
    fn repeated_lookups_share_one_parsed_font() {
        let first = Font::find(None).expect("a usable font");
        let second = Font::find(None).expect("a usable font");
        assert_eq!(first.origin(), second.origin());
        assert!(
            std::ptr::eq(
                first.face.raw_face().data.as_ptr(),
                second.face.raw_face().data.as_ptr()
            ),
            "the second lookup re-read the font instead of using the cache"
        );
    }

    /// `--font <path>` 与自动查找会命中同一个缓存条目，因为键是规范化后的路径。
    #[test]
    fn an_explicit_path_and_the_default_lookup_agree() {
        let asset = asset_candidates()
            .into_iter()
            .find(|p| p.is_file())
            .expect("assets/CodeVideoRendererFont.ttf");
        let explicit = Font::load(&asset).expect("load");
        let found = Font::find(Some(&asset)).expect("find");
        assert!(std::ptr::eq(
            explicit.face.raw_face().data.as_ptr(),
            found.face.raw_face().data.as_ptr()
        ));
    }

    /// 解析失败的字体必须被报告，而不是被泄漏。
    #[test]
    fn a_broken_font_file_is_reported() {
        let mut path = std::env::temp_dir();
        path.push(format!("cvr-broken-font-{}.ttf", std::process::id()));
        std::fs::write(&path, b"not a font").expect("write");
        let err = Font::load(&path).expect_err("must not parse");
        assert!(matches!(err, Error::FontParse { .. }), "got {err:?}");
        let _ = std::fs::remove_file(&path);
    }
}
