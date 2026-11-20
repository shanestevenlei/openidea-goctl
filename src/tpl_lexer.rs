//! Highlighting for go-zero `goctl` template files (`.tpl`).
//!
//! goctl templates are Go `text/template` documents: ordinary Go source with
//! `{{ … }}` actions embedded. We scan once and emit tokens for both worlds —
//! template comments `{{/* … */}}`, action delimiters, template keywords
//! (`if`, `else`, `end`, `range`, `with`, `template`, `define`, `block`), the
//! `.field` / `$var` references inside actions, and Go string literals, line
//! comments, keywords and numbers outside actions. Spans are UTF-8 byte
//! offsets into the source text.

use openidea_plugin::syntax::Token as SyntaxToken;
use openidea_plugin::syntax::{Fold, FoldKind, Highlight};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    ActionDelim,
    Keyword,
    Field,
    String,
    Comment,
    Number,
    Operator,
    Punctuation,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

/// Template actions that change control flow or define blocks.
const TEMPLATE_KEYWORDS: &[&str] = &[
    "if", "else", "end", "range", "with", "template", "define", "block", "nil",
];

/// Built-in template functions, highlighted as keywords when seen bare.
const TEMPLATE_FUNCS: &[&str] = &[
    "and", "or", "not", "eq", "ne", "lt", "le", "gt", "ge", "len", "index", "slice", "print",
    "printf", "println", "call", "html", "js", "urlquery", "index",
];

/// Go keywords that can appear in the literal text of a template.
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
];

/// Tokenize a `.tpl` document. Spans are UTF-8 byte offsets into `text`.
pub fn lex(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut text_start = 0;

    // Flush the plain text run between actions so far, as one Text token.
    let flush_text = |tokens: &mut Vec<Token>, from: usize, to: usize| {
        if to > from {
            tokens.push(Token {
                start: from,
                end: to,
                kind: TokenKind::Text,
            });
        }
    };

    while i < bytes.len() {
        if bytes[i] == b'{' && bytes.get(i + 1) == Some(&b'{') {
            flush_text(&mut tokens, text_start, i);

            let action_start = i;
            i += 2;
            // Trim marker `{{-` / `-}}` counts as part of the delimiter.
            let mut inner = i;
            if bytes.get(inner) == Some(&b'-') {
                inner += 1;
            }
            // Template comment action: {{/* … */}} (optionally trimmed).
            if bytes.get(inner) == Some(&b'/') && bytes.get(inner + 1) == Some(&b'*') {
                let mut j = inner + 2;
                while j < bytes.len() {
                    if bytes[j] == b'*' && bytes.get(j + 1) == Some(&b'/') {
                        j += 2;
                        break;
                    }
                    j += char_len_at(text, j);
                }
                // Consume `-}}` if present.
                if bytes.get(j) == Some(&b'-') {
                    j += 1;
                }
                if bytes.get(j) == Some(&b'}') && bytes.get(j + 1) == Some(&b'}') {
                    j += 2;
                }
                tokens.push(Token {
                    start: action_start,
                    end: j,
                    kind: TokenKind::Comment,
                });
                i = j;
                text_start = i;
                continue;
            }
            // Ordinary action: open delimiter token.
            tokens.push(Token {
                start: action_start,
                end: inner,
                kind: TokenKind::ActionDelim,
            });
            lex_action(text, inner, &mut tokens, &mut i);
            // `i` now points just past a closing `}}` (or at EOF if unterminated).
            if i != usize::MAX {
                text_start = i;
            } else {
                i = bytes.len();
            }
            continue;
        }

        // Plain Go text outside an action: highlight comments / strings / keywords.
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            flush_text(&mut tokens, text_start, i);
            let mut j = i + 2;
            while j < bytes.len() && bytes[j] != b'\n' && bytes[j] != b'\r' {
                j += char_len_at(text, j);
            }
            tokens.push(Token {
                start: i,
                end: j,
                kind: TokenKind::Comment,
            });
            i = j;
            text_start = i;
            continue;
        }
        if bytes[i] == b'"' || bytes[i] == b'`' {
            flush_text(&mut tokens, text_start, i);
            let j = scan_string(text, i);
            tokens.push(Token {
                start: i,
                end: j,
                kind: TokenKind::String,
            });
            i = j;
            text_start = i;
            continue;
        }
        if is_ident_start(bytes[i]) {
            let mut j = i + 1;
            while j < bytes.len() && is_ident_continue(bytes[j]) {
                j += 1;
            }
            if GO_KEYWORDS.contains(&&text[i..j]) {
                flush_text(&mut tokens, text_start, i);
                tokens.push(Token {
                    start: i,
                    end: j,
                    kind: TokenKind::Keyword,
                });
                text_start = j;
            }
            i = j;
            continue;
        }
        i += char_len_at(text, i);
    }
    flush_text(&mut tokens, text_start, bytes.len());
    tokens
}

