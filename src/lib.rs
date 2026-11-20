pub mod api_analysis;
pub mod api_lexer;
#[cfg(test)]
mod tpl_assets;
pub mod tpl_lexer;

use openidea_plugin::diagnostics::DiagnosticsProvider;
use openidea_plugin::host;
use openidea_plugin::indexing::{Anchor, Document as IndexDocument, DocumentIndexer, Edges};
use openidea_plugin::providers::{Document, LineMarker, Providers};
use openidea_plugin::syntax::{Fold, Symbol, SyntaxProvider, Token};
use openidea_plugin::{
    Diagnostic, Error, Event, Invocation, Location, NotificationLevel, Plugin, Position, Range,
    Severity,
};

/// Output channel title shared by every `goctl` run of this plugin.
pub const OUTPUT_CHANNEL: &str = "goctl";
/// Command id declared in `plugin.toml`.
pub const GENERATE_API_GO: &str = "generate-api-go";
/// Command id for generating a go-zero zRPC service from the focused `.proto` file.
pub const GENERATE_ZRPC: &str = "generate-zrpc";
/// Command id that copies the bundled goctl templates into a chosen folder.
pub const GENERATE_TPL_COPY: &str = "generate-tpl-copy";

#[derive(Default)]
pub struct ApiPlugin;

impl Plugin for ApiPlugin {
    fn run_command(
        &mut self,
        id: &str,
        ctx: &Invocation,
        _arg: Option<&str>,
    ) -> Result<Option<String>, Error> {
        match id {
            GENERATE_API_GO => {
                run_api_go(ctx);
                Ok(None)
            }
            GENERATE_ZRPC => {
                run_zrpc(ctx);
                Ok(None)
            }
            GENERATE_TPL_COPY => {
                run_tpl_copy(ctx);
                Ok(None)
            }
            other => Err(Error::NotFound(other.to_owned())),
        }
    }

    fn handle_events(&mut self, _events: &[Event], _ctx: &Invocation) {}
}

impl SyntaxProvider for ApiPlugin {
    fn highlights(&mut self, language: &str, text: &str) -> Vec<Token> {
        match language {
            "api" => api_lexer::syntax_tokens(text),
            "gotpl" => tpl_lexer::syntax_tokens(text),
            _ => Vec::new(),
        }
    }

    fn outline(&mut self, language: &str, text: &str) -> Vec<Symbol> {
        if language == "api" {
            api_lexer::outline(text)
        } else {
            Vec::new()
        }
    }

    fn folding(&mut self, language: &str, text: &str) -> Vec<Fold> {
        match language {
            "api" => api_lexer::folding(text),
            "gotpl" => tpl_lexer::folding(text),
            _ => Vec::new(),
        }
    }
}

/// Build the `goctl api go` argument vector.
///
/// Order is exactly `api go --api FILE --dir OUT --style STYLE`, then
/// `--home HOME` only when a template home is nonempty. Every path stays its
/// own argv element, so spaces and non-ASCII bytes never need shell escaping.
/// Template source for goctl generation, mirroring the IntelliJ dialog's
/// builtin / local (`--home`) / git (`--remote` + optional `--branch`) choice.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TemplateFlags {
    pub home: Option<String>,
    pub remote: Option<String>,
    pub branch: Option<String>,
}

impl TemplateFlags {
    /// Append the matching flags to `args` (builtin adds nothing).
    pub fn append_to(&self, args: &mut Vec<String>) {
        if let Some(home) = self.home.as_ref().filter(|h| !h.is_empty()) {
            args.push("--home".to_owned());
            args.push(home.clone());
            return;
        }
        if let Some(remote) = self.remote.as_ref().filter(|r| !r.is_empty()) {
            args.push("--remote".to_owned());
            args.push(remote.clone());
            if let Some(branch) = self.branch.as_ref().filter(|b| !b.is_empty()) {
                args.push("--branch".to_owned());
                args.push(branch.clone());
            }
        }
    }
}

pub fn api_go_args(
    api_file: &str,
    output_dir: &str,
    style: &str,
    template: &TemplateFlags,
) -> Vec<String> {
    let mut args = vec![
        "api".to_owned(),
        "go".to_owned(),
        "--api".to_owned(),
        api_file.to_owned(),
        "--dir".to_owned(),
        output_dir.to_owned(),
        "--style".to_owned(),
        style.to_owned(),
    ];
    template.append_to(&mut args);
    args
}

/// Read the naming style and template source from settings, with the P1
/// defaults (`gozero` style, built-in templates).
fn generation_settings() -> (String, TemplateFlags) {
    let style = host::setting_string("style")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "gozero".to_owned());
    let source = host::setting_string("templateSource").unwrap_or_default();
    let template = match source.as_str() {
        "local" => TemplateFlags {
            home: host::setting_string("templateHome").filter(|s| !s.is_empty()),
            ..Default::default()
        },
        "git" => TemplateFlags {
            remote: host::setting_string("templateRemote").filter(|s| !s.is_empty()),
            branch: host::setting_string("templateBranch").filter(|s| !s.is_empty()),
            ..Default::default()
        },
        _ => TemplateFlags::default(),
    };
    (style, template)
}

