//! 光栅化。
//!
//! 每一帧都用 `tiny-skia` 从头绘制：相机是一个变换，字形轮廓以字体单位缓存一次，
//! 只填充视口内的字形。由于这里没有任何东西会修改共享状态，帧可以在所有核心上
//! 同时渲染。
//!
//! 关于画质：轮廓会在相机*当前*缩放级别下重新填充，而不是采样缓存的位图，
//! 因此相机缩小后文字依然清晰——这也是原始实现从 Cairo 获得的同等保证。

use std::collections::HashMap;

use tiny_skia::{Color, FillRule, Paint, Path, Pixmap, Rect, Shader, Transform};

use crate::camera::REFERENCE_WIDTH;
use crate::config::{DEFAULT_CURSOR_HEIGHT_UNITS, DEFAULT_CURSOR_WIDTH_PX, FRAME_HEIGHT_UNITS};
use crate::font::Font;
use crate::layout::{Glyph, GlyphRole, Layout};
use crate::theme::Theme;
use crate::timeline::Timeline;

/// 一次性预计算的字形轮廓，以字体单位表示，Y 轴已翻转。
pub struct GlyphOutlines {
    paths: HashMap<u16, Path>,
}

impl GlyphOutlines {
    /// 收集布局所需每个字形的轮廓。
    pub fn build(font: &Font, layout: &Layout) -> Self {
        let mut paths: HashMap<u16, Path> = HashMap::with_capacity(layout.glyphs.len());
        for glyph in &layout.glyphs {
            if paths.contains_key(&glyph.glyph_id) {
                continue;
            }
            if let Some(path) = font.outline(ttf_parser::GlyphId(glyph.glyph_id)) {
                paths.insert(glyph.glyph_id, path);
            }
        }
        Self { paths }
    }

    /// 已缓存的轮廓数量。
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// 当没有任何轮廓被缓存时返回 `true`（例如只有空白的代码片段）。
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    fn get(&self, glyph_id: u16) -> Option<&Path> {
        self.paths.get(&glyph_id)
    }
}

/// 绘制帧所需的全部内容。跨线程共享成本很低（仅需 `&self`）。
pub struct FrameRenderer<'a> {
    /// 不可变的代码布局。
    pub layout: &'a Layout,
    /// 打字时间轴与相机路径。
    pub timeline: &'a Timeline,
    /// 颜色主题。
    pub theme: &'a Theme,
    /// 布局阶段使用的字体（计算 `units_per_em` 时需要）。
    pub font: &'a Font,
    /// 缓存的轮廓。
    pub outlines: &'a GlyphOutlines,
    /// 输出宽度，像素。
    pub width: u32,
    /// 输出高度，像素。
    pub height: u32,
    /// 光标宽度，世界像素。
    pub cursor_width: f32,
    /// 光标高度，世界像素。
    pub cursor_height: f32,
}

impl<'a> FrameRenderer<'a> {
    /// 为给定的输出尺寸构建帧渲染器。
    pub fn new(
        layout: &'a Layout,
        timeline: &'a Timeline,
        theme: &'a Theme,
        font: &'a Font,
        outlines: &'a GlyphOutlines,
        width: u32,
        height: u32,
    ) -> Self {
        let scale = height as f32 / 1080.0;
        Self {
            layout,
            timeline,
            theme,
            font,
            outlines,
            width,
            height,
            cursor_width: DEFAULT_CURSOR_WIDTH_PX * scale,
            // 0.35 个 Manim 世界单位；一个单位为 1080/8 = 135 世界像素。
            cursor_height: DEFAULT_CURSOR_HEIGHT_UNITS * (1080.0 / FRAME_HEIGHT_UNITS),
        }
    }

    /// 把呈现时间 `t` 的整帧绘制到 `pixmap` 中。
    pub fn render(&self, t: f32, pixmap: &mut Pixmap) {
        let background = self.theme.background;
        pixmap.fill(Color::from_rgba8(
            background.0,
            background.1,
            background.2,
            255,
        ));

        let cam = self.timeline.camera.at(t);
        let cam_scale = cam.scale.max(1e-4);
        let k = self.width as f32 / (REFERENCE_WIDTH * cam_scale);
        let world_to_screen = Transform::from_row(
            k,
            0.0,
            0.0,
            k,
            self.width as f32 * 0.5 - cam.x * k,
            self.height as f32 * 0.5 - cam.y * k,
        );

        // 视口（世界坐标），用于低成本地剔除字形。
        let half_w = REFERENCE_WIDTH * cam_scale * 0.5;
        let half_h = half_w * (self.height as f32 / self.width.max(1) as f32);
        let margin = self.layout.font_size_px * 2.0;

        let current_line = self.timeline.current_line_at(t);

        self.draw_current_line_bar(current_line, world_to_screen, pixmap);

        let glyph_scale = self.layout.font_size_px / self.font.units_per_em();
        let paint = Paint {
            shader: Shader::SolidColor(Color::WHITE),
            anti_alias: true,
            ..Paint::default()
        };

        for glyph in &self.layout.glyphs {
            if glyph.invisible {
                continue;
            }
            match glyph.role {
                GlyphRole::Code => {
                    let Some(index) = glyph.typed_index else {
                        continue;
                    };
                    if !self.timeline.is_visible(index, t) {
                        continue;
                    }
                }
                GlyphRole::LineNumber { .. } => {
                    if !self.line_number_visible(glyph.line, t) {
                        continue;
                    }
                }
            }

            let dx = glyph.x - cam.x;
            let dy = glyph.baseline_y - cam.y;
            if dx < -half_w - margin || dx > half_w + margin {
                continue;
            }
            if dy < -half_h - margin || dy > half_h + margin {
                continue;
            }

            let Some(path) = self.outlines.get(glyph.glyph_id) else {
                continue;
            };
            let (r, g, b) = self.glyph_color(glyph, current_line);
            let paint = Paint {
                shader: Shader::SolidColor(Color::from_rgba8(r, g, b, 255)),
                anti_alias: true,
                ..paint.clone()
            };
            let glyph_xform = Transform::from_row(
                glyph_scale,
                0.0,
                0.0,
                glyph_scale,
                glyph.x,
                glyph.baseline_y,
            );
            pixmap.fill_path(
                path,
                &paint,
                FillRule::Winding,
                world_to_screen.pre_concat(glyph_xform),
                None,
            );
        }

        self.draw_cursor(t, current_line, world_to_screen, pixmap);
    }

