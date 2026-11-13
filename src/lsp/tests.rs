use super::*;
use crate::lsp::project::ProjectIndex;
use serde_json::{json, Value};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufStream, DuplexStream};

async fn send(stream: &mut BufStream<DuplexStream>, message: Value) {
    let body = message.to_string();
    stream
        .write_all(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
        .await
        .unwrap();
    stream.flush().await.unwrap();
}

async fn receive(stream: &mut BufStream<DuplexStream>) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut header = String::new();
        stream.read_line(&mut header).await.unwrap();
        let length: usize = header
            .strip_prefix("Content-Length: ")
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut blank = String::new();
        stream.read_line(&mut blank).await.unwrap();
        assert_eq!(blank, "\r\n");
        let mut body = vec![0; length];
        stream.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    })
    .await
    .expect("LSP server did not respond")
}

#[tokio::test]
async fn native_async_backend_serves_the_document_lifecycle_over_lsp() {
    let (service, socket) = LspService::new(Backend::new);
    let (client, server) = tokio::io::duplex(8192);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(
        tower_lsp_server::Server::new(input, output, socket)
            .concurrency_level(1)
            .serve(service),
    );
    let mut client = BufStream::new(client);

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}
        }),
    )
    .await;
    let initialized = receive(&mut client).await;
    assert_eq!(initialized["id"], 1);
    assert_eq!(
        initialized["result"]["capabilities"]["definitionProvider"],
        true
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], "soli");
    assert_eq!(
        initialized["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "method": "initialized", "params": {}
        }),
    )
    .await;
    assert_eq!(receive(&mut client).await["method"], "window/logMessage");

    let uri = "file:///app/greet.sl";
    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": {"textDocument": {
                "uri": uri, "languageId": "soli", "version": 1,
                "text": "def greet(name)\n  name\nend\n\nprint(greet(\"x\"))\n"
            }}
        }),
    )
    .await;
    let diagnostics = receive(&mut client).await;
    assert_eq!(diagnostics["method"], "textDocument/publishDiagnostics");
    assert_eq!(diagnostics["params"]["uri"], uri);
    assert_eq!(diagnostics["params"]["diagnostics"], json!([]));

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition",
            "params": {"textDocument": {"uri": uri}, "position": {"line": 4, "character": 7}}
        }),
    )
    .await;
    let definition = receive(&mut client).await;
    assert_eq!(definition["result"]["uri"], uri);
    assert_eq!(
        definition["result"]["range"]["start"],
        json!({"line": 0, "character": 4})
    );

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "textDocument/rename",
            "params": {
                "textDocument": {"uri": uri}, "position": {"line": 0, "character": 5},
                "newName": "welcome"
            }
        }),
    )
    .await;
    let renamed = receive(&mut client).await;
    let edits = renamed["result"]["changes"][uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| edit["newText"] == "welcome"));

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": {"uri": uri, "version": 2},
                "contentChanges": [{"text": "def greet(\n"}]
            }
        }),
    )
    .await;
    let diagnostics = receive(&mut client).await;
    assert_eq!(diagnostics["method"], "textDocument/publishDiagnostics");
    assert_eq!(diagnostics["params"]["diagnostics"][0]["severity"], 1);

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didClose",
            "params": {"textDocument": {"uri": uri}}
        }),
    )
    .await;
    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 4, "method": "textDocument/definition",
            "params": {"textDocument": {"uri": uri}, "position": {"line": 0, "character": 5}}
        }),
    )
    .await;
    let closed = receive(&mut client).await;
    assert_eq!(closed["id"], 4);
    assert_eq!(closed["result"], Value::Null);
    assert!(closed.get("error").is_none());

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 5, "method": "shutdown"
        }),
    )
    .await;
    let shutdown = receive(&mut client).await;
    assert_eq!(shutdown["id"], 5);
    assert_eq!(shutdown["result"], Value::Null);
    assert!(shutdown.get("error").is_none());
    send(&mut client, json!({"jsonrpc": "2.0", "method": "exit"})).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("LSP server did not exit")
        .unwrap();
}

