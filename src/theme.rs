//! Theme 查找、token 配色，以及手写的默认调色板。
//!
//! 43 套 Pygments 风格位于 [`crate::theme_data`]——这是一个生成文件，因此
//! `--style material` / `--style github-dark` 的行为与原始 Python 库完全一致。
//! [`THEME_MIDNIGHT`] 是唯一一套手写的主题，也是默认主题：编辑器调色板需要
//! 与工具栏、标签页和行号槽彼此协调，而代码视频的屏幕上除了代码本身别无他物。

pub use crate::theme_data::{THEMES, Theme, ThemeStyle, TokenKind};

/// 调用方未指定时使用的主题。
pub const DEFAULT_THEME_NAME: &str = "midnight";

/// 手写的默认主题。
///
/// 每处选择都经过斟酌：
///
/// * **标点与运算符贴近 foreground。** Material 与 One Dark 把每个括号、逗号
///   和冒号都涂成亮青色，让一行代码变成满屏彩纸屑。这里让它们退后，目光自然
///   落在关键字、字符串和调用上。
/// * **六种克制的色相，各司其职**——紫色给关键字，蓝色给函数名，绿色给字符串，
///   青色给内建项与类型，琥珀色给类与数字，一种柔和的红色留给错误。
/// * **没有任何颜色完全饱和，background 也很深**，因此较亮的颜色在视频压缩后
///   依然可读，而不是发光过曝。
/// * 行号、当前行横条与光标都由 `background`/`foreground` *推导*而来（见
///   [`Theme::line_highlight`]），而不是硬编码，这使它们彼此协调——在这套主题
///   如此，在其它所有主题（包括浅色主题）中也如此。
pub static THEME_MIDNIGHT: Theme = Theme {
    name: "midnight",
    background: (16, 20, 31),
    foreground: (230, 236, 247),
    styles: &[
        (TokenKind::Comment, (110, 122, 150)),
        (TokenKind::Error, (255, 138, 144)),
        (TokenKind::Escape, (150, 226, 232)),
        (TokenKind::Generic, (230, 236, 247)),
        (TokenKind::GenericDeleted, (255, 138, 144)),
        (TokenKind::GenericEmph, (156, 199, 255)),
        (TokenKind::GenericEmphStrong, (245, 212, 142)),
        (TokenKind::GenericError, (255, 138, 144)),
        (TokenKind::GenericHeading, (168, 230, 184)),
        (TokenKind::GenericInserted, (168, 230, 184)),
        (TokenKind::GenericOutput, (110, 122, 150)),
        (TokenKind::GenericPrompt, (245, 212, 142)),
        (TokenKind::GenericStrong, (255, 138, 144)),
        (TokenKind::GenericSubheading, (150, 226, 232)),
        (TokenKind::GenericTraceback, (255, 138, 144)),
        (TokenKind::Keyword, (196, 167, 240)),
        (TokenKind::KeywordConstant, (255, 201, 138)),
        (TokenKind::KeywordDeclaration, (196, 167, 240)),
        (TokenKind::KeywordNamespace, (196, 167, 240)),
        (TokenKind::KeywordPseudo, (196, 167, 240)),
        (TokenKind::KeywordType, (150, 226, 232)),
        (TokenKind::Literal, (168, 230, 184)),
        (TokenKind::LiteralDate, (168, 230, 184)),
        (TokenKind::LiteralNumber, (255, 201, 138)),
        (TokenKind::LiteralString, (168, 230, 184)),
        (TokenKind::LiteralStringAffix, (196, 167, 240)),
        (TokenKind::LiteralStringBacktick, (168, 230, 184)),
        (TokenKind::LiteralStringChar, (168, 230, 184)),
        (TokenKind::LiteralStringDelimiter, (152, 163, 188)),
        (TokenKind::LiteralStringDoc, (110, 122, 150)),
        (TokenKind::LiteralStringDouble, (168, 230, 184)),
        (TokenKind::LiteralStringEscape, (150, 226, 232)),
        (TokenKind::LiteralStringHeredoc, (168, 230, 184)),
        (TokenKind::LiteralStringInterpol, (150, 226, 232)),
        (TokenKind::LiteralStringOther, (168, 230, 184)),
        (TokenKind::LiteralStringRegex, (150, 226, 232)),
        (TokenKind::LiteralStringSingle, (168, 230, 184)),
        (TokenKind::LiteralStringSymbol, (150, 226, 232)),
        (TokenKind::Name, (230, 236, 247)),
        (TokenKind::NameAttribute, (196, 167, 240)),
        (TokenKind::NameBuiltin, (150, 226, 232)),
        (TokenKind::NameBuiltinPseudo, (150, 226, 232)),
        (TokenKind::NameClass, (245, 212, 142)),
        (TokenKind::NameConstant, (255, 201, 138)),
        (TokenKind::NameDecorator, (156, 199, 255)),
        (TokenKind::NameEntity, (150, 226, 232)),
        (TokenKind::NameException, (245, 212, 142)),
        (TokenKind::NameFunction, (156, 199, 255)),
        (TokenKind::NameFunctionMagic, (156, 199, 255)),
        (TokenKind::NameLabel, (156, 199, 255)),
        (TokenKind::NameNamespace, (245, 212, 142)),
        (TokenKind::NameOther, (230, 236, 247)),
        (TokenKind::NameProperty, (245, 212, 142)),
        (TokenKind::NameTag, (255, 138, 144)),
        (TokenKind::NameVariable, (230, 236, 247)),
        (TokenKind::NameVariableClass, (150, 226, 232)),
        (TokenKind::NameVariableGlobal, (150, 226, 232)),
        (TokenKind::NameVariableInstance, (150, 226, 232)),
        (TokenKind::NameVariableMagic, (156, 199, 255)),
        (TokenKind::Operator, (165, 176, 200)),
        (TokenKind::OperatorWord, (196, 167, 240)),
        (TokenKind::Punctuation, (152, 163, 188)),
        (TokenKind::Text, (230, 236, 247)),
    ],
};