/// Build `goctl rpc protoc` args for the IntelliJ-compatible default layout:
/// generated Go and gRPC stubs go to `<proto-dir>/pb`, zRPC service code goes
/// to the proto directory. Each value remains one argv item.
pub fn zrpc_args(proto_file: &str, style: &str, template: &TemplateFlags) -> Vec<String> {
    let proto_dir = proto_file
        .rsplit_once('/')
        .map_or(".", |(dir, _)| if dir.is_empty() { "/" } else { dir });
    let pb_dir = if proto_dir == "/" {
        "/pb".to_owned()
    } else if proto_dir == "." {
        "pb".to_owned()
    } else {
        format!("{proto_dir}/pb")
    };
    let mut args = vec![
        "rpc".to_owned(),
        "protoc".to_owned(),
        proto_file.to_owned(),
        format!("--proto_path={proto_dir}"),
        format!("--go_out={pb_dir}"),
        format!("--go-grpc_out={pb_dir}"),
        format!("--zrpc_out={proto_dir}"),
        "--style".to_owned(),
        style.to_owned(),
    ];
    template.append_to(&mut args);
    args
}

/// Pick the source file to generate from: the first selected path (a file-tree
/// or tab context menu) with the wanted extension, otherwise the focused editor.
/// `None` when neither yields a match.
fn select_source_file(
    editor_path: Option<String>,
    selected: &[String],
    ext: &str,
) -> Option<String> {
    if let Some(path) = selected.iter().find(|p| p.ends_with(ext)) {
        return Some(path.clone());
    }
    editor_path.filter(|p| p.ends_with(ext))
}

/// Run `goctl rpc protoc` for the focused `.proto` editor.
fn run_zrpc(ctx: &Invocation) {
    let editor_path = ctx.editor().map(|e| e.path());
    let proto_file = match select_source_file(editor_path, &ctx.selected_paths(), ".proto") {
        Some(path) => path,
        None => {
            host::notify(
                NotificationLevel::Warning,
                "Goctl: Generate zRPC from Proto",
                "Open (or select in the project tree) a .proto file first.",
            );
            return;
        }
    };
    let Some(project) = ctx.project() else {
        host::notify(
            NotificationLevel::Warning,
            "Goctl: Generate zRPC from Proto",
            "This window has no open project.",
        );
        return;
    };
    let (style, template) = generation_settings();
    let args = zrpc_args(&proto_file, &style, &template);
    let _ = host::output_clear(OUTPUT_CHANNEL);
    match project.run_process("goctl", &args, None) {
        Ok(result) => {
            render_result(&args, &result);
            let _ = host::output_reveal(OUTPUT_CHANNEL);
            if result.status != Some(0) {
                host::notify(
                    NotificationLevel::Error,
                    "Goctl: Generate zRPC from Proto",
                    "goctl failed; see the goctl output for details.",
                );
            }
        }
        Err(error) => {
            let message = format!("Could not run goctl: {error}");
            let _ = host::output_append(OUTPUT_CHANNEL, &format!("{message}\n"));
            let _ = host::output_reveal(OUTPUT_CHANNEL);
            host::notify(
                NotificationLevel::Error,
                "Goctl: Generate zRPC from Proto",
                &message,
            );
        }
    }
}

/// Ask the installed goctl to initialize its version-matched built-in templates
/// under the user-selected goctl home. goctl owns the version/category layout
/// and preserves existing custom files.
fn run_tpl_copy(ctx: &Invocation) {
    const TITLE: &str = "Goctl: Copy Builtin Templates to Folder";
    let Some(project) = ctx.project() else {
        host::notify(
            NotificationLevel::Warning,
            TITLE,
            "This window has no open project.",
        );
        return;
    };
    let root = match project.root() {
        Ok(root) => root,
        Err(error) => {
            host::notify(
                NotificationLevel::Error,
                TITLE,
                &format!("Could not resolve the project directory: {error}"),
            );
            return;
        }
    };
    let target = match host::pick_directory("Choose a goctl home folder", Some(&root)) {
        Ok(Some(dir)) if !dir.is_empty() => dir,
        Ok(_) => return,
        Err(error) => {
            host::notify(
                NotificationLevel::Error,
                TITLE,
                &format!("Could not choose a folder: {error}"),
            );
            return;
        }
    };
    let args = vec![
        "template".to_owned(),
        "init".to_owned(),
        "--home".to_owned(),
        target.clone(),
    ];
    if let Err(error) = host::output_clear(OUTPUT_CHANNEL) {
        host::notify(
            NotificationLevel::Error,
            TITLE,
            &format!("Could not clear goctl output: {error}"),
        );
    }
    match project.run_process("goctl", &args, None) {
        Ok(result) => {
            render_result(&args, &result);
            let _ = host::output_reveal(OUTPUT_CHANNEL);
            if result.status == Some(0) {
                host::notify(
                    NotificationLevel::Info,
                    TITLE,
                    &format!(
                        "Initialized goctl's built-in templates under {target}. Existing templates are preserved. Set Template source to local and Template home to {target} to use them."
                    ),
                );
            } else {
                host::notify(
                    NotificationLevel::Error,
                    TITLE,
                    "goctl template init failed; see the goctl Output channel.",
                );
            }
        }
        Err(error) => {
            let message = format!("Could not run goctl template init: {error}");
            let _ = host::output_append(OUTPUT_CHANNEL, &format!("{message}\n"));
            let _ = host::output_reveal(OUTPUT_CHANNEL);
            host::notify(NotificationLevel::Error, TITLE, &message);
        }
    }
}

