use openidea_plugin::syntax::{
    self, Fold, FoldKind, Highlight, Symbol, SymbolKind, Token as SyntaxToken,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Identifier,
    Type,
    Number,
    String,
    Comment,
    Operator,
    Punctuation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

const KEYWORDS: &[&str] = &[
    "syntax",
    "info",
    "map",
    "struct",
    "interface",
    "type",
    "returns",
    "import",
    "as",
    "service",
    "get",
    "head",
    "post",
    "put",
    "patch",
    "delete",
    "connect",
    "options",
    "trace",
    "@server",
    "@doc",
    "@handler",
];

const BUILTIN_TYPES: &[&str] = &[
    "bool",
    "byte",
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
    "interface{}",
];

/// Tokenize a `.api` document. Spans are UTF-8 byte offsets into `text`.
pub fn lex(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];
        if byte.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        let start = i;
        if byte == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += char_len_at(text, i);
            }
            tokens.push(Token {
                start,
                end: i,
                kind: TokenKind::Comment,
            });
            continue;
        }
        if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < bytes.len() {
                if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    i += 2;
                    break;
                }
                i += char_len_at(text, i);
            }
            tokens.push(Token {
                start,
                end: i,
                kind: TokenKind::Comment,
            });
            continue;
        }
        if byte == b'"' || byte == b'`' {
            let quote = byte;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                if quote == b'"' && bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1 + char_len_at(text, i + 1);
                } else if quote == b'"' && matches!(bytes[i], b'\n' | b'\r') {
                    break;
                } else {
                    i += char_len_at(text, i);
                }
            }
            tokens.push(Token {
                start,
                end: i,
                kind: TokenKind::String,
            });
            continue;
        }
        if byte == b'@' {
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            if i == start + 1 {
                tokens.push(Token {
                    start,
                    end: i,
                    kind: TokenKind::Punctuation,
                });
            } else {
                tokens.push(Token {
                    start,
                    end: i,
                    kind: TokenKind::Keyword,
                });
            }
            continue;
        }
        if is_ident_start(byte) {
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let word = &text[start..i];
            let kind = if KEYWORDS.contains(&word) {
                TokenKind::Keyword
            } else if BUILTIN_TYPES.contains(&word) {
                TokenKind::Type
            } else {
                TokenKind::Identifier
            };
            tokens.push(Token {
                start,
                end: i,
                kind,
            });
            continue;
        }
        if byte.is_ascii_digit() {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            tokens.push(Token {
                start,
                end: i,
                kind: TokenKind::Number,
            });
            continue;
        }

        i += 1;
        let kind = match byte {
            b'=' | b'-' | b'*' | b'/' => TokenKind::Operator,
            b'(' | b')' | b'{' | b'}' | b'[' | b']' | b'.' | b';' | b',' | b':' => {
                TokenKind::Punctuation
            }
            _ => continue,
        };
        tokens.push(Token {
            start,
            end: i,
            kind,
        });
    }
    tokens
}

/// Return top-level type and service declarations for the structure view.
pub fn outline(text: &str) -> Vec<Symbol> {
    let tokens = lex(text);
    let mut symbols = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let declaration = match tokens[i].kind {
            TokenKind::Keyword if &text[tokens[i].start..tokens[i].end] == "type" => {
                Some((SymbolKind::Struct, true))
            }
            TokenKind::Keyword if &text[tokens[i].start..tokens[i].end] == "service" => {
                Some((SymbolKind::Interface, false))
            }
            _ => None,
        };
        let Some((kind, is_type)) = declaration else {
            i += 1;
            continue;
        };

        let declaration_start = tokens[i].start;
        let mut name_index = i + 1;
        if is_type
            && tokens.get(name_index).is_some_and(|token| {
                token.kind == TokenKind::Punctuation && &text[token.start..token.end] == "("
            })
        {
            name_index += 1;
        }
        while name_index < tokens.len() && tokens[name_index].kind != TokenKind::Identifier {
            if tokens[name_index].kind == TokenKind::Keyword
                && matches!(
                    &text[tokens[name_index].start..tokens[name_index].end],
                    "type" | "service"
                )
            {
                break;
            }
            name_index += 1;
        }
        if let Some(name) = tokens
            .get(name_index)
            .filter(|token| token.kind == TokenKind::Identifier)
        {
            let mut name_end_index = name_index;
            if !is_type {
                while name_end_index + 2 < tokens.len()
                    && token_text(&tokens[name_end_index + 1], text) == "-"
                    && tokens[name_end_index + 2].kind == TokenKind::Identifier
                    && tokens[name_end_index].end == tokens[name_end_index + 1].start
                    && tokens[name_end_index + 1].end == tokens[name_end_index + 2].start
                {
                    name_end_index += 2;
                }
            }
            let body_open = declaration_body_open(&tokens, name_end_index, text, is_type);
            let name_end = tokens[name_end_index].end;
            let declaration_end = body_open
                .and_then(|open| matching_brace_end(&tokens, open, text))
                .unwrap_or_else(|| line_end(text, name_end));
            let symbol_kind = if is_type && body_open.is_none() {
                SymbolKind::TypeAlias
            } else {
                kind
            };
            symbols.push(Symbol {
                name: text[name.start..name_end].to_owned(),
                detail: None,
                kind: symbol_kind,
                range: declaration_start..declaration_end,
                name_range: name.start..name_end,
                parent: None,
            });
            i = name_end_index + 1;
        } else {
            i += 1;
        }
    }
    symbols
}