/// Lex the inside of one `{{ … }}` action, starting at `start`. Advances `*i`
/// to just past the closing `}}` (or to `usize::MAX` if unterminated).
fn lex_action(text: &str, start: usize, tokens: &mut Vec<Token>, i: &mut usize) {
    let bytes = text.as_bytes();
    let mut p = start;
    while p < bytes.len() {
        let byte = bytes[p];
        if byte.is_ascii_whitespace() {
            p += 1;
            continue;
        }
        if byte == b'-' && bytes.get(p + 1) == Some(&b'}') && bytes.get(p + 2) == Some(&b'}') {
            tokens.push(Token {
                start: p,
                end: p + 3,
                kind: TokenKind::ActionDelim,
            });
            *i = p + 3;
            return;
        }
        if byte == b'}' && bytes.get(p + 1) == Some(&b'}') {
            tokens.push(Token {
                start: p,
                end: p + 2,
                kind: TokenKind::ActionDelim,
            });
            *i = p + 2;
            return;
        }
        // Field or variable reference: `.name`, `.` , `$.name`, `$var`.
        if byte == b'.' {
            let mut j = p + 1;
            while j < bytes.len() && is_ident_continue(bytes[j]) {
                j += 1;
            }
            tokens.push(Token {
                start: p,
                end: j,
                kind: TokenKind::Field,
            });
            p = j;
            continue;
        }
        if byte == b'$' {
            let mut j = p + 1;
            while j < bytes.len() && is_ident_continue(bytes[j]) {
                j += 1;
            }
            tokens.push(Token {
                start: p,
                end: j,
                kind: TokenKind::Field,
            });
            p = j;
            continue;
        }
        if byte == b'"' || byte == b'`' {
            let j = scan_string(text, p);
            tokens.push(Token {
                start: p,
                end: j,
                kind: TokenKind::String,
            });
            p = j;
            continue;
        }
        if is_ident_start(byte) {
            let mut j = p + 1;
            while j < bytes.len() && is_ident_continue(bytes[j]) {
                j += 1;
            }
            let word = &text[p..j];
            let kind = if TEMPLATE_KEYWORDS.contains(&word) || TEMPLATE_FUNCS.contains(&word) {
                TokenKind::Keyword
            } else {
                TokenKind::Text
            };
            tokens.push(Token {
                start: p,
                end: j,
                kind,
            });
            p = j;
            continue;
        }
        if byte.is_ascii_digit() {
            let mut j = p + 1;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            tokens.push(Token {
                start: p,
                end: j,
                kind: TokenKind::Number,
            });
            p = j;
            continue;
        }
        let kind = match byte {
            b'(' | b')' | b',' | b':' | b'|' => TokenKind::Punctuation,
            _ => TokenKind::Operator,
        };
        tokens.push(Token {
            start: p,
            end: p + 1,
            kind,
        });
        p += 1;
    }
    *i = usize::MAX;
}

fn scan_string(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let quote = bytes[start];
    let mut j = start + 1;
    while j < bytes.len() {
        if bytes[j] == quote {
            return j + 1;
        }
        if quote == b'"' && bytes[j] == b'\\' && j + 1 < bytes.len() {
            j += 1 + char_len_at(text, j + 1);
        } else if quote == b'"' && matches!(bytes[j], b'\n' | b'\r') {
            return j;
        } else {
            j += char_len_at(text, j);
        }
    }
    j
}

