//! 语法高亮。
//!
//! 原库把高亮委托给 Pygments：由一张庞大的语言名 `Literal` 列表驱动数百个
//! lexer。本 crate 改为内置一个手写的小型 scanner。它能识别代码演示视频中
//! 出现的那些语法，并输出 **与 Pygments 兼容的 token 种类**，因此导出的
//! Pygments 主题（见 [`crate::theme_data`]）可原样套用。
//!
//! 输出是*每个输入字符对应一个 token 种类*——这正是 renderer 所需的粒度，
//! 因为字形是逐字符揭示的。

// 有若干扫描需要按索引向前看（字符串前缀、JSON 键、“下一个非空白字符是否是
// 左括号”）。在这些地方，按索引的写法比等价的迭代器写法更清晰。
#![allow(clippy::explicit_counter_loop)]

use crate::theme::TokenKind;

/// 一段源代码逐字符的 token 种类。
#[derive(Debug, Clone)]
pub struct Highlighted {
    kinds: Vec<TokenKind>,
}

impl Highlighted {
    /// 覆盖的字符数量。
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    /// 没有任何内容需要高亮时为 `true`。
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// 指定字符索引处的 token 种类（缺省为 `Text`）。
    pub fn kind_at(&self, index: usize) -> TokenKind {
        self.kinds.get(index).copied().unwrap_or(TokenKind::Text)
    }

    /// 所有 token 种类。
    pub fn kinds(&self) -> &[TokenKind] {
        &self.kinds
    }
}

/// 使用 Pygments 语言名（例如 `"python"`）对 `code` 进行高亮。
///
/// 未知语言会退回到通用的 C 系 scanner，而不是报错，
/// 这与原实现对冷门 lexer 名的处理方式保持一致。
pub fn highlight(code: &str, language: &str) -> Highlighted {
    let spec = LangSpec::for_language(language);
    let mut scanner = Scanner::new(code, spec);
    scanner.run();
    Highlighted {
        kinds: scanner.kinds,
    }
}

/// 拥有手写规则的语言。其余全部使用通用 scanner。
pub fn supported_languages() -> &'static [&'static str] {
    &[
        "python",
        "javascript",
        "typescript",
        "rust",
        "go",
        "c",
        "cpp",
        "csharp",
        "java",
        "kotlin",
        "swift",
        "php",
        "ruby",
        "bash",
        "json",
        "yaml",
        "toml",
        "markdown",
        "sql",
    ]
}

/// 当 `language` 命中某一种手写语法时为 `true`。
///
/// 别名（`py`、`rs`、`js`、…）也算受支持。返回 `false` 表示 [`highlight`]
/// 会静默改用通用回退 scanner，CLI 会将其作为提示上报。
pub fn is_supported(language: &str) -> bool {
    LangSpec::lookup(language).is_some()
}

// ---------------------------------------------------------------------------
// 语言表
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flavour {
    /// 基于缩进、`#` 注释、三引号字符串、`def`/`class`。
    Python,
    /// 花括号与分号；注释标记取自 spec。
    Curly,
    /// Shell 风格：`#` 注释、`$variables`。
    Shell,
    /// JSON：字符串后跟 `:` 时即为键。
    Json,
    /// YAML/TOML/INI：`key: value` / `key = value`。
    Config,
    /// Markdown。
    Markdown,
    /// 其余情况：仅处理字符串、数字和行注释。
    Plain,
}

#[derive(Clone, Copy)]
struct LangSpec {
    flavour: Flavour,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    constants: &'static [&'static str],
    builtins: &'static [&'static str],
    line_comment: &'static str,
    block_comment: Option<(&'static str, &'static str)>,
    hash_comment: bool,
    fn_keywords: &'static [&'static str],
    class_keywords: &'static [&'static str],
    /// 将 `name(` 视为函数名。
    call_is_function: bool,
}

impl LangSpec {
    fn plain(line_comment: &'static str) -> Self {
        Self {
            flavour: Flavour::Plain,
            keywords: &[],
            types: &[],
            constants: &[],
            builtins: &[],
            line_comment,
            block_comment: None,
            hash_comment: line_comment == "#",
            fn_keywords: &[],
            class_keywords: &[],
            call_is_function: false,
        }
    }

    fn curly(line_comment: &'static str) -> Self {
        Self {
            flavour: Flavour::Curly,
            block_comment: Some(("/*", "*/")),
            call_is_function: true,
            ..Self::plain(line_comment)
        }
    }

    /// `language` 对应的语法，或通用回退 scanner。
    fn for_language(language: &str) -> Self {
        Self::lookup(language).unwrap_or_else(|| Self::plain("//"))
    }

