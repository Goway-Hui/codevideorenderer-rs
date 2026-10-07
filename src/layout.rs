//! 代码预处理与等宽网格布局。
//!
//! 这里正是 Rust 实现相对原版取得最大架构优势的地方。与其让动画库先完成文本
//! 排版、再去修补其内部实现，我们只对代码**一次性**排版，得到一张带显式坐标的
//! 扁平 glyph 列表。下游所有环节（动画、camera、光栅化）随后都只读取这份列表，
//! 于是帧成为时间的纯函数——可并行化，且随代码长度呈线性而非二次增长。

use std::path::PathBuf;

use crate::config::{
    DEFAULT_FONT_SIZE_PT, DEFAULT_TAB_WIDTH, FRAME_HEIGHT_UNITS, GUTTER_GAP_CELLS,
    NOT_AVAILABLE_CHARACTERS, PADDING_CELLS, POINTS_PER_UNIT,
};
use crate::error::{Error, Result};
use crate::font::Font;
use crate::lexer::{self, Highlighted};
use crate::theme::{Theme, TokenKind};

/// 待渲染代码的来源。
#[derive(Debug, Clone)]
pub enum Source {
    /// 内联代码。
    Text(String),
    /// 渲染时才读取的文件（UTF-8）。
    File(PathBuf),
}

impl Source {
    /// 用于内联代码的便捷构造器。
    pub fn text(code: impl Into<String>) -> Self {
        Source::Text(code.into())
    }

    /// 用于文件的便捷构造器。
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Source::File(path.into())
    }

    fn read(&self) -> Result<String> {
        match self {
            Source::Text(s) => Ok(s.clone()),
            Source::File(path) => {
                let bytes = std::fs::read(path).map_err(|source| Error::CodeFile {
                    path: path.clone(),
                    source,
                })?;
                String::from_utf8(bytes).map_err(|e| Error::Decode {
                    path: path.clone(),
                    source: e.utf8_error(),
                })
            }
        }
    }
}

/// 预处理之后、排版之前的代码。
#[derive(Debug, Clone)]
pub struct Preprocessed {
    /// 展开制表符后的文本，与用户所写完全一致（去掉首尾空行）。高亮器看到的
    /// 是*这份*文本，绝不是重写后的版本。
    pub text: String,
    /// 预处理后的各行（`text.split('\n')`）。
    pub lines: Vec<String>,
    /// 视觉上为空白行的索引。
    pub empty_lines: Vec<usize>,
    /// 每个内部空格的 `(line, column)`（保留为空白单元格，而非 glyph）。
    pub inner_spaces: Vec<(usize, usize)>,
    /// 实际会被键入的字符数（不含缩进、行尾空格与空行）。
    pub typed_chars: usize,
}

/// glyph 所代表的内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphRole {
    /// 代码本身的字符。
    Code,
    /// 一个行号数字；其颜色取决于哪一行处于激活状态。
    LineNumber { line: u32 },
}

/// 一个已排版的 glyph，使用世界坐标（相对 baseline，以参考分辨率的像素计）。
#[derive(Debug, Clone)]
pub struct Glyph {
    /// 字体内部的 glyph 索引。
    pub glyph_id: u16,
    /// glyph 原点的左边缘，世界像素。
    pub x: f32,
    /// glyph 的 baseline，世界像素（Y 向下增大）。
    pub baseline_y: f32,
    /// 预先解析好的颜色。
    pub color: (u8, u8, u8),
    /// 该 glyph 所属的行。
    pub line: u32,
    /// 行内的列。
    pub col: u32,
    /// 该 glyph 是什么。
    pub role: GlyphRole,
    /// 不得绘制任何字符（内部空格保留其单元格，但仍为空白）。
    pub invisible: bool,
    /// 键入顺序中的索引；对从不“键入”的 glyph（缩进、行尾空格、行号）为 `None`。
    pub typed_index: Option<usize>,
}