#[test]
fn a_syntax_error_diagnostic_carries_its_error_code() {
    let error = crate::lint("def broken(\n").unwrap_err();
    let diagnostic = syntax_error_diagnostic(&error);
    let code = match diagnostic.code {
        Some(lsp_types::NumberOrString::String(code)) => code,
        other => panic!("no code: {other:?}"),
    };
    assert!(code.starts_with("E02"), "{code}");
}

// ---------------------------------------------------------------------------
// Diagnostics routing: `.slv` templates and path-aware rules (#691)
// ---------------------------------------------------------------------------

fn diagnostics_at(text: &str, path: &str) -> Vec<lsp_types::Diagnostic> {
    diagnostics_for(text, Some(Path::new(path)), Some(&Default::default()))
}

#[test]
fn a_template_with_html_apostrophes_gets_no_syntax_error() {
    let text =
        "<p>It's <%= @post.title %></p>\n<% if @post.published %>\n  <p>Don't</p>\n<% end %>\n";
    let found = diagnostics_at(text, "/site/app/views/posts/show.html.slv");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn an_unclosed_template_tag_is_an_error_on_its_line() {
    let text = "<h1>Title</h1>\n<p>it's</p>\n<%= @post.title\n<p>after</p>\n";
    let found = diagnostics_at(text, "/site/app/views/posts/show.html.slv");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].severity,
        Some(lsp_types::DiagnosticSeverity::ERROR)
    );
    assert_eq!(found[0].range.start, lsp_types::Position::new(2, 0));
    assert_eq!(found[0].range.end, lsp_types::Position::new(2, 15));

    // An `if` the template engine never sees closed.
    let text = "<p>it's</p>\n<% if @show %>\n<p>a</p>\n";
    let found = diagnostics_at(text, "/site/app/views/posts/show.html.slv");
    assert!(
        found
            .iter()
            .any(|d| d.severity == Some(lsp_types::DiagnosticSeverity::ERROR)
                && d.range.start.line == 1),
        "{found:?}"
    );
}

#[test]
fn a_redundant_escape_is_reported_on_its_whole_line() {
    let text = "<p>it's</p>\n<p><%= h(@post.title) %></p>\n";
    let found = diagnostics_at(text, "/site/app/views/posts/show.html.slv");
    let escape = found
        .iter()
        .find(|d| {
            d.code
                == Some(lsp_types::NumberOrString::String(
                    "idiom/redundant-template-escape".into(),
                ))
        })
        .unwrap_or_else(|| panic!("{found:?}"));
    assert_eq!(escape.range.start, lsp_types::Position::new(1, 0));
    assert_eq!(escape.range.end, lsp_types::Position::new(1, 28));
}

#[test]
fn a_controller_buffer_gets_the_path_aware_rules_with_exact_ranges() {
    let text = "class PostsController < Controller\n  # POST /posts\n  def create(req)\n    Post.create(params)\n  end\nend\n";
    let found = diagnostics_at(text, "/site/app/controllers/posts_controller.sl");
    let mass = found
        .iter()
        .find(|d| {
            d.code
                == Some(lsp_types::NumberOrString::String(
                    "security/unfiltered-mass-assignment".into(),
                ))
        })
        .unwrap_or_else(|| panic!("{found:?}"));
    // `params`, not the column plus the length of the message.
    assert_eq!(mass.range.start, lsp_types::Position::new(3, 16));
    assert_eq!(mass.range.end, lsp_types::Position::new(3, 22));
    // Without its path the rule does not apply: what the server used to do.
    assert!(diagnostics_for(text, None, None)
        .iter()
        .all(|d| d.code != mass.code));
}

