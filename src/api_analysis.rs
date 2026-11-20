//! Tolerant `.api` model + lint + definition resolution, shared by the
//! diagnostics and definition providers.
//!
//! Everything here is pure over `&str` (plus an injectable file reader for
//! imports) so it can be unit-tested without a host and can never panic on
//! partial or malformed input.

use crate::api_lexer::{Token, TokenKind, lex};

/// A parsed `syntax = "vN"` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxVersion {
    /// The raw string, e.g. `"v1"` without quotes → `v1`.
    pub raw: String,
    /// Byte span of the string token (including quotes).
    pub span: (usize, usize),
}

/// A named import path with the span of its string literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub path: String,
    pub span: (usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Struct,
    Alias,
    Interface,
}

/// A `type Foo { … }`, `type ( Foo { … } Bar { … } )`, or `type Foo int64`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeDecl {
    pub name: String,
    pub name_span: (usize, usize),
    pub kind: TypeKind,
    /// Whole declaration span (from `type`/name to closing brace or line end).
    pub span: (usize, usize),
}

/// A receive tag or validation tag on a field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// The tag key inside the backtick string, e.g. `json`, `path`, `form`, `header`.
    pub key: String,
    pub span: (usize, usize),
}

/// One struct field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The type that owns this field (a named struct declaration name).
    pub owner: String,
    pub name: String,
    pub name_span: (usize, usize),
    /// The named type this field refers to, if it is not a primitive.
    pub type_name: Option<String>,
    pub type_span: Option<(usize, usize)>,
    pub tags: Vec<Tag>,
}

/// A reference to a named type: field type, route request/response, embed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeRef {
    pub name: String,
    pub span: (usize, usize),
}

/// One route inside a service block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub handler: Option<String>,
    pub handler_span: Option<(usize, usize)>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub req: Option<TypeRef>,
    pub resp: Option<TypeRef>,
}

/// A service block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub name_span: (usize, usize),
    pub routes: Vec<Route>,
}

/// The parsed document.
#[derive(Clone, Debug, Default)]
pub struct ApiModel {
    pub syntax: Option<SyntaxVersion>,
    pub imports: Vec<Import>,
    pub types: Vec<TypeDecl>,
    pub fields: Vec<Field>,
    pub services: Vec<Service>,
    pub type_refs: Vec<TypeRef>,
}

impl ApiModel {
    /// All references (with spans) to a named type, in source order. Used by the
    /// document indexer to emit `references` anchors for reverse lookup and by
    /// the definition provider for multi-target resolution.
    pub fn references_to(&self, name: &str) -> Vec<&TypeRef> {
        self.type_refs.iter().filter(|r| r.name == name).collect()
    }
}

/// Go primitive type names that never need resolution.
pub const GO_PRIMITIVES: &[&str] = &[
    "string",
    "bool",
    "byte",
    "rune",
    "error",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "float32",
    "float64",
    "complex64",
    "complex128",
    "uintptr",
    "any",
];

pub fn is_primitive(name: &str) -> bool {
    GO_PRIMITIVES.contains(&name)
}

/// Extract the declared syntax version from a raw `"vN"` string token.
/// Returns the inner `vN` only when it matches `v[1-9][0-9]*`.
pub fn parse_version(raw_with_quotes: &str) -> Option<String> {
    let trimmed = raw_with_quotes.trim_matches(|c| c == '"' || c == '`');
    let bytes = trimmed.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'v' || !bytes[1].is_ascii_digit() || bytes[1] == b'0' {
        return None;
    }
    if !bytes[2..].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.to_owned())
}

fn text_of<'a>(text: &'a str, token: &Token) -> &'a str {
    &text[token.start..token.end]
}

fn is_punct(text: &str, token: &Token, ch: &str) -> bool {
    token.kind == TokenKind::Punctuation && text_of(text, token) == ch
}