    /// 当 `language` 没有手写语法时返回 `None`，即 [`highlight`]
    /// 会静默使用通用回退 scanner 的情况。
    fn lookup(language: &str) -> Option<Self> {
        let lang = language.trim().to_ascii_lowercase();
        let head = lang
            .split(['+', '-', '_', '.', ' ', '#'])
            .next()
            .unwrap_or("")
            .to_string();
        let csharp = lang.contains("c#") || lang == "csharp" || lang == "cs";
        if csharp {
            return Some(LangSpec {
                keywords: CSHARP_KEYWORDS,
                types: CSHARP_TYPES,
                constants: &["true", "false", "null"],
                class_keywords: &["class", "interface", "struct", "enum", "record"],
                ..LangSpec::curly("//")
            });
        }
        Some(match head.as_str() {
            "python" | "py" => LangSpec {
                flavour: Flavour::Python,
                keywords: PY_KEYWORDS,
                constants: PY_CONSTANTS,
                builtins: PY_BUILTINS,
                fn_keywords: &["def", "async"],
                class_keywords: &["class"],
                ..LangSpec::plain("#")
            },
            "rust" | "rs" => LangSpec {
                keywords: RUST_KEYWORDS,
                types: RUST_TYPES,
                constants: RUST_CONSTANTS,
                fn_keywords: &["fn"],
                class_keywords: &["struct", "enum", "trait", "impl"],
                ..LangSpec::curly("//")
            },
            "go" => LangSpec {
                keywords: GO_KEYWORDS,
                types: GO_TYPES,
                constants: &["true", "false", "nil", "iota"],
                builtins: GO_BUILTINS,
                fn_keywords: &["func"],
                class_keywords: &["struct", "interface"],
                ..LangSpec::curly("//")
            },
            "java" => LangSpec {
                keywords: JAVA_KEYWORDS,
                types: JAVA_TYPES,
                constants: &["true", "false", "null"],
                class_keywords: &["class", "interface", "enum", "record"],
                ..LangSpec::curly("//")
            },
            "kotlin" => LangSpec {
                keywords: KOTLIN_KEYWORDS,
                constants: &["true", "false", "null"],
                fn_keywords: &["fun"],
                class_keywords: &["class", "interface", "object"],
                ..LangSpec::curly("//")
            },
            "swift" => LangSpec {
                keywords: SWIFT_KEYWORDS,
                constants: &["true", "false", "nil", "self"],
                fn_keywords: &["func"],
                class_keywords: &["class", "struct", "enum", "protocol", "extension"],
                ..LangSpec::curly("//")
            },
            "javascript" | "js" | "jsx" | "mjs" | "cjs" | "typescript" | "ts" | "tsx" => LangSpec {
                keywords: JS_KEYWORDS,
                types: JS_TYPES,
                constants: &["true", "false", "null", "undefined", "NaN", "Infinity"],
                builtins: JS_BUILTINS,
                fn_keywords: &["function"],
                class_keywords: &["class", "interface", "enum"],
                ..LangSpec::curly("//")
            },
            "c" | "cpp" | "cxx" | "cc" | "h" | "hpp" | "objc" | "objectivec" => LangSpec {
                keywords: C_KEYWORDS,
                types: C_TYPES,
                constants: &["NULL", "true", "false", "nullptr"],
                builtins: C_BUILTINS,
                class_keywords: &["struct", "class", "enum", "union", "typedef"],
                ..LangSpec::curly("//")
            },
            "php" => LangSpec {
                keywords: PHP_KEYWORDS,
                constants: &["true", "false", "null"],
                builtins: PHP_BUILTINS,
                fn_keywords: &["function"],
                class_keywords: &["class", "interface", "trait"],
                ..LangSpec::curly("//")
            },
            "ruby" | "rb" => LangSpec {
                keywords: RUBY_KEYWORDS,
                constants: &["true", "false", "nil", "self"],
                fn_keywords: &["def"],
                class_keywords: &["class", "module"],
                ..LangSpec::plain("#")
            },
            "bash" | "sh" | "shell" | "zsh" | "ksh" | "console" => LangSpec {
                flavour: Flavour::Shell,
                keywords: SHELL_KEYWORDS,
                builtins: SHELL_BUILTINS,
                ..LangSpec::plain("#")
            },
            "json" | "json5" | "jsonld" => LangSpec {
                flavour: Flavour::Json,
                constants: &["true", "false", "null"],
                ..LangSpec::plain("//")
            },
            "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "properties" | "docker"
            | "dockerfile" | "makefile" | "make" | "cmake" => LangSpec {
                flavour: Flavour::Config,
                constants: &["true", "false", "null", "yes", "no", "on", "off"],
                ..LangSpec::plain("#")
            },
            "markdown" | "md" | "rst" | "text" | "plaintext" | "tex" => LangSpec {
                flavour: Flavour::Markdown,
                ..LangSpec::plain("#")
            },
            "sql" | "mysql" | "postgresql" | "plpgsql" | "sqlite3" => LangSpec {
                keywords: SQL_KEYWORDS,
                types: SQL_TYPES,
                constants: &["NULL", "TRUE", "FALSE", "null", "true", "false"],
                builtins: SQL_BUILTINS,
                block_comment: Some(("/*", "*/")),
                ..LangSpec::plain("--")
            },
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// scanner
// ---------------------------------------------------------------------------

struct Scanner {
    chars: Vec<char>,
    kinds: Vec<TokenKind>,
    i: usize,
    spec: LangSpec,
    expect_fn_name: bool,
    expect_class_name: bool,
}

impl Scanner {
    fn new(code: &str, spec: LangSpec) -> Self {
        let chars: Vec<char> = code.chars().collect();
        let kinds = Vec::with_capacity(chars.len());
        Self {
            chars,
            kinds,
            i: 0,
            spec,
            expect_fn_name: false,
            expect_class_name: false,
        }
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    fn cur(&self) -> char {
        self.chars[self.i]
    }

    fn at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.i + offset).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        let mut j = self.i;
        for c in s.chars() {
            if j >= self.len() || self.chars[j] != c {
                return false;
            }
            j += 1;
        }
        true
    }

    fn push_span(&mut self, kind: TokenKind, count: usize) {
        for _ in 0..count {
            if self.i >= self.len() {
                break;
            }
            self.kinds.push(kind);
            self.i += 1;
        }
    }

    fn push_one(&mut self, kind: TokenKind) {
        self.push_span(kind, 1);
    }

    fn eat_while(&mut self, kind: TokenKind, pred: impl Fn(char) -> bool) {
        while self.i < self.len() && pred(self.cur()) {
            self.push_one(kind);
        }
    }

    fn at_line_start(&self) -> bool {
        let mut j = self.i;
        while j > 0 {
            let c = self.chars[j - 1];
            if c == '\n' {
                return true;
            }
            if !c.is_whitespace() {
                return false;
            }
            j -= 1;
        }
        true
    }

    fn run(&mut self) {
        while self.i < self.len() {
            // 空白与换行集中处理，让每种 flavour 都能正确处理，
            // 同时保持各 flavour scanner 简洁。
            let c = self.cur();
            if c == ' ' || c == '\t' {
                self.eat_while(TokenKind::Text, |c| c == ' ' || c == '\t');
                continue;
            }
            if c == '\n' {
                self.expect_fn_name = false;
                self.expect_class_name = false;
                self.push_one(TokenKind::Text);
                continue;
            }
            let consumed_before = self.i;
            match self.spec.flavour {
                Flavour::Python => self.scan_python(),
                Flavour::Curly => self.scan_curly(),
                Flavour::Shell => self.scan_shell(),
                Flavour::Json => self.scan_json(),
                Flavour::Config => self.scan_config(),
                Flavour::Markdown => self.scan_markdown(),
                Flavour::Plain => self.scan_plain(),
            }
            if self.i == consumed_before {
                // 兜底：每个 scanner 至少要消费一个字符，
                // 否则遇到意外输入时循环会永不停歇。
                self.push_one(TokenKind::Text);
            }
        }
        debug_assert_eq!(self.kinds.len(), self.chars.len());
        while self.kinds.len() < self.chars.len() {
            self.kinds.push(TokenKind::Text);
        }
    }

    // -- 共享辅助函数 ----------------------------------------------------

    fn eat_line_comment(&mut self, marker_len: usize) {
        self.push_span(TokenKind::CommentSingle, marker_len);
        while self.i < self.len() && self.cur() != '\n' {
            self.push_one(TokenKind::CommentSingle);
        }
    }

    fn eat_block_comment(&mut self) {
        let Some((open, close)) = self.spec.block_comment else {
            self.push_one(TokenKind::Operator);
            return;
        };
        self.push_span(TokenKind::CommentMultiline, open.chars().count());
        while self.i < self.len() {
            if self.starts_with(close) {
                self.push_span(TokenKind::CommentMultiline, close.chars().count());
                return;
            }
            self.push_one(TokenKind::CommentMultiline);
        }
    }

    fn eat_number(&mut self) {
        let kind = TokenKind::LiteralNumber;
        // 前导符号属于字面量（JSON 就是这样写负数的）。
        if self.cur() == '-' {
            self.push_one(kind);
        }
        if self.cur() == '0' && matches!(self.at(1), Some('x' | 'X' | 'b' | 'B' | 'o' | 'O')) {
            self.push_span(TokenKind::LiteralNumberHex, 2);
            self.eat_while(kind, |c| c.is_ascii_alphanumeric() || c == '_');
            return;
        }
        self.eat_while(kind, |c| c.is_ascii_digit() || c == '_');
        if self.i < self.len()
            && self.cur() == '.'
            && self.at(1).is_some_and(|c| c.is_ascii_digit())
        {
            self.push_one(kind);
            self.eat_while(kind, |c| c.is_ascii_digit() || c == '_');
        }
        if self.i < self.len() && matches!(self.cur(), 'e' | 'E') {
            self.push_one(kind);
            if self.i < self.len() && matches!(self.cur(), '+' | '-') {
                self.push_one(kind);
            }
            self.eat_while(kind, |c| c.is_ascii_digit());
        }
        self.eat_while(kind, |c| c.is_ascii_alphanumeric());
    }

    /// 消费一个带引号的字符串，反斜杠转义单独标记。
    fn eat_string(&mut self, quote: char, triple: bool, kind: TokenKind) {
        if triple {
            self.push_span(kind, 3);
        } else {
            self.push_one(kind);
        }
        while self.i < self.len() {
            let c = self.cur();
            if c == '\\' {
                self.push_one(TokenKind::LiteralStringEscape);
                if self.i < self.len() {
                    self.push_one(kind);
                }
                continue;
            }
            if triple {
                if self.cur() == quote && self.at(1) == Some(quote) && self.at(2) == Some(quote) {
                    self.push_span(kind, 3);
                    return;
                }
                self.push_one(kind);
            } else {
                if c == quote {
                    self.push_one(kind);
                    return;
                }
                if c == '\n' {
                    return; // 未终止：在行尾放弃
                }
                self.push_one(kind);
            }
        }
    }

    /// 消费带前缀的字符串，例如 `f"..."` 或 `b'''...'''`。
    fn try_prefixed_string(&mut self) -> bool {
        // 只向前看而不消费：至多两个前缀字母，然后是一个引号。
        let mut prefix = 0usize;
        for c in &self.chars[self.i..] {
            if prefix >= 2 || !matches!(c.to_ascii_lowercase(), 'r' | 'b' | 'u' | 'f') {
                break;
            }
            prefix += 1;
        }
        let quote_index = self.i + prefix;
        if prefix == 0 || quote_index >= self.len() {
            return false;
        }
        let quote = self.chars[quote_index];
        if quote != '"' && quote != '\'' {
            return false;
        }
        self.push_span(TokenKind::LiteralStringAffix, prefix);
        let triple = self.cur() == quote && self.at(1) == Some(quote) && self.at(2) == Some(quote);
        let kind = if triple {
            TokenKind::LiteralStringDoc
        } else if quote == '"' {
            TokenKind::LiteralStringDouble
        } else {
            TokenKind::LiteralStringSingle
        };
        self.eat_string(quote, triple, kind);
        true
    }

    fn classify_word(&mut self, word: &str) -> TokenKind {
        if self.expect_class_name {
            self.expect_class_name = false;
            return TokenKind::NameClass;
        }
        if self.expect_fn_name {
            self.expect_fn_name = false;
            return TokenKind::NameFunction;
        }
        if self.spec.keywords.contains(&word) {
            if self.spec.fn_keywords.contains(&word) {
                self.expect_fn_name = true;
            }
            if self.spec.class_keywords.contains(&word) {
                self.expect_class_name = true;
            }
            return TokenKind::Keyword;
        }
        if self.spec.types.contains(&word) {
            return TokenKind::KeywordType;
        }
        if self.spec.constants.contains(&word) {
            return TokenKind::KeywordConstant;
        }
        if self.spec.builtins.contains(&word) {
            return TokenKind::NameBuiltin;
        }
        if self.spec.flavour == Flavour::Python {
            if word == "self" || word == "cls" {
                return TokenKind::NameBuiltinPseudo;
            }
            if word.len() > 4 && word.starts_with("__") && word.ends_with("__") {
                return TokenKind::NameFunctionMagic;
            }
        }
        if self.spec.call_is_function && self.next_non_space_is('(') {
            return TokenKind::NameFunction;
        }
        TokenKind::Name
    }

    fn next_non_space_is(&self, want: char) -> bool {
        let mut j = self.i;
        while j < self.len() && self.chars[j] == ' ' {
            j += 1;
        }
        self.chars.get(j) == Some(&want)
    }

    fn eat_identifier_word(&mut self) {
        let start = self.i;
        while self.i < self.len() && is_ident_continue(self.cur()) {
            self.i += 1;
        }
        let word: String = self.chars[start..self.i].iter().collect();
        let kind = self.classify_word(&word);
        for _ in start..self.i {
            self.kinds.push(kind);
        }
    }

    fn eat_identifier_kind(&mut self, kind: TokenKind) {
        let start = self.i;
        while self.i < self.len() && is_ident_continue(self.cur()) {
            self.i += 1;
        }
        for _ in start..self.i {
            self.kinds.push(kind);
        }
    }

    fn eat_punct(&mut self) {
        let c = self.cur();
        if "()[]{};,.".contains(c) {
            self.push_one(TokenKind::Punctuation);
        } else {
            self.push_one(TokenKind::Operator);
        }
    }

    // -- 各 flavour scanner ---------------------------------------------

    fn scan_python(&mut self) {
        let c = self.cur();
        if c == '#' {
            self.eat_line_comment(1);
            return;
        }
        if (c == '"' || c == '\'') && self.at(1) == Some(c) && self.at(2) == Some(c) {
            self.eat_string(c, true, TokenKind::LiteralStringDoc);
            return;
        }
        if c == '"' || c == '\'' {
            let kind = if c == '"' {
                TokenKind::LiteralStringDouble
            } else {
                TokenKind::LiteralStringSingle
            };
            self.eat_string(c, false, kind);
            return;
        }
        if c.is_ascii_digit() {
            self.eat_number();
            return;
        }
        if c == '@' && self.at_line_start() {
            self.push_one(TokenKind::Operator);
            if self.i < self.len() && is_ident_start(self.cur()) {
                self.eat_identifier_kind(TokenKind::NameDecorator);
            }
            return;
        }
        if is_ident_start(c) {
            if self.try_prefixed_string() {
                return;
            }
            self.eat_identifier_word();
            return;
        }
        self.eat_punct();
    }

    fn scan_curly(&mut self) {
        let c = self.cur();
        if c == '/' && self.at(1) == Some('/') && self.spec.line_comment.starts_with('/') {
            self.eat_line_comment(2);
            return;
        }
        if c == '#' && self.spec.hash_comment {
            self.eat_line_comment(1);
            return;
        }
        if c == '/' && self.at(1) == Some('*') {
            self.eat_block_comment();
            return;
        }
        if c == '"' || c == '\'' || c == '`' {
            let kind = if c == '"' {
                TokenKind::LiteralStringDouble
            } else {
                TokenKind::LiteralStringSingle
            };
            self.eat_string(c, false, kind);
            return;
        }
        if c.is_ascii_digit() {
            self.eat_number();
            return;
        }
        if is_ident_start(c) {
            self.eat_identifier_word();
            return;
        }
        self.eat_punct();
    }

    fn scan_shell(&mut self) {
        let c = self.cur();
        if c == '#' {
            self.eat_line_comment(1);
            return;
        }
        if c == '"' || c == '\'' {
            let kind = if c == '"' {
                TokenKind::LiteralStringDouble
            } else {
                TokenKind::LiteralStringSingle
            };
            self.eat_string(c, false, kind);
            return;
        }
        if c == '$' {
            self.push_one(TokenKind::NameVariable);
            if self.i < self.len() && self.cur() == '{' {
                while self.i < self.len() && self.cur() != '}' && self.cur() != '\n' {
                    self.push_one(TokenKind::NameVariable);
                }
                if self.i < self.len() && self.cur() == '}' {
                    self.push_one(TokenKind::NameVariable);
                }
            } else {
                while self.i < self.len() && (is_ident_continue(self.cur()) || self.cur() == '?') {
                    self.push_one(TokenKind::NameVariable);
                }
            }
            return;
        }
        if c.is_ascii_digit() {
            self.eat_number();
            return;
        }
        if is_ident_start(c) {
            self.assert_word();
            return;
        }
        self.eat_punct();
    }

    /// 在不应用 Python 专属上下文规则的情况下对单词分类。
    fn assert_word(&mut self) {
        let start = self.i;
        while self.i < self.len() && is_ident_continue(self.cur()) {
            self.i += 1;
        }
        let word: String = self.chars[start..self.i].iter().collect();
        let kind = if self.spec.keywords.contains(&word.as_str()) {
            TokenKind::Keyword
        } else if self.spec.constants.contains(&word.as_str()) {
            TokenKind::KeywordConstant
        } else if self.spec.builtins.contains(&word.as_str()) {
            TokenKind::NameBuiltin
        } else {
            TokenKind::Name
        };
        for _ in start..self.i {
            self.kinds.push(kind);
        }
    }

    fn scan_json(&mut self) {
        let c = self.cur();
        if c == '"' {
            // 后跟 ':' 的字符串是对象键。
            let mut j = self.i + 1;
            while j < self.len() && self.chars[j] != '"' {
                if self.chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let mut k = j + 1;
            while k < self.len() && self.chars[k].is_whitespace() {
                k += 1;
            }
            let kind = if self.chars.get(k) == Some(&':') {
                TokenKind::NameTag
            } else {
                TokenKind::LiteralStringDouble
            };
            self.eat_string('"', false, kind);
            return;
        }
        if c.is_ascii_digit() || c == '-' {
            self.eat_number();
            return;
        }
        if is_ident_start(c) {
            self.assert_word();
            return;
        }
        self.eat_punct();
    }

    fn scan_config(&mut self) {
        let c = self.cur();
        if c == '#' {
            self.eat_line_comment(1);
            return;
        }
        if self.at_line_start() && (is_ident_start(c) || c == '"' || c == '\'') {
            let mut j = self.i;
            while j < self.len()
                && self.chars[j] != '\n'
                && self.chars[j] != ':'
                && self.chars[j] != '='
            {
                j += 1;
            }
            if self.chars.get(j).is_some_and(|c| *c == ':' || *c == '=') {
                while self.i < j {
                    self.push_one(TokenKind::NameAttribute);
                }
                return;
            }
        }
        if c == '"' || c == '\'' {
            let kind = if c == '"' {
                TokenKind::LiteralStringDouble
            } else {
                TokenKind::LiteralStringSingle
            };
            self.eat_string(c, false, kind);
            return;
        }
        if c.is_ascii_digit() {
            self.eat_number();
            return;
        }
        if is_ident_start(c) {
            let start = self.i;
            while self.i < self.len()
                && (is_ident_continue(self.cur()) || self.cur() == '-' || self.cur() == '.')
            {
                self.i += 1;
            }
            let word: String = self.chars[start..self.i].iter().collect();
            let kind = if self.spec.constants.contains(&word.as_str()) {
                TokenKind::KeywordConstant
            } else {
                TokenKind::Name
            };
            for _ in start..self.i {
                self.kinds.push(kind);
            }
            return;
        }
        self.eat_punct();
    }

    fn scan_markdown(&mut self) {
        let c = self.cur();
        if c == '#' && self.at_line_start() {
            while self.i < self.len() && self.cur() != '\n' {
                self.push_one(TokenKind::GenericHeading);
            }
            return;
        }
        if self.starts_with("```") {
            while self.i < self.len() && self.cur() != '\n' {
                self.push_one(TokenKind::LiteralStringBacktick);
            }
            while self.i < self.len() {
                if self.starts_with("```") {
                    while self.i < self.len() && self.cur() != '\n' {
                        self.push_one(TokenKind::LiteralStringBacktick);
                    }
                    return;
                }
                self.push_one(TokenKind::LiteralStringOther);
            }
            return;
        }
        if c == '`' {
            self.push_one(TokenKind::LiteralStringBacktick);
            while self.i < self.len() && self.cur() != '`' && self.cur() != '\n' {
                self.push_one(TokenKind::LiteralStringBacktick);
            }
            if self.i < self.len() && self.cur() == '`' {
                self.push_one(TokenKind::LiteralStringBacktick);
            }
            return;
        }
        if self.starts_with("**") {
            self.push_span(TokenKind::GenericStrong, 2);
            return;
        }
        if c == '*' || c == '_' {
            self.push_one(TokenKind::GenericEmph);
            return;
        }
        self.push_one(TokenKind::Text);
    }

    fn scan_plain(&mut self) {
        let c = self.cur();
        let marker = self.spec.line_comment;
        if !marker.is_empty() && !marker.starts_with('/') && self.starts_with(marker) {
            self.eat_line_comment(marker.chars().count());
            return;
        }
        if c == '/' && self.at(1) == Some('/') && marker.starts_with('/') {
            self.eat_line_comment(2);
            return;
        }
        if c == '/' && self.at(1) == Some('*') && self.spec.block_comment.is_some() {
            self.eat_block_comment();
            return;
        }
        if c == '#' && self.spec.hash_comment {
            self.eat_line_comment(1);
            return;
        }
        if c == '"' || c == '\'' {
            let kind = if c == '"' {
                TokenKind::LiteralStringDouble
            } else {
                TokenKind::LiteralStringSingle
            };
            self.eat_string(c, false, kind);
            return;
        }
        if c.is_ascii_digit() {
            self.eat_number();
            return;
        }
        if is_ident_start(c) {
            self.eat_identifier_word();
            return;
        }
        self.eat_punct();
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || (!c.is_ascii() && !c.is_whitespace())
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || (!c.is_ascii() && !c.is_whitespace())
}

// ---------------------------------------------------------------------------
// 关键字表
// ---------------------------------------------------------------------------

static PY_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
    "yield",
];
static PY_CONSTANTS: &[&str] = &["True", "False", "None", "NotImplemented", "Ellipsis"];
static PY_BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "bool",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "hasattr",
    "hash",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "map",
    "max",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "Exception",
    "ValueError",
    "TypeError",
    "KeyError",
    "IndexError",
    "RuntimeError",
    "AttributeError",
    "ImportError",
    "StopIteration",
    "ZeroDivisionError",
];

static RUST_KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "static",
    "struct",
    "super",
    "trait",
    "type",
    "union",
    "unsafe",
    "use",
    "where",
    "while",
    "macro_rules",
];
static RUST_TYPES: &[&str] = &[
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32",
    "f64", "bool", "char", "str", "String", "Vec", "Option", "Result", "Box", "Rc", "Arc",
    "HashMap", "HashSet", "BTreeMap", "RefCell", "Cell", "Mutex", "RwLock", "Path", "PathBuf",
    "Cow", "Self",
];
static RUST_CONSTANTS: &[&str] = &["true", "false", "None", "Some", "Ok", "Err"];

static GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];
static GO_TYPES: &[&str] = &[
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "any",
];
static GO_BUILTINS: &[&str] = &[
    "append", "cap", "clear", "close", "complex", "copy", "delete", "imag", "len", "make", "max",
    "min", "new", "panic", "print", "println", "real", "recover",
];

static C_KEYWORDS: &[&str] = &[
    "auto",
    "break",
    "case",
    "const",
    "continue",
    "default",
    "do",
    "else",
    "enum",
    "extern",
    "for",
    "goto",
    "if",
    "inline",
    "register",
    "restrict",
    "return",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "volatile",
    "while",
    "class",
    "namespace",
    "template",
    "typename",
    "using",
    "public",
    "private",
    "protected",
    "virtual",
    "override",
    "final",
    "new",
    "delete",
    "this",
    "throw",
    "try",
    "catch",
    "operator",
    "friend",
    "constexpr",
    "noexcept",
];
static C_TYPES: &[&str] = &[
    "char",
    "double",
    "float",
    "int",
    "long",
    "short",
    "signed",
    "unsigned",
    "void",
    "bool",
    "wchar_t",
    "size_t",
    "ssize_t",
    "uint8_t",
    "uint16_t",
    "uint32_t",
    "uint64_t",
    "int8_t",
    "int16_t",
    "int32_t",
    "int64_t",
    "string",
    "vector",
    "map",
    "set",
    "pair",
    "unique_ptr",
    "shared_ptr",
    "optional",
    "variant",
    "tuple",
];
static C_BUILTINS: &[&str] = &[
    "printf", "fprintf", "sprintf", "snprintf", "malloc", "calloc", "realloc", "free", "memcpy",
    "memset", "strlen", "strcpy", "strcmp", "puts", "putchar", "exit", "std", "cout", "cin",
    "endl",
];

static JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "for",
    "if",
    "implements",
    "import",
    "instanceof",
    "interface",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "sealed",
    "static",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "var",
    "volatile",
    "while",
    "yield",
];
static JAVA_TYPES: &[&str] = &[
    "boolean",
    "byte",
    "char",
    "double",
    "float",
    "int",
    "long",
    "short",
    "void",
    "String",
    "Object",
    "Integer",
    "Double",
    "Long",
    "Boolean",
    "List",
    "ArrayList",
    "Map",
    "HashMap",
    "Set",
    "HashSet",
    "Optional",
    "Stream",
    "Exception",
    "Thread",
];