#[test]
fn known_names_stand_in_for_the_disk_walk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("app/services")).unwrap();
    std::fs::create_dir_all(root.join("app/controllers")).unwrap();
    std::fs::write(root.join("app/services/billing.sl"), "default_rate = 3\n").unwrap();
    let controller = root.join("app/controllers/payments_controller.sl");
    let text = "def pay(req)\n  total = default_rate * 100\n  ghost + total\nend\n";
    let undefined = |known: Option<&std::collections::HashSet<String>>| -> Vec<String> {
        diagnostics_for(text, Some(&controller), known)
            .into_iter()
            .filter(|d| {
                d.code
                    == Some(lsp_types::NumberOrString::String(
                        "smell/undefined-local".into(),
                    ))
            })
            .map(|d| d.message)
            .collect()
    };
    // The disk walk finds `default_rate`; `ghost` is undefined either way.
    let walked = undefined(None);
    assert!(
        walked.iter().all(|m| !m.contains("default_rate")),
        "{walked:?}"
    );
    assert!(walked.iter().any(|m| m.contains("ghost")), "{walked:?}");
    // Given names, the disk is not read: an empty set leaves `default_rate`
    // undefined although the file declaring it is right there.
    let empty = Default::default();
    assert!(undefined(Some(&empty))
        .iter()
        .any(|m| m.contains("default_rate")));
    let known: std::collections::HashSet<String> = ["default_rate".to_string()].into();
    assert_eq!(undefined(Some(&known)), walked);
}

// ---------------------------------------------------------------------------
// The project: cross-file definition, hover and member completion (#691)
// ---------------------------------------------------------------------------

fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

const POST_MODEL: &str = "class Post < Model\n  title: String\n  scope(\"published\", fn() { this.where(\"doc.published == true\") })\n  has_many(\"comments\")\n\n  def summary(width: Int) -> String\n    @title\n  end\n\n  static def recent\n    Post.all\n  end\nend\n";

fn project() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root, "config/routes.sl", "resources(\"posts\")\n");
    write(&root, "app/models/post.sl", POST_MODEL);
    write(
        &root,
        "app/models/featured_post.sl",
        "class FeaturedPost < Post\n  rank: Int\n\n  def badge\n    \"*\"\n  end\nend\n",
    );
    write(
        &root,
        "app/helpers/format.sl",
        "# Cents to a price.\ndef format_price(cents)\n  \"#{cents / 100}\"\nend\n",
    );
    write(
        &root,
        "app/controllers/posts_controller.sl",
        "class PostsController < Controller\n  # GET /posts/:id\n  def show(req)\n    @post = Post.find(req.params[\"id\"])\n  end\nend\n",
    );
    (dir, root)
}

fn labels(response: Option<lsp_types::CompletionResponse>) -> Vec<String> {
    match response {
        Some(lsp_types::CompletionResponse::Array(items)) => {
            items.into_iter().map(|item| item.label).collect()
        }
        other => panic!("{other:?}"),
    }
}

fn complete(index: &ProjectIndex, path: &Path, text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let position = lsp_types::Position::new(
        (lines.len() - 1) as u32,
        lines.last().unwrap().encode_utf16().count() as u32,
    );
    let namespace = index.namespace_of(path);
    let table = symbols::build_symbol_table(text);
    labels(completions::get_completions(
        text,
        position,
        table.as_ref(),
        Some((index, namespace)),
    ))
}

#[test]
fn a_class_lists_its_statics_scopes_and_the_model_class_methods() {
    let (_dir, root) = project();
    let index = ProjectIndex::scan(&root);
    let controller = root.join("app/controllers/posts_controller.sl");
    let found = complete(&index, &controller, "def show(req)\n  Post.");
    for name in ["recent", "published", "where", "find_by", "create", "new"] {
        assert!(found.contains(&name.to_string()), "{name}: {found:?}");
    }
    assert!(!found.contains(&"summary".to_string()), "{found:?}");
    let found = complete(&index, &controller, "def show(req)\n  Post.fi");
    assert!(
        found.iter().all(|label| label.starts_with("fi")),
        "{found:?}"
    );
    assert!(found.contains(&"find_by".to_string()));
}

#[test]
fn an_instance_lists_its_members_and_the_model_instance_methods() {
    let (_dir, root) = project();
    let index = ProjectIndex::scan(&root);
    let controller = root.join("app/controllers/posts_controller.sl");
    // The buffer does not parse (`post.` is half typed): it still completes.
    let found = complete(
        &index,
        &controller,
        "def show(id)\n  post = Post.find(id)\n  post.",
    );
    for name in ["summary", "title", "comments", "save", "update"] {
        assert!(found.contains(&name.to_string()), "{name}: {found:?}");
    }
    assert!(!found.contains(&"recent".to_string()), "{found:?}");
}