/// 一行已排版的代码。
#[derive(Debug, Clone, Copy)]
pub struct LineBox {
    /// 行索引。
    pub index: usize,
    /// 该行的 baseline Y。
    pub baseline_y: f32,
    /// 被键入的第一列（跳过缩进）。
    pub first_typed_col: usize,
    /// 被键入的最后一列（不含；跳过行尾空格）。
    pub end_col: usize,
    /// 当该行没有可见内容时为 `true`。
    pub is_empty: bool,
    /// 键入顺序中第一个被键入 glyph 的索引。
    pub first_typed_index: Option<usize>,
    /// 键入顺序中最后一个被键入 glyph 之后的位置。
    pub end_typed_index: usize,
}

/// 代码块的完整、不可变布局。
#[derive(Debug, Clone)]
pub struct Layout {
    /// 所有 glyph（代码 + 行号），按绘制顺序。
    pub glyphs: Vec<Glyph>,
    /// 每行的几何信息。
    pub lines: Vec<LineBox>,
    /// 一个字符单元格的 advance 宽度，世界像素。
    pub cell_w: f32,
    /// 相邻 baseline 之间的垂直距离，世界像素。
    pub cell_h: f32,
    /// 排版所用的字体大小，世界像素。
    pub font_size_px: f32,
    /// 当前行高亮条的左边缘，世界像素。
    pub bar_left: f32,
    /// 当前行高亮条的右边缘，世界像素。
    pub bar_right: f32,
    /// 代码本体起始处的 X，世界像素。
    pub code_left: f32,
    /// 每个键入字符之后的光标 X（世界像素）；按键入顺序索引。
    pub cursor_x: Vec<f32>,
    /// 尚未键入任何内容时光标所在的位置。
    pub cursor_start_x: f32,
    /// 已键入字符的数量。
    pub typed_chars: usize,
    /// 代码的 glyph 总数（不含行号）。
    pub code_glyphs: usize,
}

/// 展开制表符、去掉首尾空行、标记空行与内部空格。与原始实现的预处理保持一致，
/// 使两个渲染器在“键入什么”上取得一致。
///
/// 交给下游的文本是用户代码，未经修改。内部空格通过 [`Preprocessed::inner_spaces`]
/// 保留其单元格，而不是用占位字符代替：曾经的做法意味着*高亮器*扫描的是重写后的
/// 源码，结果 `let x = foo(1, 2);` 里的 `x` 真的会被当作函数名读取。
pub fn preprocess(source: &Source) -> Result<Preprocessed> {
    let raw = source.read()?;

    if let Some(bad) = raw.chars().find(|c| NOT_AVAILABLE_CHARACTERS.contains(c)) {
        return Err(Error::InvalidCharacters(vec![bad]));
    }

    let expanded = expand_tabs(&raw, DEFAULT_TAB_WIDTH);
    let lines = strip_blank_edges(&expanded);
    if lines.is_empty() {
        return Err(Error::EmptyCode);
    }

    let mut empty_lines = Vec::new();
    let mut inner_spaces = Vec::new();
    let mut typed_chars = 0usize;

    for (line_index, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let first = chars.iter().position(|c| *c != ' ' && *c != '\t');
        let last = chars.iter().rposition(|c| *c != ' ' && *c != '\t');

        let (Some(first), Some(last)) = (first, last) else {
            // 空行：保留该行以便行号对齐，但它对键入时间轴没有任何贡献。
            empty_lines.push(line_index);
            continue;
        };

        for col in first..=last {
            if chars[col] == ' ' {
                inner_spaces.push((line_index, col));
            }
        }
        typed_chars += last - first + 1;
    }

    Ok(Preprocessed {
        text: lines.join("\n"),
        lines,
        empty_lines,
        inner_spaces,
        typed_chars,
    })
}