static CSHARP_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "base",
    "break",
    "case",
    "catch",
    "checked",
    "class",
    "const",
    "continue",
    "default",
    "delegate",
    "do",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "finally",
    "fixed",
    "for",
    "foreach",
    "get",
    "goto",
    "if",
    "implicit",
    "in",
    "interface",
    "internal",
    "is",
    "lock",
    "namespace",
    "new",
    "operator",
    "out",
    "override",
    "params",
    "partial",
    "private",
    "protected",
    "public",
    "readonly",
    "record",
    "ref",
    "return",
    "sealed",
    "set",
    "sizeof",
    "stackalloc",
    "static",
    "struct",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "unchecked",
    "unsafe",
    "using",
    "var",
    "virtual",
    "volatile",
    "where",
    "while",
    "yield",
];
static CSHARP_TYPES: &[&str] = &[
    "bool",
    "byte",
    "char",
    "decimal",
    "double",
    "float",
    "int",
    "long",
    "object",
    "sbyte",
    "short",
    "string",
    "uint",
    "ulong",
    "ushort",
    "void",
    "Task",
    "List",
    "Dictionary",
    "IEnumerable",
];

static KOTLIN_KEYWORDS: &[&str] = &[
    "as",
    "break",
    "class",
    "continue",
    "do",
    "else",
    "for",
    "fun",
    "if",
    "in",
    "interface",
    "is",
    "object",
    "package",
    "return",
    "super",
    "this",
    "throw",
    "try",
    "typealias",
    "val",
    "var",
    "when",
    "while",
    "by",
    "catch",
    "constructor",
    "finally",
    "get",
    "import",
    "init",
    "private",
    "protected",
    "public",
    "sealed",
    "set",
    "suspend",
    "data",
    "enum",
    "internal",
    "open",
    "override",
    "abstract",
    "companion",
    "const",
    "lateinit",
    "operator",
    "out",
    "reified",
];
static SWIFT_KEYWORDS: &[&str] = &[
    "associatedtype",
    "class",
    "deinit",
    "enum",
    "extension",
    "fileprivate",
    "func",
    "import",
    "init",
    "inout",
    "internal",
    "let",
    "open",
    "operator",
    "private",
    "protocol",
    "public",
    "rethrows",
    "static",
    "struct",
    "subscript",
    "typealias",
    "var",
    "break",
    "case",
    "continue",
    "default",
    "defer",
    "do",
    "else",
    "fallthrough",
    "for",
    "guard",
    "if",
    "in",
    "repeat",
    "return",
    "switch",
    "where",
    "while",
    "as",
    "catch",
    "is",
    "throw",
    "throws",
    "try",
    "await",
    "async",
    "actor",
    "some",
    "any",
];

