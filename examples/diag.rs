//! 字体诊断：`cargo run --example diag`。
//!
//! 打印布局所依赖的度量值——如果 `advance_em` 返回 0，所有字形都会落在同一个
//! X 坐标上，画面就会坍缩成一列。

use codevideorenderer::font::Font;
use codevideorenderer::layout::{Source, preprocess};

fn main() {
    let font = Font::find(None).expect("a usable font");
    println!("origin         : {}", font.origin());
    println!("units_per_em   : {}", font.units_per_em());
    println!("ascender_em    : {:.4}", font.ascender_em());
    println!("descender_em   : {:.4}", font.descender_em());
    println!("line_height_em : {:.4}", font.line_height_em());
    println!();

    for ch in ['M', 'a', ' ', '0', '_', '中', '演', '🚀'] {
        match font.glyph_index(ch) {
            Some(id) => {
                let outline = font.outline(id);
                let outline_desc = match outline {
                    Some(path) => {
                        let b = path.bounds();
                        format!(
                            "{} vertices, bounds x[{:.0},{:.0}] y[{:.0},{:.0}]",
                            path.len(),
                            b.left(),
                            b.right(),
                            b.top(),
                            b.bottom()
                        )
                    }
                    None => "no outline".to_string(),
                };
                println!(
                    "{ch:?} -> glyph {:>6}  advance_em = {:.4}  {outline_desc}",
                    id.0,
                    font.advance_em(id)
                );
            }
            None => println!("{ch:?} -> missing"),
        }
    }

    println!();
    let pre = preprocess(&Source::text("def fibonacci(n):\n    return n\n")).unwrap();
    println!("preprocessed lines   : {}", pre.lines.len());
    println!("typed chars          : {}", pre.typed_chars);
    println!("inner spaces         : {:?}", pre.inner_spaces);
}