impl Theme {
    /// 解析 `kind` 对应的颜色。
    ///
    /// 复刻 Pygments 的继承机制：没有显式条目的 token 会回退到其父 token，
    /// 最终回退到主题的 foreground 颜色。
    pub fn color(&self, kind: TokenKind) -> (u8, u8, u8) {
        let mut current = Some(kind);
        while let Some(k) = current {
            if let Some(&(_, color)) = self.styles.iter().find(|(t, _)| *t == k) {
                return color;
            }
            current = k.parent();
        }
        self.foreground
    }

    /// 从 background 向 `other` 方向混合；`t == 0` 时保持 background 不变。
    fn mix(&self, other: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
        let channel = |from: u8, to: u8| {
            (from as f32 + (to as f32 - from as f32) * t)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        (
            channel(self.background.0, other.0),
            channel(self.background.1, other.1),
            channel(self.background.2, other.2),
        )
    }

    /// 正在输入行背后的横条。
    ///
    /// 它朝 *foreground 方向*移动，因此深色主题得到一条略微提亮的横条，
    /// 浅色主题得到一条略微压暗的横条——同一段代码，按主题各自调校，
    /// 而不是硬编码一个只在深色下才合适的 `#333333`。
    pub fn line_highlight(&self) -> (u8, u8, u8) {
        self.mix(self.foreground, 0.08)
    }

    /// 非活动行号：可读，但明显退居代码之后。
    pub fn line_number(&self) -> (u8, u8, u8) {
        self.mix(self.foreground, 0.42)
    }

    /// 正在输入行所在的行号。
    pub fn line_number_active(&self) -> (u8, u8, u8) {
        self.mix(self.foreground, 0.92)
    }

    /// 光标：完整 foreground，因为它是唯一始终在移动的元素。
    pub fn cursor(&self) -> (u8, u8, u8) {
        self.foreground
    }
}

/// 按名称查找主题。
///
/// 匹配不区分大小写，并将 `_` 与 `-` 视为等价，因此 `github-dark` 和
/// `GitHub_Dark` 都能命中。手写主题会优先检查，因为它不在生成的
/// Pygments 表中。
pub fn theme_by_name(name: &str) -> Option<&'static Theme> {
    let normalized = name.trim().to_ascii_lowercase().replace('_', "-");
    if normalized == THEME_MIDNIGHT.name {
        return Some(&THEME_MIDNIGHT);
    }
    if let Some(found) = THEMES.iter().copied().find(|t| t.name == normalized) {
        return Some(found);
    }
    // 一些本身并非 Pygments 风格名称的常见拼写。
    let alias = match normalized.as_str() {
        "onedark" | "one-dark-pro" => "one-dark",
        "solarizeddark" => "solarized-dark",
        "solarizedlight" => "solarized-light",
        "gruvboxdark" => "gruvbox-dark",
        "gruvboxlight" => "gruvbox-light",
        "paraisodark" => "paraiso-dark",
        "paraisolight" => "paraiso-light",
        "stata" | "statadark" => "stata-dark",
        "light" => "friendly",
        "dark" => "native",
        _ => return None,
    };
    THEMES.iter().copied().find(|t| t.name == alias)
}