#[test]
fn at_inside_a_class_lists_its_fields_and_inherited_members() {
    let (_dir, root) = project();
    let index = ProjectIndex::scan(&root);
    let path = root.join("app/models/featured_post.sl");
    let text = "class FeaturedPost < Post\n  rank: Int\n\n  def badge\n    @";
    let found = complete(&index, &path, text);
    for name in ["rank", "badge", "title", "summary", "save"] {
        assert!(found.contains(&name.to_string()), "{name}: {found:?}");
    }
}

#[test]
fn a_literal_lists_its_builtin_methods_and_an_unknown_receiver_every_project_method() {
    let (_dir, root) = project();
    let index = ProjectIndex::scan(&root);
    let path = root.join("app/controllers/posts_controller.sl");
    let found = complete(&index, &path, "x = \"hello\".");
    assert!(found.contains(&"upcase".to_string()), "{found:?}");
    assert!(found.contains(&"length".to_string()), "{found:?}");
    let found = complete(&index, &path, "thing.");
    assert!(found.contains(&"summary".to_string()), "{found:?}");
    assert!(found.contains(&"badge".to_string()), "{found:?}");
    assert!(!found.contains(&"upcase".to_string()), "{found:?}");
}

#[test]
fn a_word_completes_project_classes_and_helpers_in_reach() {
    let (_dir, root) = project();
    let index = ProjectIndex::scan(&root);
    let controller = root.join("app/controllers/posts_controller.sl");
    let found = complete(&index, &controller, "Fea");
    assert!(found.contains(&"FeaturedPost".to_string()), "{found:?}");
    // A view helper is not in a controller's environment.
    assert!(!complete(&index, &controller, "format_").contains(&"format_price".to_string()));
    let view = root.join("app/views/posts/show.html.slv");
    assert!(complete(&index, &view, "format_").contains(&"format_price".to_string()));
}

#[test]
fn a_dot_in_a_comment_or_a_string_completes_nothing() {
    let (_dir, root) = project();
    let index = ProjectIndex::scan(&root);
    let at_end = |text: &str| {
        let lines: Vec<&str> = text.split('\n').collect();
        let position = lsp_types::Position::new(
            (lines.len() - 1) as u32,
            lines.last().unwrap().encode_utf16().count() as u32,
        );
        completions::get_completions(text, position, None, Some((&index, None)))
    };
    assert!(at_end("x = 1 # see the docs.").is_none());
    assert!(at_end("email = \"me@").is_none());
    assert!(at_end("label = \"a sentence.").is_none());
    // An interpolation is code again, and a closed string is a receiver.
    assert!(at_end("label = \"#{thing.").is_some());
    assert!(at_end("label = \"x\".").is_some());
}

#[test]
fn a_template_completes_only_inside_its_tags() {
    let text = "<p>It's done.</p>\n<%= post.";
    let end = lsp_types::Position::new(1, 10);
    assert!(completions::in_template_code(text, end));
    assert!(!completions::in_template_code(
        text,
        lsp_types::Position::new(0, 13)
    ));
    assert!(!completions::in_template_code(
        "<% x %> done.",
        lsp_types::Position::new(0, 13)
    ));
}

#[test]
fn only_soli_documents_get_soli_features() {
    let uri = |s: &str| -> Uri { s.parse().unwrap() };
    assert!(is_soli_code(&uri("file:///app/models/post.sl")));
    assert!(is_soli_code(&uri("file:///project/Solifile")));
    assert!(is_soli_document(&uri(
        "file:///app/views/posts/show.html.slv"
    )));
    assert!(!is_soli_code(&uri("file:///app/views/posts/show.html.slv")));
    assert!(!is_soli_document(&uri("file:///public/index.html")));
}