static JS_KEYWORDS: &[&str] = &[
    "var",
    "let",
    "const",
    "function",
    "return",
    "if",
    "else",
    "for",
    "while",
    "do",
    "break",
    "continue",
    "new",
    "delete",
    "typeof",
    "instanceof",
    "in",
    "of",
    "this",
    "class",
    "extends",
    "super",
    "import",
    "export",
    "default",
    "async",
    "await",
    "yield",
    "try",
    "catch",
    "finally",
    "throw",
    "switch",
    "case",
    "void",
    "with",
    "debugger",
    "interface",
    "type",
    "enum",
    "implements",
    "declare",
    "namespace",
    "abstract",
    "readonly",
    "keyof",
    "infer",
    "satisfies",
];
static JS_TYPES: &[&str] = &[
    "string", "number", "boolean", "any", "unknown", "never", "object", "symbol", "bigint",
];
static JS_BUILTINS: &[&str] = &[
    "console",
    "document",
    "window",
    "Math",
    "JSON",
    "Object",
    "Array",
    "String",
    "Number",
    "Boolean",
    "Promise",
    "Map",
    "Set",
    "Symbol",
    "Date",
    "RegExp",
    "Error",
    "parseInt",
    "parseFloat",
    "isNaN",
    "require",
    "module",
    "exports",
    "process",
    "setTimeout",
    "setInterval",
    "fetch",
];