/// 对预处理后的代码进行排版。
///
/// 世界坐标**固定在参考分辨率**（1080p）下，因此无论输出尺寸如何，代码块在帧中
/// 占据的比例都相同；输出分辨率只改变最终光栅的像素密度。这沿用了 Manim 的世界
/// 单位，也是 camera 的视野（`REFERENCE_WIDTH * scale`）在任何分辨率下都有意义的
/// 原因。
pub fn layout(
    pre: &Preprocessed,
    font: &Font,
    theme: &Theme,
    language: &str,
    line_spacing: f32,
) -> Result<Layout> {
    let highlighted: Highlighted = lexer::highlight(&pre.text, language);

    // 字体大小：Manim 的世界单位，换算为参考像素。
    let unit_px = crate::camera::REFERENCE_HEIGHT / FRAME_HEIGHT_UNITS;
    let font_size_px = (DEFAULT_FONT_SIZE_PT / POINTS_PER_UNIT) * unit_px;

    // 等宽单元格度量。
    //
    // `advance_em()` 已按字体的设计单位归一化，因此要乘以*像素字体大小*来缩放。
    // `glyph_scale` 是另一种换算——它把轮廓坐标（字体单位）映射为像素——此处绝不能
    // 使用它，否则所有 glyph 都会坍缩到同一个 X 上。
    let cell_w = advance_of(font, 'M') * font_size_px;
    let cell_h = font_size_px * (1.0 + line_spacing);
    let ascender = font.ascender_em() * font_size_px;

    // 行号槽：右对齐的行号。
    let digits = pre.lines.len().max(1).to_string().len();
    let gutter_w = digits as f32 * cell_w;
    let pad = PADDING_CELLS * cell_w;
    let gap = GUTTER_GAP_CELLS * cell_w;
    let code_left = pad + gutter_w + gap;
    let bar_left = pad * 0.25;
    let code_right = max_line_width(&pre.lines, font, font_size_px) + code_left;
    let bar_right = code_right + cell_w * 0.5;

    let top_pad = cell_h * 0.5;
    let mut glyphs: Vec<Glyph> = Vec::new();
    let mut lines: Vec<LineBox> = Vec::with_capacity(pre.lines.len());
    let mut cursor_x: Vec<f32> = Vec::with_capacity(pre.typed_chars);
    let mut typed_index = 0usize;
    // 解析颜色要沿 token 的父链走一遍，并扫描 theme 的稀疏表，这对 token 的每个
    // 字符重复做太昂贵。相邻字符几乎总是属于同一种类，因此一条缓存项就能覆盖
    // 其中的大多数。
    let mut last_color: Option<(TokenKind, (u8, u8, u8))> = None;
    // 指向 `pre.text`（`lines.join("\n")`）的字符索引。lexer 为该字符串的每个字符
    // 生成了一个 token 种类，因此这个索引与之同步推进——若对每个 glyph 重新计算，
    // 排版将随代码长度呈二次增长。
    let mut text_index = 0usize;

    for (line_index, line) in pre.lines.iter().enumerate() {
        let baseline_y = top_pad + ascender + line_index as f32 * cell_h;
        let chars: Vec<char> = line.chars().collect();
        let first = chars.iter().position(|c| *c != ' ').unwrap_or(chars.len());
        let last_exclusive = chars
            .iter()
            .rposition(|c| *c != ' ')
            .map(|i| i + 1)
            .unwrap_or(first);
        let is_empty = first >= last_exclusive;

        let mut x = code_left;
        let mut line_typed_start = None;
        for (col, ch) in chars.iter().enumerate() {
            let typed = !is_empty && col >= first && col < last_exclusive;
            // 行内的空格保留其单元格但不绘制任何内容。这与 `preprocess` 记录
            // `inner_spaces` 时所用的判定相同，因此二者无需查找表即可保持一致。
            let is_inner_space = typed && *ch == ' ';

            let kind = highlighted.kind_at(text_index + col);
            let color = match last_color {
                Some((cached, color)) if cached == kind => color,
                _ => {
                    let color = theme.color(kind);
                    last_color = Some((kind, color));
                    color
                }
            };

            // 每个字符做一次 `cmap` 查找，同时用于轮廓与 advance——这是本循环中
            // 最昂贵的调用。
            let glyph = font.glyph_index(*ch);

            if typed {
                if is_inner_space {
                    glyphs.push(Glyph {
                        glyph_id: 0,
                        x,
                        baseline_y,
                        color,
                        line: line_index as u32,
                        col: col as u32,
                        role: GlyphRole::Code,
                        invisible: true,
                        typed_index: Some(typed_index),
                    });
                } else if let Some(glyph_id) = glyph {
                    glyphs.push(Glyph {
                        glyph_id: glyph_id.0,
                        x,
                        baseline_y,
                        color,
                        line: line_index as u32,
                        col: col as u32,
                        role: GlyphRole::Code,
                        invisible: false,
                        typed_index: Some(typed_index),
                    });
                } else {
                    return Err(Error::MissingGlyph {
                        path: font.origin().to_string(),
                        codepoint: *ch as u32,
                        ch: *ch,
                    });
                }
                if line_typed_start.is_none() {
                    line_typed_start = Some(typed_index);
                }
                typed_index += 1;
            }

            let advance = if *ch == '\t' {
                cell_w
            } else {
                font.advance_em(glyph.unwrap_or(ttf_parser::GlyphId(0))) * font_size_px
            };
            x += advance;
            if typed {
                cursor_x.push(x);
            }
        }

        let first_typed_col = if is_empty { 0 } else { first };
        lines.push(LineBox {
            index: line_index,
            baseline_y,
            first_typed_col,
            end_col: last_exclusive,
            is_empty,
            first_typed_index: line_typed_start,
            end_typed_index: typed_index,
        });
        // 越过本行以及 `join("\n")` 插入的换行符。
        text_index += chars.len() + 1;
    }

    // 行号，在行号槽内右对齐。
    for (line_index, line_box) in lines.iter().enumerate() {
        let text = (line_index + 1).to_string();
        let width = text
            .chars()
            .map(|c| advance_of(font, c) * font_size_px)
            .sum::<f32>();
        let mut x = pad + gutter_w - width;
        for ch in text.chars() {
            if let Some(glyph_id) = font.glyph_index(ch) {
                glyphs.push(Glyph {
                    glyph_id: glyph_id.0,
                    x,
                    baseline_y: line_box.baseline_y,
                    color: (0, 0, 0), // 绘制时再解析
                    line: line_index as u32,
                    col: 0,
                    role: GlyphRole::LineNumber {
                        line: line_index as u32,
                    },
                    invisible: false,
                    typed_index: None,
                });
            }
            x += advance_of(font, ch) * font_size_px;
        }
    }

    let code_glyphs = glyphs.iter().filter(|g| g.role == GlyphRole::Code).count();
    Ok(Layout {
        glyphs,
        lines,
        cell_w,
        cell_h,
        font_size_px,
        bar_left,
        bar_right,
        code_left,
        cursor_start_x: code_left,
        cursor_x,
        typed_chars: typed_index,
        code_glyphs,
    })
}

/// 一个字符的 advance 宽度，以 **em 单位**计（参见 `layout` 中的注释：
/// 调用方需再乘以像素字体大小）。
fn advance_of(font: &Font, ch: char) -> f32 {
    match font.glyph_index(ch) {
        Some(id) => font.advance_em(id),
        None => font.advance_em(ttf_parser::GlyphId(0)),
    }
}

fn max_line_width(lines: &[String], font: &Font, font_size_px: f32) -> f32 {
    let mut best = 0.0f32;
    for line in lines {
        let w: f32 = line
            .chars()
            .map(|c| advance_of(font, c) * font_size_px)
            .sum();
        if w > best {
            best = w;
        }
    }
    best.max(1.0)
}

fn expand_tabs(text: &str, tab_width: usize) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split('\n') {
        let mut col = 0usize;
        for ch in line.chars() {
            if ch == '\t' {
                let spaces = tab_width - (col % tab_width);
                for _ in 0..spaces {
                    out.push(' ');
                }
                col += spaces;
            } else {
                out.push(ch);
                col += 1;
            }
        }
        out.push('\n');
    }
    out.pop();
    out
}

fn strip_blank_edges(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines
}