/// Return multi-line brace regions and adjacent line/block comment folds.
pub fn folding(text: &str) -> Vec<Fold> {
    let tokens = lex(text);
    let skip: Vec<_> = tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::String | TokenKind::Comment))
        .map(|token| token.start..token.end)
        .collect();
    let mut folds = syntax::bracket_folds(text, b'{', b'}', &skip);

    let comments: Vec<_> = tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Comment)
        .collect();
    let mut group: Option<(usize, usize, u32, u32)> = None;
    for comment in comments {
        let start_line = line_number(text, comment.start);
        let end_line = line_number(text, comment.end.saturating_sub(1));
        match group {
            Some((start, end, first_line, last_line))
                if text[end..comment.start].chars().all(char::is_whitespace)
                    && start_line <= last_line + 1 =>
            {
                group = Some((start, comment.end, first_line, end_line));
            }
            Some((_, _, first_line, last_line)) => {
                if last_line > first_line {
                    folds.push(Fold {
                        start_line: first_line,
                        end_line: last_line,
                        kind: FoldKind::Comment,
                    });
                }
                group = Some((comment.start, comment.end, start_line, end_line));
            }
            None => group = Some((comment.start, comment.end, start_line, end_line)),
        }
    }
    if let Some((_, _, first_line, last_line)) = group
        && last_line > first_line
    {
        folds.push(Fold {
            start_line: first_line,
            end_line: last_line,
            kind: FoldKind::Comment,
        });
    }
    folds.sort_by_key(|fold| (fold.start_line, fold.end_line));
    folds
}

pub fn syntax_tokens(text: &str) -> Vec<SyntaxToken> {
    lex(text)
        .into_iter()
        .map(|token| SyntaxToken {
            range: token.start..token.end,
            kind: match token.kind {
                TokenKind::Keyword => Highlight::Keyword,
                TokenKind::Identifier => Highlight::Variable,
                TokenKind::Type => Highlight::Type,
                TokenKind::Number => Highlight::Number,
                TokenKind::String => Highlight::String,
                TokenKind::Comment if text[token.start..token.end].starts_with("/**") => {
                    Highlight::DocComment
                }
                TokenKind::Comment => Highlight::Comment,
                TokenKind::Operator => Highlight::Operator,
                TokenKind::Punctuation => Highlight::Punctuation,
            },
        })
        .collect()
}

fn declaration_body_open(
    tokens: &[Token],
    name_index: usize,
    text: &str,
    is_type: bool,
) -> Option<usize> {
    let next =
        (name_index + 1..tokens.len()).find(|&index| tokens[index].kind != TokenKind::Comment)?;
    if token_text(&tokens[next], text) == "{" {
        return Some(next);
    }
    if is_type && matches!(token_text(&tokens[next], text), "struct" | "interface") {
        return (next + 1..tokens.len())
            .find(|&index| tokens[index].kind != TokenKind::Comment)
            .filter(|&index| token_text(&tokens[index], text) == "{");
    }
    None
}

fn token_text<'a>(token: &Token, text: &'a str) -> &'a str {
    &text[token.start..token.end]
}

fn matching_brace_end(tokens: &[Token], open: usize, text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for token in &tokens[open..] {
        let slice = &text[token.start..token.end];
        if token.kind == TokenKind::Punctuation && slice == "{" {
            depth += 1;
        } else if token.kind == TokenKind::Punctuation && slice == "}" {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(token.end);
            }
        }
    }
    None
}

fn line_end(text: &str, from: usize) -> usize {
    text[from..]
        .find(['\n', '\r'])
        .map_or(text.len(), |offset| from + offset)
}