/// Run `goctl api go` for the focused `.api` editor and stream the result into
/// the shared Output channel. Never panics: every failure becomes a
/// notification and (for process results) output text.
fn run_api_go(ctx: &Invocation) {
    let editor_path = ctx.editor().map(|e| e.path());
    let api_file = match select_source_file(editor_path, &ctx.selected_paths(), ".api") {
        Some(path) => path,
        None => {
            host::notify(
                NotificationLevel::Warning,
                "Goctl: Generate Go from API",
                "Open (or select in the project tree) a .api file first.",
            );
            return;
        }
    };

    let project = match ctx.project() {
        Some(project) => project,
        None => {
            host::notify(
                NotificationLevel::Warning,
                "Goctl: Generate Go from API",
                "This window has no open project.",
            );
            return;
        }
    };
    let project_root = match project.root() {
        Ok(root) => root,
        Err(error) => {
            host::notify(
                NotificationLevel::Error,
                "Goctl: Generate Go from API",
                &format!("Could not read the project root: {error}"),
            );
            return;
        }
    };
    let initial = if project_root.is_empty() {
        None
    } else {
        Some(project_root.as_str())
    };

    let output_dir = match host::pick_directory("Select Go output directory", initial) {
        Ok(Some(dir)) if !dir.is_empty() => dir,
        Ok(_) => return, // cancelled: do not spawn
        Err(error) => {
            host::notify(
                NotificationLevel::Error,
                "Goctl: Generate Go from API",
                &format!("Could not choose an output directory: {error}"),
            );
            return;
        }
    };

    let (style, template) = generation_settings();
    let args = api_go_args(&api_file, &output_dir, &style, &template);

    if let Err(error) = host::output_clear(OUTPUT_CHANNEL) {
        host::notify(
            NotificationLevel::Error,
            "Goctl: Generate Go from API",
            &format!("Could not clear the goctl output: {error}"),
        );
    }

    match project.run_process("goctl", &args, None) {
        Ok(result) => {
            render_result(&args, &result);
            let _ = host::output_reveal(OUTPUT_CHANNEL);
            if result.status != Some(0) {
                host::notify(
                    NotificationLevel::Error,
                    "Goctl: Generate Go from API",
                    "goctl failed; see the goctl output for details.",
                );
            }
        }
        Err(error) => {
            let message = format!("Could not run goctl: {error}");
            let _ = host::output_append(OUTPUT_CHANNEL, &format!("{message}\n"));
            let _ = host::output_reveal(OUTPUT_CHANNEL);
            host::notify(
                NotificationLevel::Error,
                "Goctl: Generate Go from API",
                &message,
            );
        }
    }
}

/// Append the command line, both streams, and the exit status to the Output
/// channel. Both streams are kept even when only one carries text.
fn render_result(args: &[String], result: &openidea_plugin::ProcessOutput) {
    let command_line = std::iter::once("goctl".to_owned())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let status = match result.status {
        Some(code) => format!("exit status {code}"),
        None => "terminated by signal".to_owned(),
    };

    let mut text = String::new();
    text.push_str(&format!("$ {command_line}\n"));
    if !result.stdout.is_empty() {
        text.push_str(&result.stdout);
        if !result.stdout.ends_with('\n') {
            text.push('\n');
        }
    }
    if !result.stderr.is_empty() {
        text.push_str("[stderr]\n");
        text.push_str(&result.stderr);
        if !result.stderr.ends_with('\n') {
            text.push('\n');
        }
    }
    text.push_str(&format!("[goctl] {status}\n"));
    let _ = host::output_append(OUTPUT_CHANNEL, &text);
}

// ---------------------------------------------------------------------------
// Diagnostics: version-aware `.api` checking
// ---------------------------------------------------------------------------

/// Best-effort goctl version as `(major, minor)`, read from settings
/// (`goctlVersion`, e.g. `"1.6.4"`). Unknown → `None`, which disables the
/// version-gated rules instead of guessing.
fn configured_goctl_version() -> Option<(u32, u32)> {
    let raw = host::setting_string("goctlVersion")?;
    let mut parts = raw.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    Some((major, minor))
}

fn position_of(text: &str, at: usize) -> Position {
    openidea_plugin::text::position(text, at)
}

impl DiagnosticsProvider for ApiPlugin {
    fn diagnostics(&mut self, doc: &Document) -> Vec<Diagnostic> {
        if doc.language != "api" {
            return Vec::new();
        }
        let reader = CurrentProjectImports::default();
        let issues = api_analysis::lint(&doc.path, &doc.text, configured_goctl_version(), &reader);
        issues
            .into_iter()
            .map(|issue| {
                let range = Range::new(
                    position_of(&doc.text, issue.span.0.min(doc.text.len())),
                    position_of(&doc.text, issue.span.1.min(doc.text.len())),
                );
                let severity = match issue.severity {
                    api_analysis::IssueSeverity::Error => Severity::Error,
                    api_analysis::IssueSeverity::Warning => Severity::Warning,
                    api_analysis::IssueSeverity::Hint => Severity::Hint,
                };
                Diagnostic::new(range, severity, issue.message).code(issue.code)
            })
            .collect()
    }
}

/// Import reader that resolves paths against the open project that owns the
/// file, using `project.read`. Providers do not receive an `Invocation`, so we
/// locate the project via `host::projects()` (same approach as the official
/// mybatis plugin).
#[derive(Default)]
struct CurrentProjectImports;

impl CurrentProjectImports {
    fn project_for(path: &str) -> Option<openidea_plugin::Project> {
        host::projects().ok()?.into_iter().find(|p| {
            p.root()
                .map(|root| path == root || path.starts_with(&format!("{root}/")))
                .unwrap_or(false)
        })
    }
}

impl api_analysis::ImportReader for CurrentProjectImports {
    fn read(&self, path: &str) -> Option<String> {
        let project = Self::project_for(path)?;
        project.read_file(path).ok()
    }
}

// ---------------------------------------------------------------------------
// Definition: jump from Request/Response reference to its declaration
// ---------------------------------------------------------------------------