#[test]
fn definition_and_hover_cross_files() {
    let (_dir, root) = project();
    let index = ProjectIndex::scan(&root);
    let controller = root.join("app/controllers/posts_controller.sl");
    let uri = Uri::from_file_path(&controller).unwrap();
    let text = std::fs::read_to_string(&controller).unwrap();
    let table = symbols::build_symbol_table(&text);
    let namespace = index.namespace_of(&controller);

    // `Post` in `Post.find(…)`, line 3.
    let at_post = lsp_types::Position::new(3, 13);
    let target = goto::goto_definition(
        &text,
        at_post,
        table.as_ref(),
        &uri,
        Some((&index, namespace)),
    );
    let Some(lsp_types::GotoDefinitionResponse::Scalar(location)) = target else {
        panic!("{target:?}");
    };
    assert_eq!(
        location.uri,
        Uri::from_file_path(root.join("app/models/post.sl")).unwrap()
    );
    assert_eq!(location.range.start, lsp_types::Position::new(0, 6));
    assert_eq!(location.range.end, lsp_types::Position::new(0, 10));

    // `find` is Model's, not the project's: nothing to jump to.
    let at_find = lsp_types::Position::new(3, 18);
    assert!(goto::goto_definition(
        &text,
        at_find,
        table.as_ref(),
        &uri,
        Some((&index, namespace))
    )
    .is_none());

    let hovered =
        hover::get_hover(&text, at_post, table.as_ref(), Some((&index, namespace))).unwrap();
    let lsp_types::HoverContents::Array(items) = hovered.contents else {
        panic!();
    };
    let lsp_types::MarkedString::String(text) = &items[0] else {
        panic!();
    };
    assert!(text.contains("class Post < Model"), "{text}");
    assert!(text.contains("app/models/post.sl:1"), "{text}");

    // A member through a resolved receiver.
    let source = "def show(id)\n  post = Post.find(id)\n  post.summary(3)\nend\n";
    let table = symbols::build_symbol_table(source);
    let at_summary = lsp_types::Position::new(2, 8);
    let target = goto::goto_definition(
        source,
        at_summary,
        table.as_ref(),
        &uri,
        Some((&index, namespace)),
    );
    let Some(lsp_types::GotoDefinitionResponse::Scalar(location)) = target else {
        panic!("{target:?}");
    };
    assert_eq!(location.range.start, lsp_types::Position::new(5, 6));
    let hovered = hover::get_hover(
        source,
        at_summary,
        table.as_ref(),
        Some((&index, namespace)),
    )
    .unwrap();
    let lsp_types::HoverContents::Array(items) = hovered.contents else {
        panic!();
    };
    assert!(
        matches!(&items[0], lsp_types::MarkedString::String(t) if t.contains("def summary(width: Int) -> String")),
        "{items:?}"
    );
}

fn request<'a>(
    uri: &'a Uri,
    source: &'a str,
    position: lsp_types::Position,
    template: bool,
    table: Option<&'a SymbolTable>,
) -> references::Request<'a> {
    references::Request {
        uri,
        source,
        position,
        template,
        table,
    }
}