static PHP_KEYWORDS: &[&str] = &[
    "abstract",
    "and",
    "array",
    "as",
    "break",
    "callable",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "enum",
    "extends",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "if",
    "implements",
    "include",
    "include_once",
    "instanceof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "require_once",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "xor",
    "yield",
];
static PHP_BUILTINS: &[&str] = &[
    "strlen",
    "count",
    "array_map",
    "array_filter",
    "implode",
    "explode",
    "sprintf",
    "printf",
    "var_dump",
    "print_r",
    "is_array",
    "is_string",
    "is_null",
    "json_encode",
    "json_decode",
    "preg_match",
    "trim",
    "substr",
];

static RUBY_KEYWORDS: &[&str] = &[
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "for",
    "if",
    "in",
    "module",
    "next",
    "not",
    "or",
    "redo",
    "rescue",
    "retry",
    "return",
    "self",
    "super",
    "then",
    "unless",
    "until",
    "when",
    "while",
    "yield",
    "require",
    "require_relative",
    "attr_accessor",
    "attr_reader",
    "attr_writer",
    "puts",
    "raise",
    "lambda",
    "proc",
];

static SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "function", "in", "select", "time", "return", "exit", "break", "continue", "local", "export",
    "readonly", "declare", "unset", "shift", "source", "alias", "set", "trap", "eval", "exec",
];
static SHELL_BUILTINS: &[&str] = &[
    "echo", "printf", "read", "cd", "pwd", "ls", "cp", "mv", "rm", "mkdir", "touch", "cat", "grep",
    "sed", "awk", "find", "xargs", "sort", "uniq", "head", "tail", "cut", "tr", "wc", "chmod",
    "curl", "wget", "git", "make", "sudo", "apt", "brew", "pip", "python", "node", "npm", "cargo",
    "go", "docker", "kubectl",
];