    fn line_number_visible(&self, line: u32, t: f32) -> bool {
        // 当某行的第一个字符被输入时，行号即出现——与原始实现添加 mobject 时
        // 使用的触发时机相同。
        let Some(box_) = self.layout.lines.get(line as usize) else {
            return false;
        };
        match box_.first_typed_index {
            Some(index) => self.timeline.is_visible(index, t),
            None => {
                // 空行：当上一行结束后才显示行号。
                let typed = self.timeline.typed_count_at(t);
                typed >= box_.end_typed_index
            }
        }
    }

    fn glyph_color(&self, glyph: &Glyph, current_line: u32) -> (u8, u8, u8) {
        match glyph.role {
            GlyphRole::Code => glyph.color,
            GlyphRole::LineNumber { line } => {
                if line == current_line {
                    self.theme.line_number_active()
                } else {
                    self.theme.line_number()
                }
            }
        }
    }

    fn draw_current_line_bar(&self, line: u32, transform: Transform, pixmap: &mut Pixmap) {
        let Some(box_) = self.layout.lines.get(line as usize) else {
            return;
        };
        let top = box_.baseline_y - self.layout.cell_h * 0.8;
        let bottom = top + self.layout.cell_h;
        let Some(rect) = Rect::from_ltrb(self.layout.bar_left, top, self.layout.bar_right, bottom)
        else {
            return;
        };
        let (r, g, b) = self.theme.line_highlight();
        let paint = Paint {
            shader: Shader::SolidColor(Color::from_rgba8(r, g, b, 255)),
            anti_alias: false,
            ..Paint::default()
        };
        pixmap.fill_rect(rect, &paint, transform, None);
    }

    fn draw_cursor(&self, t: f32, current_line: u32, transform: Transform, pixmap: &mut Pixmap) {
        let typed = self.timeline.typed_count_at(t);
        let x = if typed == 0 {
            self.layout.cursor_start_x
        } else {
            self.layout
                .cursor_x
                .get(typed - 1)
                .copied()
                .unwrap_or(self.layout.cursor_start_x)
        };
        let Some(box_) = self.layout.lines.get(current_line as usize) else {
            return;
        };
        let center_y = box_.baseline_y - self.font.ascender_em() * self.layout.font_size_px * 0.5;
        let half = self.cursor_height * 0.5;
        let Some(rect) = Rect::from_ltrb(
            x,
            center_y - half,
            x + self.cursor_width.max(1.0),
            center_y + half,
        ) else {
            return;
        };
        let (r, g, b) = self.theme.cursor();
        let paint = Paint {
            shader: Shader::SolidColor(Color::from_rgba8(r, g, b, 255)),
            anti_alias: false,
            ..Paint::default()
        };
        pixmap.fill_rect(rect, &paint, transform, None);
    }

    /// 渲染一帧并返回其 RGBA 字节。
    ///
    /// 为自行管理编码器的调用方（或想在不涉及任何 I/O 的情况下测量光栅化耗时）
    /// 提供便利。尺寸无效时返回 `None`。
    pub fn render_to_rgba(&self, t: f32) -> Option<Vec<u8>> {
        let mut pixmap = Pixmap::new(self.width, self.height)?;
        self.render(t, &mut pixmap);
        Some(pixmap.data().to_vec())
    }

    /// 复用调用方持有的 RGBA 缓冲区渲染一帧，并将缓冲区交回。
    ///
    /// 首次传入一个空的 `Vec`，之后传入上次返回的缓冲区即可。
    /// 复用缓冲区消除了每帧一次的分配——1080p 下为 8 MB——当在多个线程上
    /// 渲染成千上万帧时，这是一笔实实在在的开销。
    pub fn render_into_vec(&self, t: f32, buffer: Vec<u8>) -> Option<Vec<u8>> {
        let size = tiny_skia::IntSize::from_wh(self.width, self.height)?;
        let expected = size.width() as usize * size.height() as usize * 4;
        let buffer = if buffer.len() == expected {
            buffer
        } else {
            vec![0u8; expected]
        };
        let mut pixmap = Pixmap::from_vec(buffer, size)?;
        self.render(t, &mut pixmap);
        Some(pixmap.take())
    }
}