fn files_of(locations: &[lsp_types::Location], root: &Path) -> Vec<String> {
    let mut files: Vec<String> = locations
        .iter()
        .map(|location| {
            // On Windows the canonical root is `\\?\C:\…` while the URI's
            // path is `C:\…` (its drive letter possibly in another case):
            // compare them without the verbatim prefix, ignoring case.
            let unverbatim = |p: &Path| p.to_string_lossy().trim_start_matches(r"\\?\").to_string();
            let path = unverbatim(&location.uri.to_file_path().unwrap());
            let root = unverbatim(root);
            assert!(
                path.to_lowercase().starts_with(&root.to_lowercase()),
                "{path} is not under {root}"
            );
            path[root.len()..]
                .trim_start_matches(['/', '\\'])
                .replace('\\', "/")
        })
        .collect();
    files.sort();
    files
}

#[test]
fn references_rename_and_doc_hover_cross_files() {
    let (_dir, root) = project();
    write(
        &root,
        "app/views/posts/show.html.slv",
        "<h1>Post format_price</h1>\n<%= format_price(@post.cents) %>\n<%# format_price %>\n",
    );
    write(&root, "tests/post_spec.sl", "recent = Post.recent\n");
    let index = ProjectIndex::scan(&root);
    let texts =
        |open: &HashMap<PathBuf, String>| references::project_texts(index.source_files(), open);

    // `Post` at its declaration: every file that names the class, the view's
    // markup excluded.
    let model = root.join("app/models/post.sl");
    let model_uri = Uri::from_file_path(&model).unwrap();
    let model_text = std::fs::read_to_string(&model).unwrap();
    let model_table = symbols::build_symbol_table(&model_text);
    let namespace = index.namespace_of(&model);
    let at_post = request(
        &model_uri,
        &model_text,
        lsp_types::Position::new(0, 7),
        false,
        model_table.as_ref(),
    );
    let found = references::find_references(&at_post, Some((&index, namespace)), || {
        texts(&HashMap::new())
    })
    .unwrap();
    assert_eq!(
        files_of(&found, &root),
        [
            "app/controllers/posts_controller.sl",
            "app/models/featured_post.sl",
            "app/models/post.sl",
            "app/models/post.sl",
            "tests/post_spec.sl",
        ]
    );

    // An open buffer is searched as the editor has it, not as the disk does.
    let spec = root.join("tests/post_spec.sl");
    let open = HashMap::from([(spec.clone(), "a = Post.recent\nb = Post.all\n".to_string())]);
    let found =
        references::find_references(&at_post, Some((&index, namespace)), || texts(&open)).unwrap();
    assert_eq!(
        files_of(&found, &root)
            .iter()
            .filter(|f| *f == "tests/post_spec.sl")
            .count(),
        2
    );

    // A helper renamed from its `def` reaches the view's code, not its markup
    // or its comment tag.
    let helper = root.join("app/helpers/format.sl");
    let helper_uri = Uri::from_file_path(&helper).unwrap();
    let helper_text = std::fs::read_to_string(&helper).unwrap();
    let table = symbols::build_symbol_table(&helper_text);
    let at_def = request(
        &helper_uri,
        &helper_text,
        lsp_types::Position::new(1, 6),
        false,
        table.as_ref(),
    );
    let edit = rename::rename_symbol(
        &at_def,
        "money",
        Some((&index, index.namespace_of(&helper))),
        || texts(&HashMap::new()),
    )
    .unwrap()
    .unwrap();
    let changes = edit.changes.unwrap();
    let view_uri = Uri::from_file_path(root.join("app/views/posts/show.html.slv")).unwrap();
    assert_eq!(changes[&helper_uri].len(), 1);
    assert_eq!(changes[&view_uri].len(), 1);
    assert_eq!(
        changes[&view_uri][0].range.start,
        lsp_types::Position::new(1, 4)
    );

    // From the view, the hover shows the comment above the helper's `def`.
    let view_text = std::fs::read_to_string(root.join("app/views/posts/show.html.slv")).unwrap();
    let hovered = hover::get_hover(
        &view_text,
        lsp_types::Position::new(1, 6),
        None,
        Some((&index, None)),
    )
    .unwrap();
    let lsp_types::HoverContents::Array(items) = hovered.contents else {
        panic!();
    };
    assert!(
        matches!(&items[0], lsp_types::MarkedString::String(t) if t.contains("Cents to a price.")),
        "{items:?}"
    );

    // A member only one class declares renames everywhere; once two classes
    // declare it, `x.summary` cannot be told apart and the rename is refused.
    let at_summary = request(
        &model_uri,
        &model_text,
        lsp_types::Position::new(5, 7),
        false,
        model_table.as_ref(),
    );
    assert!(
        rename::rename_symbol(&at_summary, "abstract", Some((&index, namespace)), || {
            texts(&HashMap::new())
        })
        .unwrap()
        .is_some()
    );
    write(
        &root,
        "app/models/page.sl",
        "class Page\n  def summary\n    1\n  end\nend\n",
    );
    let index = ProjectIndex::scan(&root);
    let refused = rename::rename_symbol(&at_summary, "abstract", Some((&index, namespace)), || {
        references::project_texts(index.source_files(), &HashMap::new())
    });
    assert_eq!(refused, Err(rename::Refusal::AmbiguousMember(2)));

    // A local stays in its document.
    let source = "def show(id)\n  post = Post.find(id)\n  post\nend\n";
    let table = symbols::build_symbol_table(source);
    let controller_uri =
        Uri::from_file_path(root.join("app/controllers/posts_controller.sl")).unwrap();
    let at_local = request(
        &controller_uri,
        source,
        lsp_types::Position::new(2, 3),
        false,
        table.as_ref(),
    );
    let found =
        references::find_references(&at_local, Some((&index, Some(Namespace::Global))), || {
            panic!("a local must not read the project")
        })
        .unwrap();
    assert_eq!(found.len(), 2);
}

/// The whole loop over the wire: the workspace root from `initialize`, the
/// watched-files registration, a definition into another file, and the index
/// following a deletion reported by the client.
#[tokio::test]
async fn the_index_follows_the_workspace_over_lsp() {
    let (_dir, root) = project();
    let (service, socket) = LspService::new(Backend::new);
    let (client, server) = tokio::io::duplex(1 << 16);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(
        tower_lsp_server::Server::new(input, output, socket)
            .concurrency_level(1)
            .serve(service),
    );
    let mut client = BufStream::new(client);
    let root_uri = Uri::from_file_path(&root).unwrap();

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "capabilities": {"workspace": {"didChangeWatchedFiles": {"dynamicRegistration": true}}},
                "workspaceFolders": [{"uri": root_uri.as_str(), "name": "site"}]
            }
        }),
    )
    .await;
    let initialized = receive(&mut client).await;
    let capabilities = &initialized["result"]["capabilities"];
    assert_eq!(
        capabilities["completionProvider"]["triggerCharacters"],
        json!([".", "@"])
    );
    assert_eq!(capabilities["textDocumentSync"]["save"], true);

    send(
        &mut client,
        json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
    )
    .await;
    assert_eq!(receive(&mut client).await["method"], "window/logMessage");
    let registration = receive(&mut client).await;
    assert_eq!(registration["method"], "client/registerCapability");
    assert_eq!(
        registration["params"]["registrations"][0]["method"],
        "workspace/didChangeWatchedFiles"
    );
    send(
        &mut client,
        json!({"jsonrpc": "2.0", "id": registration["id"], "result": null}),
    )
    .await;

    let controller = root.join("app/controllers/posts_controller.sl");
    let uri = Uri::from_file_path(&controller).unwrap();
    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": {"textDocument": {
                "uri": uri.as_str(), "languageId": "soli", "version": 1,
                "text": std::fs::read_to_string(&controller).unwrap()
            }}
        }),
    )
    .await;
    assert_eq!(
        receive(&mut client).await["method"],
        "textDocument/publishDiagnostics"
    );

    let definition = json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition",
        "params": {"textDocument": {"uri": uri.as_str()}, "position": {"line": 3, "character": 13}}
    });
    send(&mut client, definition.clone()).await;
    let found = receive(&mut client).await;
    let post = root.join("app/models/post.sl");
    assert_eq!(
        found["result"]["uri"],
        Uri::from_file_path(&post).unwrap().as_str()
    );

    std::fs::remove_file(&post).unwrap();
    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles",
            "params": {"changes": [{"uri": Uri::from_file_path(&post).unwrap().as_str(), "type": 3}]}
        }),
    )
    .await;
    send(&mut client, definition).await;
    let found = receive(&mut client).await;
    assert_eq!(found["id"], 2);
    assert_eq!(found["result"], Value::Null);

    send(
        &mut client,
        json!({"jsonrpc": "2.0", "id": 5, "method": "shutdown"}),
    )
    .await;
    assert_eq!(receive(&mut client).await["id"], 5);
    send(&mut client, json!({"jsonrpc": "2.0", "method": "exit"})).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("LSP server did not exit")
        .unwrap();
}
