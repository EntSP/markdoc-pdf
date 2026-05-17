//! Tiny built-in syntax highlighter for fenced code blocks.
//!
//! Scope is intentionally narrow: this is a hand-rolled lexer that
//! tags four token classes (keyword / string / comment / number) for a
//! handful of languages commonly used in technical documentation.
//! Languages without a dedicated lexer fall back to plain rendering.
//!
//! The output is a `Vec<TokenSpan>` describing byte ranges and their
//! class. The caller maps each span to an `InlineRange` carrying the
//! palette colour from `Style::code_highlight`.
//!
//! Why not syntect? syntect ships Sublime grammar bundles that double
//! the binary size and pull in a regex engine. Documentation rarely
//! needs more than keyword/string/comment colouring, so a 200-line
//! lexer covers the typical case without the dependency cost.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenClass {
    Keyword,
    String,
    Comment,
    Number,
}

#[derive(Debug, Clone, Copy)]
pub struct TokenSpan {
    pub start: usize,
    pub end: usize,
    pub class: TokenClass,
}

/// Tokenise `source` for the language identified by `lang`. Returns
/// an empty vec when the language isn't recognised — the caller then
/// renders the block uncoloured.
pub fn tokenize(lang: &str, source: &str) -> Vec<TokenSpan> {
    let lang = lang.trim();
    let eq = |name: &str| lang.eq_ignore_ascii_case(name);
    if eq("rust") {
        lex_c_like(source, RUST_KEYWORDS, true, true, true)
    } else if eq("javascript") || eq("typescript") || eq("js") || eq("ts") || eq("jsx") || eq("tsx")
    {
        lex_c_like(source, JS_KEYWORDS, true, true, true)
    } else if eq("c") || eq("cpp") || eq("c++") || eq("h") || eq("hpp") {
        lex_c_like(source, C_KEYWORDS, true, true, false)
    } else if eq("go") {
        lex_c_like(source, GO_KEYWORDS, true, true, true)
    } else if eq("java") {
        lex_c_like(source, JAVA_KEYWORDS, true, true, false)
    } else if eq("python") || eq("py") {
        lex_python(source)
    } else if eq("bash") || eq("sh") || eq("shell") || eq("zsh") {
        lex_shell(source)
    } else if eq("json") {
        lex_json(source)
    } else if eq("toml") {
        lex_hash_keywords(source, &[])
    } else if eq("yaml") || eq("yml") {
        lex_hash_keywords(source, YAML_KEYWORDS)
    } else {
        Vec::new()
    }
}

// ── Lexer combinators ────────────────────────────────────────────────────

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "yield", "box",
];

const JS_KEYWORDS: &[&str] = &[
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "async",
    "interface",
    "type",
    "enum",
    "implements",
    "private",
    "protected",
    "public",
    "readonly",
    "static",
];

const C_KEYWORDS: &[&str] = &[
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
    "bool",
    "true",
    "false",
    "nullptr",
    "namespace",
    "class",
    "public",
    "private",
    "protected",
    "virtual",
    "override",
    "template",
    "typename",
    "new",
    "delete",
    "this",
    "operator",
];

const GO_KEYWORDS: &[&str] = &[
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
    "true",
    "false",
    "nil",
    "iota",
];

const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "void",
    "volatile",
    "while",
    "var",
    "yield",
    "record",
    "sealed",
    "permits",
];

const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "match", "case",
];

const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "case", "esac", "for", "select", "while", "until", "do",
    "done", "in", "function", "time", "coproc", "return", "exit", "break", "continue", "local",
    "readonly", "export", "declare", "set", "unset", "shift", "trap", "eval", "exec", "source",
    "true", "false",
];

const YAML_KEYWORDS: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// C-family lexer: line `//` comments, block `/* ... */` comments,
/// `"..."` string literals, decimal/hex numbers, identifiers checked
/// against `keywords`. `block_comments` and `single_line_comments`
/// toggle the comment styles; `lifetimes` is a Rust-specific switch
/// that prevents `'a` from being lexed as an unterminated character
/// literal.
fn lex_c_like(
    source: &str,
    keywords: &[&str],
    single_line_comments: bool,
    block_comments: bool,
    lifetimes: bool,
) -> Vec<TokenSpan> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];

        if single_line_comments && b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::Comment,
            });
            continue;
        }
        if block_comments && b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::Comment,
            });
            continue;
        }
        if b == b'"' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            i = (i + 1).min(bytes.len());
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::String,
            });
            continue;
        }
        if b == b'\'' {
            // Lifetime in Rust (e.g. 'static) — lex as keyword, not string.
            if lifetimes && i + 1 < bytes.len() && is_ident_start(bytes[i + 1] as char) {
                let start = i;
                i += 1;
                while i < bytes.len() && is_ident_cont(bytes[i] as char) {
                    i += 1;
                }
                // If followed by another quote it was actually a char
                // literal; fall back to string class.
                if i < bytes.len() && bytes[i] == b'\'' {
                    i += 1;
                    out.push(TokenSpan {
                        start,
                        end: i,
                        class: TokenClass::String,
                    });
                } else {
                    out.push(TokenSpan {
                        start,
                        end: i,
                        class: TokenClass::Keyword,
                    });
                }
                continue;
            }
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            i = (i + 1).min(bytes.len());
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::String,
            });
            continue;
        }
        if b.is_ascii_digit() {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_')
            {
                i += 1;
            }
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::Number,
            });
            continue;
        }
        if is_ident_start(b as char) {
            let start = i;
            while i < bytes.len() && is_ident_cont(bytes[i] as char) {
                i += 1;
            }
            let word = &source[start..i];
            if keywords.contains(&word) {
                out.push(TokenSpan {
                    start,
                    end: i,
                    class: TokenClass::Keyword,
                });
            }
            continue;
        }
        i += 1;
    }
    out
}