/// Parse a `.api` document into a tolerant model. Never panics.
pub fn parse(text: &str) -> ApiModel {
    let tokens = lex(text);
    let mut model = ApiModel::default();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        let word = text_of(text, token);
        match word {
            "syntax" => {
                i = parse_syntax(text, &tokens, i, &mut model);
                continue;
            }
            "import" => {
                i = parse_import(text, &tokens, i, &mut model);
                continue;
            }
            "type" => {
                i = parse_type(text, &tokens, i, &mut model);
                continue;
            }
            "service" => {
                i = parse_service(text, &tokens, i, &mut model);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    model
}

fn next_non_comment(tokens: &[Token], from: usize) -> Option<usize> {
    (from..tokens.len()).find(|&k| tokens[k].kind != TokenKind::Comment)
}

fn parse_syntax(text: &str, tokens: &[Token], at: usize, model: &mut ApiModel) -> usize {
    // syntax = "vN"
    let Some(eq) = next_non_comment(tokens, at + 1) else {
        return at + 1;
    };
    if text_of(text, &tokens[eq]) != "=" {
        return at + 1;
    }
    let Some(val) = next_non_comment(tokens, eq + 1) else {
        return eq + 1;
    };
    if tokens[val].kind == TokenKind::String {
        let raw = text_of(text, &tokens[val]);
        model.syntax = Some(SyntaxVersion {
            raw: raw.trim_matches(|c| c == '"' || c == '`').to_owned(),
            span: (tokens[val].start, tokens[val].end),
        });
    }
    val + 1
}

fn parse_import(text: &str, tokens: &[Token], at: usize, model: &mut ApiModel) -> usize {
    // import "path"   (optionally a group `import ( "a" "b" )`)
    let Some(next) = next_non_comment(tokens, at + 1) else {
        return at + 1;
    };
    if tokens[next].kind == TokenKind::String {
        let raw = text_of(text, &tokens[next]);
        model.imports.push(Import {
            path: raw.trim_matches(|c| c == '"' || c == '`').to_owned(),
            span: (tokens[next].start, tokens[next].end),
        });
        return next + 1;
    }
    if is_punct(text, &tokens[next], "(") {
        let mut k = next + 1;
        while k < tokens.len() && !is_punct(text, &tokens[k], ")") {
            if tokens[k].kind == TokenKind::String {
                let raw = text_of(text, &tokens[k]);
                model.imports.push(Import {
                    path: raw.trim_matches(|c| c == '"' || c == '`').to_owned(),
                    span: (tokens[k].start, tokens[k].end),
                });
            }
            k += 1;
        }
        return k + 1;
    }
    at + 1
}

/// Find a top-level `{` that immediately follows a declaration (after skipping
/// an optional `struct`/`interface` keyword). Returns the token index of `{`.
fn declaration_body_open(text: &str, tokens: &[Token], name_index: usize) -> Option<usize> {
    let next = next_non_comment(tokens, name_index + 1)?;
    if is_punct(text, &tokens[next], "{") {
        return Some(next);
    }
    if matches!(text_of(text, &tokens[next]), "struct" | "interface") {
        let brace = next_non_comment(tokens, next + 1)?;
        if is_punct(text, &tokens[brace], "{") {
            return Some(brace);
        }
    }
    None
}

fn matching_close(tokens: &[Token], text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (idx, token) in tokens.iter().enumerate().skip(open) {
        if is_punct(text, token, "{") {
            depth += 1;
        } else if is_punct(text, token, "}") {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(idx);
            }
        }
    }
    None
}

fn parse_type(text: &str, tokens: &[Token], at: usize, model: &mut ApiModel) -> usize {
    let Some(next) = next_non_comment(tokens, at + 1) else {
        return at + 1;
    };
    // type ( A { … } B { … } )
    if is_punct(text, &tokens[next], "(") {
        let mut k = next + 1;
        while k < tokens.len() && !is_punct(text, &tokens[k], ")") {
            if tokens[k].kind == TokenKind::Comment {
                k += 1;
                continue;
            }
            if tokens[k].kind == TokenKind::Identifier {
                k = parse_single_type(text, tokens, k, model, Some(at));
            } else {
                k += 1;
            }
        }
        return k + 1;
    }
    parse_single_type(text, tokens, next, model, None)
}

/// Parse one type declaration starting at the name token index `name_index`.
fn parse_single_type(
    text: &str,
    tokens: &[Token],
    name_index: usize,
    model: &mut ApiModel,
    group_anchor: Option<usize>,
) -> usize {
    if tokens[name_index].kind != TokenKind::Identifier {
        return name_index + 1;
    }
    let name = text_of(text, &tokens[name_index]).to_owned();
    let name_span = (tokens[name_index].start, tokens[name_index].end);
    let decl_start = group_anchor.map_or(name_span.0, |_| name_span.0);

    if let Some(open) = declaration_body_open(text, tokens, name_index) {
        let close = matching_close(tokens, text, open);
        let is_iface = next_non_comment(tokens, name_index + 1)
            .is_some_and(|n| text_of(text, &tokens[n]) == "interface");
        let kind = if is_iface {
            TypeKind::Interface
        } else {
            TypeKind::Struct
        };
        let span_end = close.map_or(tokens[open].end, |c| tokens[c].end);
        model.types.push(TypeDecl {
            name: name.clone(),
            name_span,
            kind,
            span: (decl_start, span_end),
        });
        if kind == TypeKind::Struct {
            parse_fields(text, tokens, open, close, &name, model);
        }
        close.map_or(open + 1, |c| c + 1)
    } else {
        // Alias: type Foo int64  — ends at end of line.
        let end = line_end(text, name_span.1);
        model.types.push(TypeDecl {
            name,
            name_span,
            kind: TypeKind::Alias,
            span: (decl_start, end),
        });
        // The aliased type might itself be a named reference.
        let mut k = name_index + 1;
        while k < tokens.len() && tokens[k].start < end {
            if tokens[k].kind == TokenKind::Identifier {
                model.type_refs.push(TypeRef {
                    name: text_of(text, &tokens[k]).to_owned(),
                    span: (tokens[k].start, tokens[k].end),
                });
                break;
            }
            k += 1;
        }
        k
    }
}

fn parse_fields(
    text: &str,
    tokens: &[Token],
    open: usize,
    close: Option<usize>,
    owner: &str,
    model: &mut ApiModel,
) {
    let end = close.unwrap_or(tokens.len());
    let mut k = open + 1;
    while k < end {
        let token = &tokens[k];
        if token.kind == TokenKind::Comment {
            k += 1;
            continue;
        }
        // Nested anonymous struct: `Name struct { … }` or `struct { … }` — recurse.
        if is_punct(text, token, "{") {
            let nested_close = matching_close(tokens, text, k);
            parse_fields(text, tokens, k, nested_close, owner, model);
            k = nested_close.map_or(k + 1, |c| c + 1);
            continue;
        }
        if is_punct(text, token, "}") {
            k += 1;
            continue;
        }
        // A field: NAME TYPE [`tags`]  or embedded TYPE.
        if token.kind == TokenKind::Identifier {
            // Field name followed by a type token.
            if let Some(type_idx) = next_non_comment(tokens, k + 1)
                && type_idx < end
                && !is_punct(text, &tokens[type_idx], "}")
            {
                let name = text_of(text, token).to_owned();
                let name_span = (token.start, token.end);
                let mut type_name = None;
                let mut type_span = None;
                let t = &tokens[type_idx];
                if t.kind == TokenKind::Identifier {
                    let tn = text_of(text, t).to_owned();
                    if !is_primitive(&tn) {
                        type_name = Some(tn.clone());
                        type_span = Some((t.start, t.end));
                        model.type_refs.push(TypeRef {
                            name: tn,
                            span: (t.start, t.end),
                        });
                    }
                } else if t.kind == TokenKind::Type {
                    // primitive; still record the field
                } else if text_of(text, t) == "[" || text_of(text, t) == "*" {
                    // slice/array/pointer: find the inner named type
                    if let Some((inner, inner_span)) =
                        find_named_in_type(text, tokens, type_idx, end)
                    {
                        type_name = Some(inner.clone());
                        type_span = Some(inner_span);
                        model.type_refs.push(TypeRef {
                            name: inner,
                            span: inner_span,
                        });
                    }
                }
                let tags = collect_tags(text, tokens, k + 1, end);
                model.fields.push(Field {
                    owner: owner.to_owned(),
                    name,
                    name_span,
                    type_name,
                    type_span,
                    tags,
                });
                // advance past this field's tokens up to a tag or line/newline
                k = skip_field(text, tokens, k + 1, end);
                continue;
            }
        }
        k += 1;
    }
}

/// For `[]Foo` / `*Foo` / `[N]Foo` / `[]*Foo`, return `Foo` and its span when it
/// is a named (non-primitive) type. The span covers the inner identifier so a
/// caret on `Foo` (not the `[`/`*`) resolves the definition.
fn find_named_in_type(
    text: &str,
    tokens: &[Token],
    start: usize,
    end: usize,
) -> Option<(String, (usize, usize))> {
    let mut k = start;
    while k < end {
        let t = &tokens[k];
        if t.kind == TokenKind::Identifier {
            let name = text_of(text, t);
            if !is_primitive(name) {
                return Some((name.to_owned(), (t.start, t.end)));
            }
            return None;
        }
        if t.kind == TokenKind::Type {
            return None;
        }
        if is_punct(text, t, ",") || is_punct(text, t, "}") || t.kind == TokenKind::String {
            return None;
        }
        k += 1;
    }
    None
}

fn collect_tags(text: &str, tokens: &[Token], from: usize, end: usize) -> Vec<Tag> {
    let mut tags = Vec::new();
    let mut k = from;
    while k < end {
        let t = &tokens[k];
        if t.kind == TokenKind::String {
            // A backtick tag string like `json:"name" path:"id"`.
            let raw = text_of(text, t);
            for key in ["json", "path", "form", "header"] {
                if raw.contains(&format!("{key}:\"")) {
                    tags.push(Tag {
                        key: key.to_owned(),
                        span: (t.start, t.end),
                    });
                }
            }
            // A second string token on the same line is another tag string.
            k += 1;
            continue;
        }
        if is_punct(text, t, "}") || t.kind == TokenKind::Comment {
            break;
        }
        k += 1;
    }
    tags
}

fn skip_field(text: &str, tokens: &[Token], from: usize, end: usize) -> usize {
    let mut k = from;
    while k < end {
        let t = &tokens[k];
        if is_punct(text, t, "}") {
            return k;
        }
        // End of field: a string tag then next identifier.
        k += 1;
        if k < end
            && tokens[k].kind == TokenKind::Identifier
            && tokens[k].start > t.end
            && text[t.end..tokens[k].start].contains('\n')
        {
            return k;
        }
    }
    k
}

fn parse_service(text: &str, tokens: &[Token], at: usize, model: &mut ApiModel) -> usize {
    let Some(name_idx) = next_non_comment(tokens, at + 1) else {
        return at + 1;
    };
    if tokens[name_idx].kind != TokenKind::Identifier {
        return at + 1;
    }
    let mut name = text_of(text, &tokens[name_idx]).to_owned();
    let name_start = tokens[name_idx].start;
    let mut name_end = tokens[name_idx].end;
    // Hyphenated service names: user-api
    let mut k = name_idx + 1;
    while k + 1 < tokens.len()
        && text_of(text, &tokens[k]) == "-"
        && tokens[k + 1].kind == TokenKind::Identifier
        && tokens[k - 1].end == tokens[k].start
    {
        name.push('-');
        name.push_str(text_of(text, &tokens[k + 1]));
        name_end = tokens[k + 1].end;
        k += 2;
    }
    let name_span = (name_start, name_end);
    let Some(open) = next_non_comment(tokens, k).filter(|&o| is_punct(text, &tokens[o], "{"))
    else {
        model.services.push(Service {
            name,
            name_span,
            routes: Vec::new(),
        });
        return k;
    };
    let close = matching_close(tokens, text, open);
    let routes = parse_routes(text, tokens, open, close.unwrap_or(tokens.len()));
    // Collect request/response type references into the model so definition
    // resolution and unresolved-type lint can see them.
    for route in &routes {
        if let Some(r) = &route.req {
            model.type_refs.push(r.clone());
        }
        if let Some(r) = &route.resp {
            model.type_refs.push(r.clone());
        }
    }
    model.services.push(Service {
        name,
        name_span,
        routes,
    });
    close.map_or(open + 1, |c| c + 1)
}

fn parse_routes(text: &str, tokens: &[Token], open: usize, end: usize) -> Vec<Route> {
    let mut routes: Vec<Route> = Vec::new();
    let mut current: Option<Route> = None;
    let mut k = open + 1;
    let push = |routes: &mut Vec<Route>, current: &mut Option<Route>| {
        if let Some(route) = current.take() {
            routes.push(route);
        }
    };
    while k < end {
        let t = &tokens[k];
        match t.kind {
            TokenKind::Keyword => {
                let kw = text_of(text, t);
                if kw == "@handler" {
                    if current.is_some() {
                        push(&mut routes, &mut current);
                    }
                    let r = current.get_or_insert_with(empty_route);
                    if let Some(name) = next_non_comment(tokens, k + 1)
                        && tokens[name].kind == TokenKind::Identifier
                    {
                        r.handler = Some(text_of(text, &tokens[name]).to_owned());
                        r.handler_span = Some((tokens[name].start, tokens[name].end));
                    }
                } else if matches!(
                    kw,
                    "get"
                        | "post"
                        | "put"
                        | "patch"
                        | "delete"
                        | "head"
                        | "options"
                        | "trace"
                        | "connect"
                ) {
                    let r = current.get_or_insert_with(empty_route);
                    r.method = Some(kw.to_owned());
                    // Path literal spans from the token right after the method up
                    // to (but not including) the first `(`.  Tokens may be
                    // split (`/`, `user`, `/`, `login`) so join the raw slice.
                    let path_start = next_non_comment(tokens, k + 1);
                    if let Some(p) = path_start {
                        let paren = (p..end).find(|&j| is_punct(text, &tokens[j], "("));
                        let path_end_byte = paren.map_or(tokens[p].end, |j| tokens[j].start);
                        let raw = &text[tokens[p].start..path_end_byte];
                        r.path = Some(raw.trim().trim_matches('"').to_owned());
                        let after = paren.unwrap_or(p + 1);
                        k = parse_route_types(text, tokens, after, end, r);
                        continue;
                    }
                }
            }
            _ => {}
        }
        k += 1;
    }
    push(&mut routes, &mut current);
    routes
}

fn empty_route() -> Route {
    Route {
        handler: None,
        handler_span: None,
        method: None,
        path: None,
        req: None,
        resp: None,
    }
}

/// `p` points at the `(` that begins the request type (or the token after path).
fn parse_route_types(
    text: &str,
    tokens: &[Token],
    p: usize,
    end: usize,
    route: &mut Route,
) -> usize {
    let mut k = p;
    // (Req)
    while k < end && tokens[k].kind == TokenKind::Comment {
        k += 1;
    }
    if k < end && is_punct(text, &tokens[k], "(") {
        let (next, req) = read_paren_type(text, tokens, k, end);
        route.req = req;
        k = next;
    }
    // returns
    if k < end && tokens[k].kind == TokenKind::Keyword && text_of(text, &tokens[k]) == "returns" {
        k += 1;
        while k < end && tokens[k].kind == TokenKind::Comment {
            k += 1;
        }
        if k < end && is_punct(text, &tokens[k], "(") {
            let (next, resp) = read_paren_type(text, tokens, k, end);
            route.resp = resp;
            k = next;
        }
    }
    k
}

fn read_paren_type(
    text: &str,
    tokens: &[Token],
    open: usize,
    end: usize,
) -> (usize, Option<TypeRef>) {
    let mut k = open + 1;
    let mut found = None;
    while k < end && !is_punct(text, &tokens[k], ")") {
        if tokens[k].kind == TokenKind::Identifier && found.is_none() {
            let name = text_of(text, &tokens[k]);
            if !is_primitive(name) {
                found = Some(TypeRef {
                    name: name.to_owned(),
                    span: (tokens[k].start, tokens[k].end),
                });
            }
        }
        k += 1;
    }
    (k + 1, found)
}

fn line_end(text: &str, from: usize) -> usize {
    text[from..]
        .find(['\n', '\r'])
        .map_or(text.len(), |offset| from + offset)
}

// ---------------------------------------------------------------------------
// Lint
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiIssue {
    pub span: (usize, usize),
    pub severity: IssueSeverity,
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueSeverity {
    Error,
    Warning,
    Hint,
}

/// Read the text of an imported file. Injectable so lint is testable and the
/// provider can supply project/fs reads.
pub trait ImportReader {
    fn read(&self, path: &str) -> Option<String>;
}

/// A reader that returns nothing (single-file lint, e.g. when no project).
pub struct NoImports;
impl ImportReader for NoImports {
    fn read(&self, _path: &str) -> Option<String> {
        None
    }
}

/// Lint a single file. `goctl_version` is `Some((major, minor))` when known.
/// Cross-file rules use `reader` to load imports; cycles are followed once.
pub fn lint(
    path: &str,
    text: &str,
    goctl_version: Option<(u32, u32)>,
    reader: &dyn ImportReader,
) -> Vec<ApiIssue> {
    let model = parse(text);
    let mut issues = lint_model(&model, goctl_version);

    // Cross-file: gather transitively imported models (best-effort).
    let mut seen = std::collections::HashSet::new();
    seen.insert(normalize(path));
    let mut stack: Vec<(String, ApiModel)> = Vec::new();
    collect_imports(path, &model, reader, &mut seen, &mut stack, &mut issues);

    // Duplicate type declarations across files (main + imports).
    let mut all_types: Vec<(String, String)> = Vec::new(); // (name, file)
    for t in &model.types {
        all_types.push((t.name.clone(), normalize(path)));
    }
    for (file, m) in &stack {
        for t in &m.types {
            all_types.push((t.name.clone(), file.clone()));
        }
    }
    let mut dup_reported = std::collections::HashSet::new();
    for t in &model.types {
        let count = all_types.iter().filter(|(n, _)| n == &t.name).count();
        if count > 1 && dup_reported.insert(t.name.clone()) {
            issues.push(ApiIssue {
                span: t.name_span,
                severity: IssueSeverity::Error,
                code: "api.type.duplicate",
                message: format!("Duplicate type declaration: '{}'", t.name),
            });
        }
    }

    // Multiple distinct service names across files.
    let mut names = std::collections::BTreeSet::new();
    for s in &model.services {
        names.insert(s.name.clone());
    }
    for (_, m) in &stack {
        for s in &m.services {
            names.insert(s.name.clone());
        }
    }
    if names.len() > 1 {
        let canonical = names.iter().next().cloned().unwrap_or_default();
        for s in &model.services {
            if s.name != canonical {
                issues.push(ApiIssue {
                    span: s.name_span,
                    severity: IssueSeverity::Error,
                    code: "api.service.multiple",
                    message: format!(
                        "Service name '{}' conflicts with '{}'. Only one service is allowed.",
                        s.name, canonical
                    ),
                });
            }
        }
    }

    // Unresolved named types: a ref not primitive and declared in no reachable file.
    let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();
    for t in &model.types {
        known.insert(t.name.clone());
    }
    for (_, m) in &stack {
        for t in &m.types {
            known.insert(t.name.clone());
        }
    }
    let mut reported_refs = std::collections::HashSet::new();
    for r in &model.type_refs {
        if is_primitive(&r.name) || known.contains(&r.name) {
            continue;
        }
        if reported_refs.insert((r.name.clone(), r.span.0)) {
            issues.push(ApiIssue {
                span: r.span,
                severity: IssueSeverity::Error,
                code: "api.type.unresolved",
                message: format!("Unresolved type: '{}'", r.name),
            });
        }
    }

    issues
}

fn lint_model(model: &ApiModel, goctl_version: Option<(u32, u32)>) -> Vec<ApiIssue> {
    let mut issues = Vec::new();

    // syntax
    match &model.syntax {
        Some(v) => {
            if parse_version(&format!("\"{}\"", v.raw)).is_none() {
                issues.push(ApiIssue {
                    span: v.span,
                    severity: IssueSeverity::Error,
                    code: "api.syntax.invalid",
                    message: format!(
                        "Invalid syntax version '{}': must match v[1-9][0-9]*",
                        v.raw
                    ),
                });
            }
        }
        None => {
            // From goctl 1.6 the syntax header is required.
            if goctl_version.is_some_and(|(major, minor)| major > 1 || (major == 1 && minor >= 6)) {
                issues.push(ApiIssue {
                    span: (0, 0),
                    severity: IssueSeverity::Warning,
                    code: "api.syntax.missing",
                    message: "Missing `syntax = \"v1\"` header (required by goctl 1.6+)".to_owned(),
                });
            }
        }
    }

    // duplicate fields within an owner
    let mut field_seen: std::collections::HashMap<(String, String), (usize, usize)> =
        std::collections::HashMap::new();
    for f in &model.fields {
        let key = (f.owner.clone(), f.name.clone());
        if field_seen.contains_key(&key) {
            issues.push(ApiIssue {
                span: f.name_span,
                severity: IssueSeverity::Error,
                code: "api.field.duplicate",
                message: format!("Duplicate field '{}' in type '{}'", f.name, f.owner),
            });
        } else {
            field_seen.insert(key, f.name_span);
        }
    }

    // duplicate handlers / routes
    let mut handlers = std::collections::HashSet::new();
    let mut route_keys = std::collections::HashSet::new();
    for s in &model.services {
        for r in &s.routes {
            if let (Some(h), Some(span)) = (&r.handler, r.handler_span)
                && !handlers.insert(h.clone())
            {
                issues.push(ApiIssue {
                    span,
                    severity: IssueSeverity::Error,
                    code: "api.handler.duplicate",
                    message: format!("Duplicate handler '{}'", h),
                });
            }
            if let (Some(m), Some(p)) = (&r.method, &r.path) {
                let key = format!("{} {}", m.to_uppercase(), p);
                if !route_keys.insert(key.clone()) {
                    if let Some(span) = r.handler_span.or(r.req.as_ref().map(|v| v.span)) {
                        issues.push(ApiIssue {
                            span,
                            severity: IssueSeverity::Error,
                            code: "api.route.duplicate",
                            message: format!("Duplicate route '{}'", key),
                        });
                    }
                }
            }
        }
    }

    issues
}

fn collect_imports(
    current_path: &str,
    model: &ApiModel,
    reader: &dyn ImportReader,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<(String, ApiModel)>,
    issues: &mut Vec<ApiIssue>,
) {
    let dir = parent_dir(current_path);
    for import in &model.imports {
        let resolved = if import.path.starts_with('/') {
            import.path.clone()
        } else {
            join_path(&dir, &import.path)
        };
        let normalized = normalize(&resolved);
        if seen.contains(&normalized) {
            // Possible cycle back to an already-visited file: report if it is
            // the current file or an ancestor we are revisiting mid-DFS.
            if normalized == normalize(current_path) {
                issues.push(ApiIssue {
                    span: import.span,
                    severity: IssueSeverity::Error,
                    code: "api.import.cycle",
                    message: format!("Import cycle: '{}' imports itself", import.path),
                });
            }
            continue;
        }
        let Some(text) = reader.read(&resolved) else {
            continue;
        };
        seen.insert(normalized.clone());
        let imported = parse(&text);
        // syntax mismatch check
        if let (Some(main), Some(sub)) = (&model.syntax, &imported.syntax)
            && main.raw != sub.raw
        {
            issues.push(ApiIssue {
                span: import.span,
                severity: IssueSeverity::Error,
                code: "api.syntax.import_mismatch",
                message: format!(
                    "Imported file syntax '{}' differs from '{}'",
                    sub.raw, main.raw
                ),
            });
        }
        collect_imports(&resolved, &imported, reader, seen, out, issues);
        out.push((normalized, imported));
    }
}

// ---------------------------------------------------------------------------
// Definition resolution
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefTarget {
    pub path: String,
    pub span: (usize, usize),
}

/// Resolve the type reference containing byte `at` to its declaration.
/// Searches the file, then transitively through readable imports.
pub fn find_definition(
    path: &str,
    text: &str,
    at: usize,
    reader: &dyn ImportReader,
) -> Vec<DefTarget> {
    let model = parse(text);
    let Some(name) = type_ref_at(&model, at) else {
        return Vec::new();
    };
    if is_primitive(&name) {
        return Vec::new();
    }
    let mut out = Vec::new();
    // Current file first.
    for t in &model.types {
        if t.name == name {
            out.push(DefTarget {
                path: path.to_owned(),
                span: t.name_span,
            });
        }
    }
    // Imports (breadth-first).
    let mut seen = std::collections::HashSet::new();
    seen.insert(normalize(path));
    let mut queue = vec![(path.to_owned(), model)];
    let mut guard = 0;
    while let Some((file, m)) = queue.pop() {
        guard += 1;
        if guard > 256 {
            break;
        }
        let dir = parent_dir(&file);
        for import in &m.imports {
            let resolved = if import.path.starts_with('/') {
                import.path.clone()
            } else {
                join_path(&dir, &import.path)
            };
            let n = normalize(&resolved);
            if !seen.insert(n.clone()) {
                continue;
            }
            let Some(t) = reader.read(&resolved) else {
                continue;
            };
            let im = parse(&t);
            for decl in &im.types {
                if decl.name == name {
                    out.push(DefTarget {
                        path: resolved.clone(),
                        span: decl.name_span,
                    });
                }
            }
            queue.push((resolved, im));
        }
    }
    out
}

/// The named type reference whose span contains byte `at`, if any.
fn type_ref_at(model: &ApiModel, at: usize) -> Option<String> {
    for r in &model.type_refs {
        if at >= r.span.0 && at <= r.span.1 {
            return Some(r.name.clone());
        }
    }
    // Fall back: the field type span.
    for f in &model.fields {
        if let (Some(name), Some(span)) = (&f.type_name, f.type_span)
            && at >= span.0
            && at <= span.1
        {
            return Some(name.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let mut out = String::new();
    if path.starts_with('/') {
        out.push('/');
    }
    out.push_str(&parts.join("/"));
    out
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(0) => "/".to_owned(),
        Some(i) => path[..i].to_owned(),
        None => ".".to_owned(),
    }
}

fn join_path(dir: &str, rel: &str) -> String {
    if dir == "/" {
        format!("/{rel}")
    } else if dir.is_empty() || dir == "." {
        rel.to_owned()
    } else {
        format!("{dir}/{rel}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Files(HashMap<String, String>);
    impl ImportReader for Files {
        fn read(&self, path: &str) -> Option<String> {
            self.0.get(path).cloned()
        }
    }

    fn no_files() -> NoImports {
        NoImports
    }

    #[test]
    fn parse_version_accepts_v1_and_rejects_v0_and_garbage() {
        assert_eq!(parse_version("\"v1\""), Some("v1".to_owned()));
        assert_eq!(parse_version("\"v23\""), Some("v23".to_owned()));
        assert_eq!(parse_version("\"v0\""), None);
        assert_eq!(parse_version("\"1\""), None);
        assert_eq!(parse_version("\"v1x\""), None);
    }

    #[test]
    fn parses_types_fields_services_and_routes() {
        let src = r#"
syntax = "v1"
type (
  LoginReq {
    Username string `json:"username"`
    Password string `json:"password"`
  }
  LoginResp {
    Token string `json:"token"`
  }
)
service user-api {
  @handler Login
  post /user/login (LoginReq) returns (LoginResp)
}
"#;
        let m = parse(src);
        assert_eq!(m.syntax.as_ref().unwrap().raw, "v1");
        let names: Vec<_> = m.types.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"LoginReq"));
        assert!(names.contains(&"LoginResp"));
        assert_eq!(m.services.len(), 1);
        assert_eq!(m.services[0].name, "user-api");
        let route = &m.services[0].routes[0];
        assert_eq!(route.handler.as_deref(), Some("Login"));
        assert_eq!(route.method.as_deref(), Some("post"));
        assert_eq!(route.path.as_deref(), Some("/user/login"));
        assert_eq!(route.req.as_ref().unwrap().name, "LoginReq");
        assert_eq!(route.resp.as_ref().unwrap().name, "LoginResp");
        // both req and resp are in type_refs
        assert!(m.type_refs.iter().any(|r| r.name == "LoginReq"));
        assert!(m.type_refs.iter().any(|r| r.name == "LoginResp"));
        // fields parsed
        assert!(
            m.fields
                .iter()
                .any(|f| f.name == "Username" && f.owner == "LoginReq")
        );
    }

    #[test]
    fn malformed_input_never_panics_and_parses_prefix() {
        for src in [
            "type User {",
            "type User { id int64",
            "service {",
            "@handler",
            "syntax = ",
            "type ( Foo {",
            "",
            "}{}[](",
        ] {
            let _ = parse(src);
        }
    }

    #[test]
    fn lint_flags_duplicate_type_missing_syntax_unresolved_and_tag_rules() {
        let src = r#"
type User {
  id int64 `json:"id" path:"id"`
  id int64 `json:"again"`
  mate UnknownType
}
type User {
  name string
}
"#;
        let issues = lint("/p/a.api", src, Some((1, 6)), &no_files());
        let codes: Vec<_> = issues.iter().map(|i| i.code).collect();
        assert!(codes.contains(&"api.field.duplicate"), "{codes:?}");
        assert!(codes.contains(&"api.type.duplicate"), "{codes:?}");
        assert!(codes.contains(&"api.type.unresolved"), "{codes:?}");
        assert!(codes.contains(&"api.syntax.missing"), "{codes:?}");
    }

    #[test]
    fn lint_clean_file_reports_nothing() {
        let src = "syntax = \"v1\"\ntype User { id int64 `json:\"id\"` }\nservice u-api { @handler G get /u (User) returns (User) }\n";
        let issues = lint("/p/a.api", src, Some((1, 6)), &no_files());
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn lint_gates_syntax_required_on_goctl_version() {
        let src = "type User { id int64 }\n";
        let old = lint("/p/a.api", src, Some((1, 5)), &no_files());
        assert!(!old.iter().any(|i| i.code == "api.syntax.missing"));
        let new = lint("/p/a.api", src, Some((1, 6)), &no_files());
        assert!(new.iter().any(|i| i.code == "api.syntax.missing"));
        let unknown = lint("/p/a.api", src, None, &no_files());
        assert!(!unknown.iter().any(|i| i.code == "api.syntax.missing"));
    }

    #[test]
    fn lint_reports_multiple_service_names_and_duplicate_handler_route() {
        let src = r#"
syntax = "v1"
service a-api {
  @handler H1
  get /x (Req) returns (Resp)
  @handler H1
  get /x (Req) returns (Resp)
}
service b-api {
  @handler H2
  get /y (Req) returns (Resp)
}
type Req {}
type Resp {}
"#;
        let issues = lint("/p/a.api", src, Some((1, 6)), &no_files());
        let codes: Vec<_> = issues.iter().map(|i| i.code).collect();
        assert!(codes.contains(&"api.handler.duplicate"), "{codes:?}");
        assert!(codes.contains(&"api.route.duplicate"), "{codes:?}");
        assert!(codes.contains(&"api.service.multiple"), "{codes:?}");
    }

    #[test]
    fn lint_follows_imports_for_duplicate_and_unresolved() {
        let main = "syntax = \"v1\"\nimport \"shared.api\"\ntype User { id int64 }\nservice u-api { @handler G get /u (User) returns (Shared) }\n";
        let mut files = HashMap::new();
        files.insert(
            "/p/shared.api".to_owned(),
            "syntax = \"v1\"\ntype User { x int64 }\ntype Shared { y int64 }\n".to_owned(),
        );
        let reader = Files(files);
        let issues = lint("/p/main.api", main, Some((1, 6)), &reader);
        let codes: Vec<_> = issues.iter().map(|i| i.code).collect();
        assert!(codes.contains(&"api.type.duplicate"), "{codes:?}");
        // Shared is defined in the import, so not unresolved
        assert!(!codes.contains(&"api.type.unresolved"), "{codes:?}");
    }

    #[test]
    fn lint_flags_syntax_import_mismatch() {
        let main = "syntax = \"v2\"\nimport \"other.api\"\ntype U {}\n";
        let mut files = HashMap::new();
        files.insert(
            "/p/other.api".to_owned(),
            "syntax = \"v1\"\ntype X {}\n".to_owned(),
        );
        let reader = Files(files);
        let issues = lint("/p/main.api", main, Some((1, 6)), &reader);
        assert!(
            issues
                .iter()
                .any(|i| i.code == "api.syntax.import_mismatch")
        );
    }

    #[test]
    fn definition_resolves_local_and_imported_and_skips_primitives() {
        let src = "syntax = \"v1\"\ntype User { id int64 }\nservice u-api { @handler G get /u (User) returns (User) }\n";
        let m = parse(src);
        // caret inside the request (User): the first "User " occurrence after get
        let user_use = src.match_indices("User").nth(1).unwrap().0; // second occurrence (route)
        let targets = find_definition("/p/a.api", src, user_use + 1, &no_files());
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, "/p/a.api");
        assert_eq!(&src[targets[0].span.0..targets[0].span.1], "User");
        let _ = m;

        // primitive -> none
        let prim = "syntax = \"v1\"\ntype U { id int64 }\n";
        let id_pos = prim.find("int64").unwrap();
        assert!(find_definition("/p/a.api", prim, id_pos + 1, &no_files()).is_empty());
    }

    #[test]
    fn definition_crosses_import() {
        let main = "syntax = \"v1\"\nimport \"shared.api\"\nservice u-api { @handler G get /u (Shared) returns (Shared) }\n";
        let mut files = HashMap::new();
        files.insert(
            "/p/shared.api".to_owned(),
            "syntax = \"v1\"\ntype Shared { id int64 }\n".to_owned(),
        );
        let reader = Files(files);
        let at = main.find("(Shared)").unwrap() + 2;
        let targets = find_definition("/p/main.api", main, at, &reader);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, "/p/shared.api");
    }

    // --- Regression: issue #2 (false "multiple receive tags") ---------------

    #[test]
    fn lint_allows_multiple_receive_tags_on_one_field() {
        // go-zero allows several of json/path/form/header on one field; only a
        // repeated *same* key type would be odd. The old rule flagged this.
        let src = "syntax = \"v1\"\ntype User {\n  AgentID int64 `json:\"agentId\" path:\"agentId\"`\n  Name string `json:\"name\" form:\"name\"`\n}\n";
        let issues = lint("/p/a.api", src, Some((1, 6)), &no_files());
        assert!(
            !issues.iter().any(|i| i.code == "api.tag.multiple"),
            "unexpected tag.multiple: {issues:?}"
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    // --- Regression: issue #3 (slice / pointer / array element jump) --------

    #[test]
    fn definition_resolves_slice_pointer_and_array_element_types() {
        let src = "syntax = \"v1\"\ntype Item {\n  id int64\n}\ntype Box {\n  Items []Item\n  Ptr *Item\n  Fixed [3]Item\n}\n";
        let model = parse(src);
        // The slice element TypeRef span must cover the inner identifier "Item",
        // not the leading "[".
        let slice_ref = model
            .type_refs
            .iter()
            .find(|r| r.name == "Item" && &src[r.span.0..r.span.1] == "Item");
        assert!(
            slice_ref.is_some(),
            "slice element TypeRef should point at 'Item', refs={:?}",
            model.type_refs
        );

        for needle in ["[]Item", "*Item", "[3]Item"] {
            let at = src.find(needle).unwrap() + needle.len() - 2;
            let targets = find_definition("/p/a.api", src, at, &no_files());
            assert_eq!(targets.len(), 1, "no jump for {needle}");
            assert_eq!(&src[targets[0].span.0..targets[0].span.1], "Item");
        }
    }

    // --- Feature: reverse lookup (references) + multi-target ----------------

    #[test]
    fn model_collects_references_for_reverse_lookup() {
        let src = "syntax = \"v1\"\ntype Req { id int64 }\nservice u-api {\n  @handler A get /a (Req) returns (Req)\n  @handler B get /b (Req)\n}\n";
        let model = parse(src);
        // Two route reqs + one response reference the name Req.
        let refs = model.references_to("Req");
        assert_eq!(refs.len(), 3, "refs={refs:?}");
    }
}