/// 所有内置主题名称，按字母序排列。
pub fn theme_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = THEMES.iter().map(|t| t.name).collect();
    names.push(THEME_MIDNIGHT.name);
    names.sort_unstable();
    names
}

/// 未指定时使用的主题。
pub fn default_theme() -> &'static Theme {
    theme_by_name(DEFAULT_THEME_NAME).unwrap_or(&THEME_MIDNIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luma(color: (u8, u8, u8)) -> u32 {
        color.0 as u32 + color.1 as u32 + color.2 as u32
    }

    #[test]
    fn the_hand_written_theme_is_the_default_and_is_findable() {
        assert_eq!(default_theme().name, DEFAULT_THEME_NAME);
        assert!(theme_names().contains(&"midnight"));
        assert_eq!(
            theme_by_name("Midnight").expect("case-insensitive").name,
            "midnight"
        );
    }

    /// UI 颜色必须跟随主题，这正是浅色 Pygments 风格能用的原因：固定的
    /// `#333333` 横条在浅色背景上会看不见。
    #[test]
    fn ui_colours_follow_the_theme_background() {
        let dark = &THEME_MIDNIGHT;
        assert!(
            luma(dark.line_highlight()) > luma(dark.background),
            "on a dark theme the current-line bar lifts towards the foreground"
        );
        assert!(
            luma(dark.line_highlight()) - luma(dark.background) < 60,
            "the bar must stay a whisper, not a stripe"
        );
        assert!(luma(dark.line_number()) > luma(dark.background));
        assert!(luma(dark.line_number()) < luma(dark.foreground));
        assert_eq!(dark.cursor(), dark.foreground);

        let light = theme_by_name("friendly").expect("a light Pygments style");
        assert!(
            luma(light.line_highlight()) < luma(light.background),
            "on a light theme the same maths darkens the bar"
        );
        assert!(luma(light.line_number()) < luma(light.background));
    }

    /// 这套调色板的核心要义：标点绝不能喧宾夺主。
    #[test]
    fn punctuation_recedes_behind_keywords_and_strings() {
        let theme = &THEME_MIDNIGHT;
        let contrast = |kind: TokenKind| {
            (luma(theme.color(kind)) as i32 - luma(theme.background) as i32).unsigned_abs()
        };
        assert!(
            contrast(TokenKind::Punctuation) < contrast(TokenKind::Keyword),
            "punctuation must sit closer to the background than keywords do"
        );
        assert!(contrast(TokenKind::Operator) < contrast(TokenKind::LiteralString));
        assert!(contrast(TokenKind::Comment) < contrast(TokenKind::Name));
    }
}