fn lex_python(source: &str) -> Vec<TokenSpan> {
    lex_hash_keywords(source, PYTHON_KEYWORDS)
}

fn lex_shell(source: &str) -> Vec<TokenSpan> {
    lex_hash_keywords(source, SHELL_KEYWORDS)
}

/// Lexer for `#`-comment languages with double- and single-quoted
/// strings and a keyword set. Used for shell, Python, YAML, TOML.
fn lex_hash_keywords(source: &str, keywords: &[&str]) -> Vec<TokenSpan> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'#' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::Comment,
            });
            continue;
        }
        if b == b'"' || b == b'\'' {
            let quote = b;
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            i = (i + 1).min(bytes.len());
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::String,
            });
            continue;
        }
        if b.is_ascii_digit() {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_')
            {
                i += 1;
            }
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::Number,
            });
            continue;
        }
        if is_ident_start(b as char) {
            let start = i;
            while i < bytes.len() && is_ident_cont(bytes[i] as char) {
                i += 1;
            }
            let word = &source[start..i];
            if keywords.contains(&word) {
                out.push(TokenSpan {
                    start,
                    end: i,
                    class: TokenClass::Keyword,
                });
            }
            continue;
        }
        i += 1;
    }
    out
}

/// JSON has no comments per the spec but we tolerate `//` and `#` for
/// the JSON-with-comments dialects (json5, hjson) since they're common
/// in documentation. Highlights strings, numbers, and the `true /
/// false / null` literals.
fn lex_json(source: &str) -> Vec<TokenSpan> {
    const JSON_KEYWORDS: &[&str] = &["true", "false", "null"];
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'#' || (b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/') {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::Comment,
            });
            continue;
        }
        if b == b'"' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            i = (i + 1).min(bytes.len());
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::String,
            });
            continue;
        }
        if b.is_ascii_digit() || (b == b'-' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'.'
                    || bytes[i] == b'+'
                    || bytes[i] == b'-')
            {
                i += 1;
            }
            out.push(TokenSpan {
                start,
                end: i,
                class: TokenClass::Number,
            });
            continue;
        }
        if is_ident_start(b as char) {
            let start = i;
            while i < bytes.len() && is_ident_cont(bytes[i] as char) {
                i += 1;
            }
            if JSON_KEYWORDS.contains(&&source[start..i]) {
                out.push(TokenSpan {
                    start,
                    end: i,
                    class: TokenClass::Keyword,
                });
            }
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(spans: &[TokenSpan], src: &str) -> Vec<(TokenClass, String)> {
        spans
            .iter()
            .map(|s| (s.class, src[s.start..s.end].to_string()))
            .collect()
    }

    #[test]
    fn rust_keywords_strings_comments() {
        let src = r#"// hi
fn main() { let s = "hello"; }"#;
        let toks = tokenize("rust", src);
        let cls = classes(&toks, src);
        assert!(cls.contains(&(TokenClass::Comment, "// hi".to_string())));
        assert!(cls.contains(&(TokenClass::Keyword, "fn".to_string())));
        assert!(cls.contains(&(TokenClass::Keyword, "let".to_string())));
        assert!(cls.contains(&(TokenClass::String, "\"hello\"".to_string())));
    }

    #[test]
    fn rust_lifetime_is_keyword() {
        let src = "fn f<'a>(x: &'a str) {}";
        let toks = tokenize("rust", src);
        let cls = classes(&toks, src);
        assert!(
            cls.iter()
                .any(|(c, t)| *c == TokenClass::Keyword && t == "'a")
        );
    }

    #[test]
    fn json_string_and_number() {
        let src = r#"{"x": 42, "y": "hi"}"#;
        let toks = tokenize("json", src);
        let cls = classes(&toks, src);
        assert!(cls.contains(&(TokenClass::Number, "42".to_string())));
        assert!(cls.contains(&(TokenClass::String, "\"x\"".to_string())));
    }

    #[test]
    fn shell_comment_and_keyword() {
        let src = "# hi\nif true; then echo hi; fi";
        let toks = tokenize("bash", src);
        let cls = classes(&toks, src);
        assert!(cls.contains(&(TokenClass::Comment, "# hi".to_string())));
        assert!(cls.contains(&(TokenClass::Keyword, "if".to_string())));
        assert!(cls.contains(&(TokenClass::Keyword, "then".to_string())));
    }

    #[test]
    fn unknown_language_yields_no_tokens() {
        assert!(tokenize("brainfuck", "+++++").is_empty());
        assert!(tokenize("", "anything").is_empty());
    }
}