impl Providers for ApiPlugin {
    fn definition(&mut self, doc: &Document, at: Position) -> Vec<Location> {
        if doc.language != "api" {
            return Vec::new();
        }
        let Some(byte) = openidea_plugin::text::offset(&doc.text, at) else {
            return Vec::new();
        };
        let reader = CurrentProjectImports;
        api_analysis::find_definition(&doc.path, &doc.text, byte, &reader)
            .into_iter()
            .map(|target| {
                // Re-read the target text to convert byte spans to positions;
                // fall back to the current doc when it is the same file.
                let text = if target.path == doc.path {
                    doc.text.clone()
                } else {
                    api_analysis::ImportReader::read(&reader, &target.path).unwrap_or_default()
                };
                Location {
                    path: target.path,
                    range: Range::new(
                        position_of(&text, target.span.0.min(text.len())),
                        position_of(&text, target.span.1.min(text.len())),
                    ),
                }
            })
            .collect()
    }

    fn line_markers(&mut self, doc: &Document) -> Vec<LineMarker> {
        if doc.language != "api" {
            return Vec::new();
        }
        let len = doc.text.len();
        declaration_markers(&doc.text)
            .into_iter()
            .flat_map(|m| {
                // One marker per reference. The host groups every marker on a
                // declaration's line into a single gutter icon whose click opens a
                // chooser listing all distinct targets, so emitting every reference
                // lets the user jump to any of them (not just the first).
                let count = m.references.len();
                let name = m.name;
                let path = doc.path.clone();
                let text = doc.text.as_str();
                m.references
                    .into_iter()
                    .map(move |(start, end)| LineMarker {
                        line: m.line,
                        icon: Some("navigate".to_owned()),
                        tooltip: format!("{name} · {count} 个引用 — 点击选择"),
                        target: Some(Location {
                            path: path.clone(),
                            range: Range::new(
                                position_of(text, start.min(len)),
                                position_of(text, end.min(len)),
                            ),
                        }),
                    })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Document index: reverse lookup (Find Usages) + multi-target Go to Decl
// ---------------------------------------------------------------------------

/// Key prefix for a named type. Definitions and references share the same key
/// (`api:type:<name>`), so the host graph links every use to every declaration.
fn type_key(name: &str) -> String {
    format!("api:type:{name}")
}

struct DeclarationMarker {
    line: u32,
    name: String,
    /// Byte spans of every reference to this declaration outside its own name.
    references: Vec<(usize, usize)>,
}

/// For each declared type, collect the byte spans of all references to it
/// (field/route/alias). Declarations with no reference are emitted empty and
/// skipped by callers. One entry per declaration; callers expand to one marker
/// per reference so the host can offer a chooser across every referrer.
fn declaration_markers(text: &str) -> Vec<DeclarationMarker> {
    let model = api_analysis::parse(text);
    model
        .types
        .iter()
        .map(|decl| {
            let references: Vec<(usize, usize)> = model
                .type_refs
                .iter()
                .filter(|r| r.name == decl.name && !api_analysis::is_primitive(&r.name))
                .map(|r| r.span)
                // Skip a self-reference that lands exactly on the declaration name.
                .filter(|span| *span != decl.name_span)
                .collect();
            let line = text[..decl.name_span.0.min(text.len())]
                .matches('\n')
                .count() as u32;
            DeclarationMarker {
                line,
                name: decl.name.clone(),
                references,
            }
        })
        .collect()
}

impl DocumentIndexer for ApiPlugin {
    fn index(&mut self, doc: &IndexDocument) -> Edges {
        // Only index `.api` documents; other extensions are never routed here.
        if !doc.path.ends_with(".api") {
            return Edges::default();
        }
        let model = api_analysis::parse(&doc.text);
        let mut edges = Edges::default();
        // Definitions: the name token of each type declaration.
        for decl in &model.types {
            edges.definitions.push(Anchor {
                key: type_key(&decl.name),
                range: byte_span_to_range(&doc.text, decl.name_span),
            });
        }
        // References: field types, route request/response, embed, alias targets.
        for reference in &model.type_refs {
            if api_analysis::is_primitive(&reference.name) {
                continue;
            }
            edges.references.push(Anchor {
                key: type_key(&reference.name),
                range: byte_span_to_range(&doc.text, reference.span),
            });
        }
        edges
    }
}

/// Convert a byte span to a UTF-16 `Range`, clamped to the document length.
fn byte_span_to_range(text: &str, span: (usize, usize)) -> Range {
    Range::new(
        position_of(text, span.0.min(text.len())),
        position_of(text, span.1.min(text.len())),
    )
}

openidea_plugin::register_plugin!(ApiPlugin, syntax, providers, diagnostics, indexing);

#[cfg(test)]
mod tests {
    use super::*;
    use openidea_plugin::ProcessOutput;
    use openidea_plugin::mock::MockHost;

    fn caps() -> MockHost {
        MockHost::install().with_capabilities(&["project.read", "process:goctl"])
    }

    #[test]
    fn api_go_args_use_default_style_in_order() {
        let args = api_go_args("/p/user.api", "/p/out", "gozero", &TemplateFlags::default());
        assert_eq!(
            args,
            [
                "api",
                "go",
                "--api",
                "/p/user.api",
                "--dir",
                "/p/out",
                "--style",
                "gozero"
            ]
        );
    }

    #[test]
    fn api_go_args_append_home_only_when_nonempty() {
        let local = TemplateFlags {
            home: Some("/tpl/go".to_owned()),
            ..Default::default()
        };
        let args = api_go_args("/p/user.api", "/p/out", "goZero", &local);
        assert_eq!(&args[args.len() - 2..], ["--home", "/tpl/go"]);

        let blank = TemplateFlags {
            home: Some(String::new()),
            ..Default::default()
        };
        let blank = api_go_args("/p/user.api", "/p/out", "gozero", &blank);
        assert!(!blank.iter().any(|a| a == "--home"));
    }

    #[test]
    fn api_go_args_append_git_remote_and_branch() {
        let git = TemplateFlags {
            remote: Some("https://github.com/zeromicro/go-zero-template".to_owned()),
            branch: Some("main".to_owned()),
            ..Default::default()
        };
        let args = api_go_args("/p/user.api", "/p/out", "gozero", &git);
        assert_eq!(
            &args[args.len() - 4..],
            [
                "--remote",
                "https://github.com/zeromicro/go-zero-template",
                "--branch",
                "main"
            ]
        );
    }

    #[test]
    fn api_go_args_keep_paths_with_spaces_and_unicode_as_single_items() {
        let args = api_go_args(
            "/p 项目/my api.api",
            "/out 目录/gen",
            "go_zero",
            &TemplateFlags::default(),
        );
        assert_eq!(args[3], "/p 项目/my api.api");
        assert_eq!(args[5], "/out 目录/gen");
        assert_eq!(args[7], "go_zero");
        assert_eq!(args.len(), 8);
    }

    #[test]
    fn select_source_file_prefers_tree_selection_then_editor() {
        // The explicit project-tree target wins even while another matching file is open.
        assert_eq!(
            select_source_file(Some("/p/a.api".into()), &["/p/b.api".to_owned()], ".api"),
            Some("/p/b.api".to_owned())
        );
        // With no tree selection (editor context menu), use the focused editor.
        assert_eq!(
            select_source_file(Some("/p/a.api".into()), &[], ".api"),
            Some("/p/a.api".to_owned())
        );
        // Editor path with wrong extension is ignored; fall back to first match.
        assert_eq!(
            select_source_file(
                Some("/p/readme.md".into()),
                &["/p/x.go".to_owned(), "/p/main.api".to_owned()],
                ".api"
            ),
            Some("/p/main.api".to_owned())
        );
        // No editor (file-tree context): pick from selected paths.
        assert_eq!(
            select_source_file(None, &["/p/demo.proto".to_owned()], ".proto"),
            Some("/p/demo.proto".to_owned())
        );
        // Nothing usable.
        assert_eq!(
            select_source_file(None, &["/p/x.go".to_owned()], ".api"),
            None
        );
        assert_eq!(select_source_file(None, &[], ".api"), None);
    }

    #[test]
    fn command_runs_from_file_tree_selection_without_open_editor() {
        // Regression: right-clicking a .api file in the tree (no editor focus)
        // must still generate, resolving the file from the selected paths.
        let host = caps();
        host.set_project_root(Some("/project"));
        host.select_paths(&["/project/app/user.api"]);
        host.set_setting("style", "\"gozero\"");
        host.answer_next_prompt(Some(&["/project/out"]));
        host.on_process("goctl", |_, _| {
            Ok(ProcessOutput {
                status: Some(0),
                stdout: "Done.\n".into(),
                stderr: String::new(),
            })
        });

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_API_GO, &host.invocation(), None)
            .unwrap();

        let runs = host.process_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].args[3], "/project/app/user.api");
    }

    #[test]
    fn command_success_runs_goctl_and_streams_output() {
        let host = caps();
        host.set_project_root(Some("/project"));
        host.open_editor("/project/app/user.api", "api", "type User {}", &[]);
        host.set_setting("style", "\"gozero\"");
        host.answer_next_prompt(Some(&["/project/out"]));
        host.on_process("goctl", |args, _| {
            assert_eq!(args[0], "api");
            Ok(ProcessOutput {
                status: Some(0),
                stdout: "Done.\n".into(),
                stderr: String::new(),
            })
        });

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_API_GO, &host.invocation(), None)
            .unwrap();

        let runs = host.process_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].program, "goctl");
        assert_eq!(runs[0].cwd.as_deref(), Some("/project"));
        assert_eq!(runs[0].args[3], "/project/app/user.api");
        assert_eq!(runs[0].args[5], "/project/out");
        let out = host.output();
        assert!(out.get(OUTPUT_CHANNEL).unwrap().contains("Done."));
    }

    #[test]
    fn command_keeps_stderr_and_nonzero_status() {
        let host = caps();
        host.set_project_root(Some("/project"));
        host.open_editor("/project/user.api", "api", "type User {}", &[]);
        host.answer_next_prompt(Some(&["/project/out"]));
        host.on_process("goctl", |_, _| {
            Ok(ProcessOutput {
                status: Some(2),
                stdout: String::new(),
                stderr: "boom\n".into(),
            })
        });

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_API_GO, &host.invocation(), None)
            .unwrap();

        let out = host.output();
        let text = out.get(OUTPUT_CHANNEL).unwrap();
        assert!(text.contains("boom"));
        assert!(text.contains("exit status 2"));
        assert!(!host.notifications().is_empty(), "failure should notify");
    }

    #[test]
    fn command_uses_saved_style_and_template_home_and_reports_signal_termination() {
        let host = caps();
        host.set_project_root(Some("/project"));
        host.open_editor("/project/中文.api", "api", "type User {}", &[]);
        host.set_setting("style", "\"go_zero\"");
        host.set_setting("templateSource", "\"local\"");
        host.set_setting("templateHome", "\"/templates/自定义\"");
        host.answer_next_prompt(Some(&["/output 目录"]));
        host.on_process("goctl", |_, _| {
            Ok(ProcessOutput {
                status: None,
                stdout: "generated".into(),
                stderr: "notice".into(),
            })
        });

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_API_GO, &host.invocation(), None)
            .unwrap();

        let run = host.process_runs().pop().unwrap();
        assert_eq!(run.args[3], "/project/中文.api");
        assert_eq!(run.args[5], "/output 目录");
        assert_eq!(&run.args[7..], ["go_zero", "--home", "/templates/自定义"]);
        let text = host.output().get(OUTPUT_CHANNEL).unwrap().clone();
        assert!(text.contains("generated"));
        assert!(text.contains("[stderr]\nnotice"));
        assert!(text.contains("terminated by signal"));
        assert!(!host.notifications().is_empty());
    }

    #[test]
    fn command_without_editor_does_not_spawn() {
        let host = caps();
        host.set_project_root(Some("/project"));

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_API_GO, &host.invocation(), None)
            .unwrap();

        assert!(host.process_runs().is_empty());
        assert!(!host.notifications().is_empty());
    }

    #[test]
    fn command_on_non_api_editor_does_not_spawn() {
        let host = caps();
        host.set_project_root(Some("/project"));
        host.open_editor("/project/main.go", "go", "package main", &[]);

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_API_GO, &host.invocation(), None)
            .unwrap();

        assert!(host.process_runs().is_empty());
        assert!(!host.notifications().is_empty());
    }

    #[test]
    fn command_without_project_does_not_spawn() {
        let host = caps();
        host.set_project_root(None);
        host.open_editor("/project/user.api", "api", "type User {}", &[]);

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_API_GO, &host.invocation(), None)
            .unwrap();

        assert!(host.process_runs().is_empty());
        assert!(!host.notifications().is_empty());
    }

    #[test]
    fn command_cancelled_directory_picker_does_not_spawn() {
        let host = caps();
        host.set_project_root(Some("/project"));
        host.open_editor("/project/user.api", "api", "type User {}", &[]);
        host.answer_next_prompt(None);

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_API_GO, &host.invocation(), None)
            .unwrap();

        assert!(host.process_runs().is_empty());
    }

    #[test]
    fn zrpc_args_use_intellij_default_output_layout() {
        let local = TemplateFlags {
            home: Some("/templates/go".to_owned()),
            ..Default::default()
        };
        let args = zrpc_args("/project/proto/user.proto", "gozero", &local);
        assert_eq!(
            args,
            [
                "rpc",
                "protoc",
                "/project/proto/user.proto",
                "--proto_path=/project/proto",
                "--go_out=/project/proto/pb",
                "--go-grpc_out=/project/proto/pb",
                "--zrpc_out=/project/proto",
                "--style",
                "gozero",
                "--home",
                "/templates/go",
            ]
        );
        let relative = zrpc_args("user.proto", "gozero", &TemplateFlags::default());
        assert_eq!(relative[3], "--proto_path=.");
        assert_eq!(relative[4], "--go_out=pb");
        assert_eq!(relative[6], "--zrpc_out=.");
    }

    #[test]
    fn zrpc_command_runs_for_proto_editor() {
        let host = caps();
        host.set_project_root(Some("/project"));
        host.open_editor(
            "/project/proto/user.proto",
            "protobuf",
            r#"syntax = "proto3";"#,
            &[],
        );
        host.set_setting("style", "\"go_zero\"");
        host.on_process("goctl", |args, _| {
            assert_eq!(args[0], "rpc");
            assert_eq!(args[1], "protoc");
            Ok(ProcessOutput {
                status: Some(0),
                stdout: "generated zRPC".into(),
                stderr: String::new(),
            })
        });

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_ZRPC, &host.invocation(), None)
            .unwrap();

        let run = host.process_runs().pop().unwrap();
        assert_eq!(run.args[2], "/project/proto/user.proto");
        assert_eq!(run.args[4], "--go_out=/project/proto/pb");
        assert_eq!(run.args[6], "--zrpc_out=/project/proto");
        assert_eq!(run.args[8], "go_zero");
        assert!(
            host.output()
                .get(OUTPUT_CHANNEL)
                .unwrap()
                .contains("generated zRPC")
        );
    }

    #[test]
    fn zrpc_run_failure_writes_one_line_without_escape_text() {
        let host = caps();
        host.set_project_root(Some("/project"));
        host.open_editor(
            "/project/proto/user.proto",
            "protobuf",
            r#"syntax = "proto3";"#,
            &[],
        );
        host.on_process("goctl", |_, _| Err(Error::NotFound("goctl".into())));

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_ZRPC, &host.invocation(), None)
            .unwrap();

        let output = host.output();
        let text = output.get(OUTPUT_CHANNEL).expect("failure message");
        assert!(text.contains("Could not run goctl"), "{text:?}");
        assert!(
            !text.contains("\\n"),
            "output carries a literal escape instead of a line break: {text:?}"
        );
    }

    #[test]
    fn diagnostics_apply_version_gate_from_setting() {
        let host = caps();
        host.set_setting("goctlVersion", "\"1.6.4\"");
        host.open_editor("/project/main.api", "api", "type User { id int64 }", &[]);
        let mut plugin = ApiPlugin;
        host.refresh_diagnostics(&mut plugin);
        let issues = host.provider_diagnostics();
        assert!(
            issues["/project/main.api"]
                .iter()
                .any(|d| d.code.as_deref() == Some("api.syntax.missing"))
        );

        host.set_setting("goctlVersion", "\"1.5.1\"");
        host.refresh_diagnostics(&mut plugin);
        let issues = host.provider_diagnostics();
        assert!(!issues.get("/project/main.api").is_some_and(|items| {
            items
                .iter()
                .any(|d| d.code.as_deref() == Some("api.syntax.missing"))
        }));
    }

    #[test]
    fn definition_provider_resolves_request_and_response_to_local_types() {
        let host = caps();
        let source = r#"syntax = "v1"
type Request { id int64 }
type Response { ok bool }
service user-api { @handler H post /x (Request) returns (Response) }"#;
        host.open_editor("/project/main.api", "api", source, &[]);
        let doc = host.provider_document().unwrap();
        let mut plugin = ApiPlugin;
        let req = source.find("(Request)").unwrap() + 2;
        let req_position = openidea_plugin::text::position(source, req);
        let req_result = plugin.definition(&doc, req_position);
        assert_eq!(req_result.len(), 1);
        assert_eq!(req_result[0].path, "/project/main.api");
        assert_eq!(
            req_result[0].range.start,
            openidea_plugin::text::position(source, source.find("Request").unwrap())
        );

        let resp = source.find("(Response)").unwrap() + 2;
        let resp_result = plugin.definition(&doc, openidea_plugin::text::position(source, resp));
        assert_eq!(resp_result.len(), 1);
        assert_eq!(
            resp_result[0].range.start,
            openidea_plugin::text::position(source, source.find("Response").unwrap())
        );
    }

    #[test]
    fn syntax_provider_returns_highlights_outline_and_folding_for_api() {
        let mut plugin = ApiPlugin;
        let source = "type User {\n id int64\n}\n";

        assert!(!plugin.highlights("api", source).is_empty());
        assert_eq!(plugin.outline("api", source)[0].name, "User");
        assert!(!plugin.folding("api", source).is_empty());
    }

    #[test]
    fn indexer_emits_definition_and_reference_anchors_sharing_a_key() {
        let src = r#"syntax = "v1"
type Request { id int64 }
type Response { ok bool }
type Box { items []Request }
service user-api {
  @handler H get /x (Request) returns (Response)
}"#;
        let doc = IndexDocument {
            path: "/project/main.api".to_owned(),
            text: src.to_owned(),
            symbols: Vec::new(),
        };
        let mut plugin = ApiPlugin;
        let edges = plugin.index(&doc);

        // Definitions for Request, Response, Box.
        let def_keys: Vec<_> = edges.definitions.iter().map(|a| a.key.as_str()).collect();
        assert!(def_keys.contains(&"api:type:Request"));
        assert!(def_keys.contains(&"api:type:Response"));
        assert!(def_keys.contains(&"api:type:Box"));
        // The Box definition anchor covers the name, not the body.
        let box_def = edges
            .definitions
            .iter()
            .find(|a| a.key == "api:type:Box")
            .unwrap();
        assert_eq!(
            box_def.range.start,
            position_of(src, src.find("Box").unwrap())
        );

        // References to Request: field []Request + route (Request) = 2.
        let req_refs = edges
            .references
            .iter()
            .filter(|a| a.key == "api:type:Request")
            .count();
        assert_eq!(req_refs, 2, "refs={:?}", edges.references);
        // Reference to Response: the route returns (Response) = 1.
        let resp_refs = edges
            .references
            .iter()
            .filter(|a| a.key == "api:type:Response")
            .count();
        assert_eq!(resp_refs, 1);
    }

    #[test]
    fn indexer_skips_non_api_paths() {
        let doc = IndexDocument {
            path: "/project/main.go".to_owned(),
            text: "package main".to_owned(),
            symbols: Vec::new(),
        };
        let mut plugin = ApiPlugin;
        let edges = plugin.index(&doc);
        assert!(edges.definitions.is_empty());
        assert!(edges.references.is_empty());
    }

    fn marker_reference_starts(src: &str, name: &str) -> Vec<openidea_plugin::Position> {
        // Byte offsets of every reference to `name` outside its own declaration,
        // matching how the indexer emits `references` anchors.
        let model = api_analysis::parse(src);
        let decl_name_span = model
            .types
            .iter()
            .find(|d| d.name == name)
            .map(|d| d.name_span)
            .expect("declaration exists");
        model
            .type_refs
            .iter()
            .filter(|r| r.name == name && !api_analysis::is_primitive(&r.name))
            .map(|r| r.span)
            .filter(|span| *span != decl_name_span)
            .map(|span| openidea_plugin::text::position(src, span.0))
            .collect()
    }

    #[test]
    fn line_markers_emit_one_marker_per_reference_so_all_referrers_are_reachable() {
        // A Request referenced in two routes must produce two markers (host groups
        // them on the declaration line → chooser with every referrer).
        let src = r#"syntax = "v1"
type Request { id int64 }
type Lonely { x int64 }
service user-api {
  @handler A get /a (Request) returns (Request)
  @handler B get /b (Request) returns (Request)
}"#;
        let doc = Document {
            path: "/p/main.api".into(),
            language: "api".into(),
            version: 0,
            text: src.into(),
        };
        let mut plugin = ApiPlugin;
        let markers = plugin.line_markers(&doc);
        let expected = marker_reference_starts(src, "Request");
        // Four references to Request: /a req, /a resp, /b req, /b resp.
        assert_eq!(expected.len(), 4);
        assert_eq!(markers.len(), 4);
        let decl_line = src[..src.find("Request {").unwrap()].matches('\n').count() as u32;
        for marker in &markers {
            assert_eq!(marker.line, decl_line);
            assert_eq!(marker.icon.as_deref(), Some("navigate"));
            assert!(marker.tooltip.contains("Request"));
            assert_eq!(marker.tooltip, "Request · 4 个引用 — 点击选择");
            let target = marker.target.as_ref().expect("has target");
            assert_eq!(target.path, "/p/main.api");
        }
        let starts: Vec<_> = markers
            .iter()
            .map(|m| m.target.as_ref().unwrap().range.start)
            .collect();
        assert_eq!(starts, expected);
    }

    #[test]
    fn line_markers_skip_declarations_without_references() {
        let src = "syntax = \"v1\"\ntype Alone { x int64 }\n";
        let doc = Document {
            path: "/p/main.api".into(),
            language: "api".into(),
            version: 0,
            text: src.into(),
        };
        let mut plugin = ApiPlugin;
        assert!(plugin.line_markers(&doc).is_empty());
    }

    #[test]
    fn line_markers_skip_non_api_and_handle_malformed_without_panic() {
        let mut plugin = ApiPlugin;
        let other = Document {
            path: "/p/x.go".into(),
            language: "go".into(),
            version: 0,
            text: "package x".into(),
        };
        assert!(plugin.line_markers(&other).is_empty());
        let broken = Document {
            path: "/p/b.api".into(),
            language: "api".into(),
            version: 0,
            text: "type { {{{ ( [".into(),
        };
        let _ = plugin.line_markers(&broken);
    }

    #[test]
    fn definition_resolves_slice_element_for_route_type() {
        // Regression for slice field element: []Request must be clickable.
        let host = caps();
        let source = "syntax = \"v1\"\ntype Request { id int64 }\ntype Box { items []Request }\n";
        host.open_editor("/project/main.api", "api", source, &[]);
        let doc = host.provider_document().unwrap();
        let mut plugin = ApiPlugin;
        let at = source.find("[]Request").unwrap() + 4;
        let result = plugin.definition(&doc, openidea_plugin::text::position(source, at));
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].range.start,
            openidea_plugin::text::position(source, source.find("Request").unwrap())
        );
    }

    #[test]
    fn tpl_copy_runs_goctl_template_init_for_the_picked_home() {
        let host = MockHost::install().with_capabilities(&["project.read", "process:goctl"]);
        host.set_project_root(Some("/project"));
        host.answer_next_prompt(Some(&["/custom/goctl home"]));
        host.on_process("goctl", |args, _| {
            assert_eq!(args, ["template", "init", "--home", "/custom/goctl home"]);
            Ok(ProcessOutput {
                status: Some(0),
                stdout: "Templates initialized under /custom/goctl home/1.10.1\n".into(),
                stderr: String::new(),
            })
        });

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_TPL_COPY, &host.invocation(), None)
            .unwrap();

        let runs = host.process_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].program, "goctl");
        assert_eq!(runs[0].cwd.as_deref(), Some("/project"));
        let output = host.output();
        assert!(
            output
                .get(OUTPUT_CHANNEL)
                .unwrap()
                .contains("Templates initialized")
        );
        assert!(
            host.notifications()
                .iter()
                .any(|n| n.body.contains("Template home to /custom/goctl home"))
        );
    }

    #[test]
    fn tpl_copy_reports_failed_goctl_template_init() {
        let host = MockHost::install().with_capabilities(&["project.read", "process:goctl"]);
        host.set_project_root(Some("/project"));
        host.answer_next_prompt(Some(&["/custom/templates"]));
        host.on_process("goctl", |_, _| {
            Ok(ProcessOutput {
                status: Some(2),
                stdout: String::new(),
                stderr: "permission denied\n".into(),
            })
        });
        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_TPL_COPY, &host.invocation(), None)
            .unwrap();
        let text = host.output().get(OUTPUT_CHANNEL).unwrap().clone();
        assert!(text.contains("permission denied"));
        assert!(text.contains("exit status 2"));
        assert!(
            host.notifications()
                .iter()
                .any(|n| n.level == NotificationLevel::Error)
        );
    }

    #[test]
    fn tpl_lexer_handles_all_bundled_goctl_template_fixtures() {
        for (path, source) in tpl_assets::TEMPLATES {
            let tokens = tpl_lexer::lex(source);
            for token in tokens {
                assert!(token.start <= token.end, "invalid token span in {path}");
                assert!(token.end <= source.len(), "token exceeds {path}");
                assert!(
                    source.is_char_boundary(token.start),
                    "start splits UTF-8 in {path}"
                );
                assert!(
                    source.is_char_boundary(token.end),
                    "end splits UTF-8 in {path}"
                );
            }
        }
    }

    #[test]
    fn tpl_copy_reports_when_the_folder_picker_is_cancelled() {
        let host = MockHost::install().with_capabilities(&["project.read", "process:goctl"]);
        host.set_project_root(Some("/project"));
        host.answer_next_prompt(None);

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_TPL_COPY, &host.invocation(), None)
            .unwrap();

        assert!(host.output().get(OUTPUT_CHANNEL).is_none());
    }

    #[test]
    fn tpl_copy_without_a_project_notifies_and_stops() {
        let host = MockHost::install().with_capabilities(&["project.read", "process:goctl"]);
        host.set_project_root(None);

        let mut plugin = ApiPlugin;
        plugin
            .run_command(GENERATE_TPL_COPY, &host.invocation(), None)
            .unwrap();

        assert!(host.output().get(OUTPUT_CHANNEL).is_none());
        assert!(
            host.notifications()
                .iter()
                .any(|n| n.body.contains("no open project")),
            "expected a no-project notification"
        );
    }
}