fn line_number(text: &str, byte: usize) -> u32 {
    text.as_bytes()[..byte.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u32
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

#[cfg(test)]
mod tests {
    use super::{TokenKind, folding, lex, outline};
    use openidea_plugin::syntax::{FoldKind, SymbolKind};

    #[test]
    fn lexes_api_keywords_identifiers_numbers_strings_comments_and_punctuation() {
        let source = "syntax = \"v1\"; type User { id int64 } // note";
        let tokens = lex(source);
        let actual: Vec<(&str, TokenKind)> = tokens
            .iter()
            .map(|token| (&source[token.start..token.end], token.kind))
            .collect();

        assert_eq!(
            actual,
            vec![
                ("syntax", TokenKind::Keyword),
                ("=", TokenKind::Operator),
                ("\"v1\"", TokenKind::String),
                (";", TokenKind::Punctuation),
                ("type", TokenKind::Keyword),
                ("User", TokenKind::Identifier),
                ("{", TokenKind::Punctuation),
                ("id", TokenKind::Identifier),
                ("int64", TokenKind::Type),
                ("}", TokenKind::Punctuation),
                ("// note", TokenKind::Comment),
            ]
        );
    }

    #[test]
    fn token_ranges_are_utf8_byte_offsets_after_unicode() {
        let source = "// 中文\ntype User {}";
        let token = lex(source)
            .into_iter()
            .find(|token| &source[token.start..token.end] == "type")
            .expect("type token");

        assert_eq!(token.start, source.find("type").unwrap());
        assert_eq!(&source[token.start..token.end], "type");
    }

    #[test]
    fn unterminated_strings_and_comments_stop_at_end_of_input() {
        for (source, expected_kind) in [
            ("\"unfinished", TokenKind::String),
            ("// unfinished", TokenKind::Comment),
            ("/* unfinished", TokenKind::Comment),
        ] {
            let tokens = lex(source);
            assert_eq!(tokens.len(), 1, "source: {source}");
            assert_eq!(tokens[0].kind, expected_kind, "source: {source}");
            assert_eq!((tokens[0].start, tokens[0].end), (0, source.len()));
        }
    }

    #[test]
    fn outlines_top_level_types_and_services_with_declaration_ranges() {
        let source = "type User {\n  id int64\n}\nservice users {\n  get /users\n}\n";
        let symbols = outline(source);

        assert_eq!(symbols.len(), 2);
        assert_eq!(
            (symbols[0].name.as_str(), symbols[0].kind),
            ("User", SymbolKind::Struct)
        );
        assert_eq!(
            (symbols[1].name.as_str(), symbols[1].kind),
            ("users", SymbolKind::Interface)
        );
        assert_eq!(&source[symbols[0].name_range.clone()], "User");
        assert_eq!(&source[symbols[1].name_range.clone()], "users");
        assert_eq!(
            &source[symbols[0].range.clone()],
            "type User {\n  id int64\n}"
        );
        assert_eq!(
            &source[symbols[1].range.clone()],
            "service users {\n  get /users\n}"
        );
        assert!(symbols.iter().all(|symbol| symbol.parent.is_none()));
    }

    #[test]
    fn outlines_aliases_without_borrowing_later_braces_and_keeps_hyphenated_service_names() {
        let source = "type UserID int64\ntype User struct {}\nservice user-api {\n get /users\n}\n";
        let symbols = outline(source);

        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0].name, "UserID");
        assert_eq!(symbols[0].kind, SymbolKind::TypeAlias);
        assert_eq!(&source[symbols[0].range.clone()], "type UserID int64");
        assert_eq!(symbols[1].name, "User");
        assert_eq!(symbols[1].kind, SymbolKind::Struct);
        assert_eq!(symbols[2].name, "user-api");
        assert_eq!(&source[symbols[2].name_range.clone()], "user-api");
    }

    #[test]
    fn folds_multiline_braces_and_contiguous_comments_but_ignores_mismatches() {
        let source = "// first\n// second\ntype User {\n  nested {\n  }\n}\n}";
        let folds = folding(source);

        assert!(folds.contains(&openidea_plugin::syntax::Fold {
            start_line: 0,
            end_line: 1,
            kind: FoldKind::Comment,
        }));
        assert!(folds.contains(&openidea_plugin::syntax::Fold {
            start_line: 2,
            end_line: 5,
            kind: FoldKind::Region,
        }));
        assert!(folds.contains(&openidea_plugin::syntax::Fold {
            start_line: 3,
            end_line: 4,
            kind: FoldKind::Region,
        }));
        assert_eq!(
            folds
                .iter()
                .filter(|fold| fold.kind == FoldKind::Region)
                .count(),
            2
        );
    }
}