fn char_len_at(text: &str, byte: usize) -> usize {
    text[byte..].chars().next().map_or(1, char::len_utf8)
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Map the lexer output to editor highlight tokens (Text runs carry no color).
pub fn syntax_tokens(text: &str) -> Vec<SyntaxToken> {
    lex(text)
        .into_iter()
        .filter_map(|token| {
            let kind = match token.kind {
                TokenKind::Text => return None,
                TokenKind::ActionDelim => Highlight::Operator,
                TokenKind::Keyword => Highlight::Keyword,
                TokenKind::Field => Highlight::Property,
                TokenKind::String => Highlight::String,
                TokenKind::Comment => Highlight::Comment,
                TokenKind::Number => Highlight::Number,
                TokenKind::Operator => Highlight::Operator,
                TokenKind::Punctuation => Highlight::Punctuation,
            };
            Some(SyntaxToken {
                range: token.start..token.end,
                kind,
            })
        })
        .collect()
}

/// Fold each multi-line `{{ if/range/with/define/block }} … {{ end }}` block.
pub fn folding(text: &str) -> Vec<Fold> {
    let mut folds = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    for token in lex(text) {
        if token.kind != TokenKind::Keyword {
            continue;
        }
        let word = &text[token.start..token.end];
        let line = line_number(text, token.start);
        if matches!(word, "if" | "range" | "with" | "define" | "block") {
            stack.push(line);
        } else if word == "end"
            && let Some(start_line) = stack.pop()
            && line > start_line
        {
            folds.push(Fold {
                start_line,
                end_line: line,
                kind: FoldKind::Region,
            });
        }
    }
    folds.sort_by_key(|fold| (fold.start_line, fold.end_line));
    folds
}

fn line_number(text: &str, byte: usize) -> u32 {
    text.as_bytes()[..byte.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::{TokenKind, folding, lex, syntax_tokens};
    use openidea_plugin::syntax::{FoldKind, Highlight};

    #[test]
    fn lexes_go_text_and_template_actions() {
        let src = "package {{.pkgName}}\n{{if .hasDoc}}{{.doc}}{{end}}\n";
        let tokens = lex(src);
        // Go keyword `package` is highlighted even outside an action.
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == TokenKind::Keyword && &src[t.start..t.end] == "package")
        );
        // `.pkgName` field ref inside the first action.
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == TokenKind::Field && &src[t.start..t.end] == ".pkgName")
        );
        // Control keyword `if` and `end` inside actions.
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == TokenKind::Keyword && &src[t.start..t.end] == "if")
        );
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == TokenKind::Keyword && &src[t.start..t.end] == "end")
        );
    }

    #[test]
    fn treats_template_comment_action_as_a_comment() {
        let src = "{{/* note: safe to edit */}}\ncode";
        let tokens = lex(src);
        let comment = tokens
            .iter()
            .find(|t| t.kind == TokenKind::Comment)
            .expect("comment");
        assert_eq!(
            &src[comment.start..comment.end],
            "{{/* note: safe to edit */}}"
        );
    }

    #[test]
    fn unterminated_action_is_bounded() {
        let src = "x {{ .name";
        // Must not panic and must not run past the end.
        let tokens = lex(src);
        assert!(tokens.iter().all(|t| t.end <= src.len()));
    }

    #[test]
    fn syntax_tokens_skip_plain_text_and_mark_fields() {
        // Plain text identifiers inside an action (`.logic`, arg names) are Text and
        // emit no colored token; `.logic` itself is a Field → Property.
        let src = "{{.logic}}";
        let out = syntax_tokens(src);
        assert!(out.iter().any(|t| t.kind == Highlight::Property));
        assert!(out.iter().all(|t| &src[t.range.clone()] != "logic"));
    }

    #[test]
    fn folds_multiline_if_blocks() {
        let src = "a\n{{if .x}}\nb\nc\n{{end}}\nd\n";
        let folds = folding(src);
        assert_eq!(folds.len(), 1);
        assert_eq!(folds[0].kind, FoldKind::Region);
        assert_eq!((folds[0].start_line, folds[0].end_line), (1, 4));
    }

    #[test]
    fn folding_ignores_end_delimiters_inside_action_strings_and_comments() {
        let src = r#"{{if .x}}
{{printf "}}" "literal {{end}}" .x}}
{{/* literal }} {{end}} still comment */}}
{{.x}}
{{end}}
"#;
        let folds = folding(src);
        assert_eq!(folds.len(), 1);
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 4));
    }

    #[test]
    fn unicode_before_action_keeps_byte_offsets() {
        let src = "// 中文\n{{.name}}";
        let tokens = lex(src);
        let field = tokens
            .iter()
            .find(|t| t.kind == TokenKind::Field)
            .expect("field");
        assert_eq!(&src[field.start..field.end], ".name");
        assert_eq!(field.start, src.find(".name").unwrap());
    }
}