static SQL_KEYWORDS: &[&str] = &[
    "select",
    "from",
    "where",
    "insert",
    "into",
    "values",
    "update",
    "set",
    "delete",
    "create",
    "table",
    "alter",
    "drop",
    "index",
    "view",
    "join",
    "inner",
    "left",
    "right",
    "outer",
    "full",
    "on",
    "group",
    "by",
    "order",
    "having",
    "limit",
    "offset",
    "union",
    "all",
    "distinct",
    "as",
    "and",
    "or",
    "not",
    "is",
    "in",
    "between",
    "like",
    "exists",
    "case",
    "when",
    "then",
    "else",
    "end",
    "with",
    "primary",
    "key",
    "foreign",
    "references",
    "default",
    "constraint",
    "unique",
    "begin",
    "commit",
    "rollback",
    "SELECT",
    "FROM",
    "WHERE",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "DELETE",
    "CREATE",
    "TABLE",
    "JOIN",
    "ORDER",
    "GROUP",
    "LIMIT",
    "NULL",
    "IS",
];
static SQL_TYPES: &[&str] = &[
    "int",
    "integer",
    "bigint",
    "smallint",
    "serial",
    "text",
    "varchar",
    "char",
    "boolean",
    "date",
    "timestamp",
    "numeric",
    "decimal",
    "real",
    "double",
    "json",
    "jsonb",
    "uuid",
    "INT",
    "INTEGER",
    "BIGINT",
    "TEXT",
    "VARCHAR",
    "BOOLEAN",
    "DATE",
    "TIMESTAMP",
    "NUMERIC",
];
static SQL_BUILTINS: &[&str] = &[
    "count",
    "sum",
    "avg",
    "min",
    "max",
    "coalesce",
    "cast",
    "now",
    "length",
    "lower",
    "upper",
    "trim",
    "substring",
    "concat",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "COALESCE",
    "CAST",
];
