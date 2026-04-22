//! ERB-style template engine for Soli MVC.
//!
//! Supports:
//! - `<%= expr %>` - HTML-escaped output
//! - `<%- expr %>` - Raw/unescaped output (no HTML escaping)
//! - `<% if/for/end %>` - Control flow
//! - `<%= yield %>` - Layout content insertion point
//! - `<%= render 'partial' %>` - Partial rendering

pub mod core_eval;
pub mod helpers;
pub mod layout;
pub mod parser;
pub mod renderer;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use crate::interpreter::value::Value;
use parser::parse_template;
use renderer::{render_nodes_with_path, render_with_interpreter};
use std::rc::Rc;

/// A cached template with its parsed AST and modification time.
#[derive(Debug, Clone)]
struct CachedTemplate {
    nodes: Arc<Vec<parser::TemplateNode>>,
    modified: SystemTime,
}

/// Maximum size for path cache to prevent unbounded memory growth.
const PATH_CACHE_MAX_SIZE: usize = 1000;

/// Maximum size for template cache to prevent unbounded memory growth.
const TEMPLATE_CACHE_MAX_SIZE: usize = 500;

/// Template cache that stores parsed templates and tracks file changes.
///
/// Shared across worker threads via `Arc<TemplateCache>`; a template parsed by
/// any worker is visible to the others, avoiding per-worker reparsing.
pub struct TemplateCache {
    /// Base directory for views (e.g., app/views)
    views_dir: PathBuf,
    /// Cached parsed templates (PathBuf -> nodes).
    cache: RwLock<HashMap<PathBuf, CachedTemplate>>,
    /// Cached path resolutions (template_name -> resolved_path).
    /// Arc so cache hits are pointer increments, not heap clones.
    path_cache: RwLock<HashMap<String, Arc<PathBuf>>>,
}

impl TemplateCache {
    /// Create a new template cache for the given views directory.
    pub fn new(views_dir: impl Into<PathBuf>) -> Self {
        Self {
            views_dir: views_dir.into(),
            cache: RwLock::new(HashMap::new()),
            path_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Get the views directory path.
    pub fn views_dir(&self) -> &Path {
        &self.views_dir
    }

    /// Render a template with the given data.
    ///
    /// # Arguments
    /// * `template_name` - Template path relative to views dir (e.g., "users/index")
    /// * `data` - Data context for the template
    /// * `layout` - Optional layout name (defaults to "application"), or None to skip layout
    pub fn render(
        &self,
        template_name: &str,
        data: &Value,
        layout: Option<Option<&str>>,
    ) -> Result<String, String> {
        // Treat undefined variable lookups as Null throughout template
        // rendering — controllers commonly omit optional locals like
        // `flash`, and the scaffolded layout references them directly.
        let _guard = crate::interpreter::executor::enter_template_lenient_vars();

        // Get the template file path
        let template_path = self.resolve_template_path(template_name)?;

        // Get template from cache
        let nodes = self.get_or_load_template(&template_path)?;

        // Create partial renderer closure
        let partial_renderer =
            |name: &str, ctx: &Value| -> Result<String, String> { self.render_partial(name, ctx) };

        // Create ONE interpreter for both view and layout rendering
        let mut interpreter = core_eval::create_template_interpreter(data);

        // Render the template content with shared interpreter
        let template_path_str = template_path.to_string_lossy();
        let content = render_with_interpreter(
            &mut interpreter,
            &nodes,
            data,
            Some(&partial_renderer),
            Some(&template_path_str),
        )?;

        // If the template is a markdown file, convert to HTML
        let content = if is_markdown_template(&template_path) {
            markdown_to_html(&content)
        } else {
            content
        };

        // Apply layout if specified, reusing the same interpreter
        match layout {
            Some(Some(layout_name)) => self.render_layout_with_shared_interpreter(
                &mut interpreter,
                &content,
                data,
                layout_name,
                &partial_renderer,
            ),
            Some(None) => {
                // Explicitly no layout (layout: false)
                Ok(content)
            }
            None => {
                // Default layout: application
                // But if the template name starts with underscore (partial), skip layout
                if template_name.contains("/_") || template_name.starts_with('_') {
                    Ok(content)
                } else {
                    self.render_layout_with_shared_interpreter(
                        &mut interpreter,
                        &content,
                        data,
                        "application",
                        &partial_renderer,
                    )
                }
            }
        }
    }

    /// Render a partial template (no layout).
    pub fn render_partial(&self, name: &str, data: &Value) -> Result<String, String> {
        // Partials start with underscore
        let partial_name = if name.contains('/') {
            // e.g., "users/card" -> "users/_card"
            let parts: Vec<&str> = name.rsplitn(2, '/').collect();
            if parts.len() == 2 {
                format!("{}/_{}", parts[1], parts[0])
            } else {
                format!("_{}", name)
            }
        } else {
            format!("_{}", name)
        };

        let template_path = self.resolve_template_path(&partial_name)?;
        let nodes = self.get_or_load_template(&template_path)?;

        let partial_renderer =
            |n: &str, ctx: &Value| -> Result<String, String> { self.render_partial(n, ctx) };

        let template_path_str = template_path.to_string_lossy();
        let content = render_nodes_with_path(
            &nodes,
            data,
            Some(&partial_renderer),
            Some(&template_path_str),
        )?;

        // If the partial is a markdown file, convert to HTML
        if is_markdown_template(&template_path) {
            Ok(markdown_to_html(&content))
        } else {
            Ok(content)
        }
    }

    /// Render content with a named layout, reusing an existing interpreter.
    fn render_layout_with_shared_interpreter(
        &self,
        interpreter: &mut crate::interpreter::executor::Interpreter,
        content: &str,
        data: &Value,
        layout_name: &str,
        partial_renderer: &dyn Fn(&str, &Value) -> Result<String, String>,
    ) -> Result<String, String> {
        // Strip "layouts/" prefix if present to avoid double prefixing
        let layout_name = layout_name.trim_start_matches("layouts/");
        let layout_name = layout_name.trim_start_matches("layouts");

        // Fast path: avoid format! allocation for the most common layout name
        let layout_template;
        let layout_key = if layout_name == "application" {
            "layouts/application"
        } else {
            layout_template = format!("layouts/{}", layout_name);
            &layout_template
        };

        match self.resolve_template_path(layout_key) {
            Ok(layout_path) => {
                let layout_nodes = self.get_or_load_template(&layout_path)?;
                let layout_path_str = layout_path.to_string_lossy();
                layout::render_layout_with_interpreter(
                    interpreter,
                    &layout_nodes,
                    content,
                    data,
                    Some(partial_renderer),
                    Some(&layout_path_str),
                )
            }
            Err(_) => {
                // No layout file, return content as-is
                Ok(content.to_string())
            }
        }
    }

    /// Resolve a template name to a file path (cached).
    /// Returns Arc<PathBuf> so cache hits are pointer increments, not heap clones.
    fn resolve_template_path(&self, name: &str) -> Result<Arc<PathBuf>, String> {
        // Check path cache first
        if let Some(path) = self
            .path_cache
            .read()
            .ok()
            .and_then(|c| c.get(name).cloned())
        {
            return Ok(path);
        }

        // Cache miss - do file system lookup
        let resolved = Arc::new(self.do_resolve_template_path(name)?);

        // Cache the result (with eviction if cache is too large)
        if let Ok(mut path_cache) = self.path_cache.write() {
            if path_cache.len() >= PATH_CACHE_MAX_SIZE {
                path_cache.clear();
            }
            path_cache.insert(name.to_string(), Arc::clone(&resolved));
        }

        Ok(resolved)
    }

    /// Actually resolve the template path (file system lookup).
    fn do_resolve_template_path(&self, name: &str) -> Result<PathBuf, String> {
        // Try main views directory first
        if let Ok(path) = self.try_resolve_in_dir(&self.views_dir, name) {
            return Ok(path);
        }

        // If template name starts with an engine name, resolve from the engine's views dir.
        // e.g. render("shop/index") → engines/shop/app/views/shop/index.html.slv
        if let Some(slash_pos) = name.find('/') {
            let candidate = &name[..slash_pos];
            if crate::serve::engine_loader::is_engine_name(candidate) {
                if let Some(engine_views_dir) = self
                    .views_dir
                    .parent() // app/views -> app
                    .and_then(|p| p.parent()) // app -> project root
                    .map(|root| {
                        root.join("engines")
                            .join(candidate)
                            .join("app")
                            .join("views")
                    })
                {
                    let view_path = &name[slash_pos + 1..];
                    if let Ok(path) = self.try_resolve_in_dir(&engine_views_dir, view_path) {
                        return Ok(path);
                    }
                    if let Ok(path) = self.try_resolve_in_dir(&engine_views_dir, name) {
                        return Ok(path);
                    }
                }
            }
        }

        Err(format!(
            "Template '{}' not found in {}",
            name,
            self.views_dir.display()
        ))
    }

    /// Try to resolve a template path in a specific directory with various extensions.
    fn try_resolve_in_dir(&self, dir: &Path, name: &str) -> Result<PathBuf, String> {
        let extensions = [
            ".html.slv",
            ".slv",
            ".html.md",
            ".md",
            ".html.erb",
            ".erb",
            "",
        ];

        for ext in extensions {
            let path = if ext.is_empty() {
                dir.join(name)
            } else {
                dir.join(format!("{}{}", name, ext))
            };
            if path.exists() {
                return Ok(path);
            }
        }

        Err(format!("Template not found in {}", dir.display()))
    }

    /// Get a template from cache or load and parse it.
    fn get_or_load_template(&self, path: &Path) -> Result<Arc<Vec<parser::TemplateNode>>, String> {
        // Check cache first (fast path - shared read lock)
        if let Some(nodes) = self
            .cache
            .read()
            .ok()
            .and_then(|c| c.get(path).map(|entry| entry.nodes.clone()))
        {
            return Ok(nodes);
        }

        // Cache miss - load and parse template
        let source = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read template '{}': {}", path.display(), e))?;

        let modified = fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let nodes = Arc::new(parse_template(&source)?);

        // Update cache (with eviction if cache is too large)
        if let Ok(mut cache) = self.cache.write() {
            if cache.len() >= TEMPLATE_CACHE_MAX_SIZE {
                cache.clear();
            }
            cache.insert(
                path.to_path_buf(),
                CachedTemplate {
                    nodes: nodes.clone(),
                    modified,
                },
            );
        }

        Ok(nodes)
    }

    /// Clear the template cache (useful for hot reload).
    pub fn clear(&self) {
        if let Ok(mut c) = self.cache.write() {
            c.clear();
        }
        if let Ok(mut c) = self.path_cache.write() {
            c.clear();
        }
    }

    /// Check if any tracked templates have changed.
    pub fn has_changes(&self) -> bool {
        let Ok(cache) = self.cache.read() else {
            return false;
        };
        for (path, cached) in cache.iter() {
            if let Ok(metadata) = fs::metadata(path) {
                if let Ok(modified) = metadata.modified() {
                    if modified != cached.modified {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Create a response hash for rendered HTML content.
pub fn html_response(body: String, status: i64) -> Value {
    use crate::interpreter::value::{HashKey, HashPairs};
    use ahash::RandomState as AHasher;

    // Inject live reload script if enabled
    let body = if crate::serve::live_reload::is_live_reload_enabled() {
        crate::serve::live_reload::inject_live_reload_script(&body)
    } else {
        body
    };

    let mut headers: HashPairs = HashPairs::with_capacity_and_hasher(1, AHasher::default());
    headers.insert(
        HashKey::String("Content-Type".to_string()),
        Value::String("text/html; charset=utf-8".to_string()),
    );

    let mut result: HashPairs = HashPairs::with_capacity_and_hasher(3, AHasher::default());
    result.insert(HashKey::String("status".to_string()), Value::Int(status));
    result.insert(
        HashKey::String("headers".to_string()),
        Value::Hash(Rc::new(RefCell::new(headers))),
    );
    result.insert(HashKey::String("body".to_string()), Value::String(body));

    Value::Hash(Rc::new(RefCell::new(result)))
}

/// Check if a template path is a markdown file.
fn is_markdown_template(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.ends_with(".md")
}

/// Convert markdown text to HTML using pulldown-cmark.
pub fn markdown_to_html(markdown: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(markdown, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::{HashKey, HashPairs};
    use std::fs;

    #[test]
    fn test_is_markdown_template() {
        assert!(is_markdown_template(Path::new("app/views/docs/index.md")));
        assert!(is_markdown_template(Path::new(
            "app/views/docs/index.html.md"
        )));
        assert!(!is_markdown_template(Path::new(
            "app/views/docs/index.html.slv"
        )));
        assert!(!is_markdown_template(Path::new("app/views/docs/index.slv")));
        assert!(!is_markdown_template(Path::new("app/views/docs/index.erb")));
    }

    #[test]
    fn test_markdown_to_html_basic() {
        let md = "# Hello World\n\nThis is **bold** and *italic*.";
        let html = markdown_to_html(md);
        assert!(html.contains("<h1>Hello World</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
    }

    #[test]
    fn test_markdown_to_html_list() {
        let md = "- Item 1\n- Item 2\n- Item 3";
        let html = markdown_to_html(md);
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>Item 1</li>"));
        assert!(html.contains("<li>Item 2</li>"));
        assert!(html.contains("<li>Item 3</li>"));
    }

    #[test]
    fn test_markdown_to_html_table() {
        let md = "| Col A | Col B |\n|-------|-------|\n| 1     | 2     |";
        let html = markdown_to_html(md);
        assert!(html.contains("<table>"));
        assert!(html.contains("<th>Col A</th>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn test_markdown_to_html_strikethrough() {
        let md = "This is ~~deleted~~ text.";
        let html = markdown_to_html(md);
        assert!(html.contains("<del>deleted</del>"));
    }

    #[test]
    fn test_markdown_to_html_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let html = markdown_to_html(md);
        assert!(html.contains("<code"));
        assert!(html.contains("fn main() {}"));
    }

    #[test]
    fn test_resolve_template_path_md() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        // Create a .html.md file
        fs::write(views.join("page.html.md"), "# Page").unwrap();

        let cache = TemplateCache::new(&views);
        let resolved = cache.resolve_template_path("page").unwrap();
        assert!(resolved.to_string_lossy().ends_with(".html.md"));
    }

    #[test]
    fn test_resolve_template_path_md_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        // Create a .md file (no .html prefix)
        fs::write(views.join("page.md"), "# Page").unwrap();

        let cache = TemplateCache::new(&views);
        let resolved = cache.resolve_template_path("page").unwrap();
        assert!(resolved.to_string_lossy().ends_with(".md"));
    }

    #[test]
    fn test_resolve_template_path_slv_preferred_over_md() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        // Create both .html.slv and .html.md — .slv should win
        fs::write(views.join("page.html.slv"), "<h1>SLV</h1>").unwrap();
        fs::write(views.join("page.html.md"), "# MD").unwrap();

        let cache = TemplateCache::new(&views);
        let resolved = cache.resolve_template_path("page").unwrap();
        assert!(resolved.to_string_lossy().ends_with(".html.slv"));
    }

    #[test]
    fn test_render_md_template() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        fs::write(views.join("test.md"), "# Hello\n\nThis is **bold**.").unwrap();

        let cache = TemplateCache::new(&views);
        let result = cache.render("test", &Value::Null, Some(None)).unwrap();
        assert!(result.contains("<h1>Hello</h1>"));
        assert!(result.contains("<strong>bold</strong>"));
    }

    #[test]
    fn test_render_md_template_with_template_tags() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        fs::write(views.join("greeting.md"), "# Hello <%= name %>").unwrap();

        let mut data = HashPairs::default();
        data.insert(
            HashKey::String("name".to_string()),
            Value::String("World".to_string()),
        );
        let data = Value::Hash(Rc::new(RefCell::new(data)));

        let cache = TemplateCache::new(&views);
        let result = cache.render("greeting", &data, Some(None)).unwrap();
        assert!(result.contains("<h1>Hello World</h1>"));
    }

    /// Helper to register a fake mounted engine for template resolution tests.
    fn register_test_engine(name: &str, path: &Path, mounted_at: &str) {
        use crate::serve::engine_loader::{mount_engines, EngineConfig, EngineMount};

        // Create engine manifest so discover_engines finds it
        let engine_dir = path.join("engines").join(name);
        fs::create_dir_all(&engine_dir).unwrap();
        fs::write(
            engine_dir.join("engine.sl"),
            format!(
                "engine \"{}\" {{\n    version: \"1.0.0\",\n    dependencies: []\n}}",
                name
            ),
        )
        .unwrap();

        let config = EngineConfig {
            engines: vec![EngineMount {
                name: name.to_string(),
                mounted_at: mounted_at.to_string(),
            }],
        };
        mount_engines(path, &config).unwrap();
    }

    #[test]
    fn test_resolve_engine_view() {
        // Simulate: project/app/views is the views_dir
        // Engine view at: project/engines/shop/app/views/shop/index.html.slv
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let views = root.join("app/views");
        fs::create_dir_all(&views).unwrap();

        let engine_views = root.join("engines/shop/app/views/shop");
        fs::create_dir_all(&engine_views).unwrap();
        fs::write(engine_views.join("index.html.slv"), "<h1>Shop</h1>").unwrap();

        register_test_engine("shop", root, "/shop");

        let cache = TemplateCache::new(&views);
        // render("shop/index") should find the engine view
        let result = cache.resolve_template_path("shop/index");
        assert!(result.is_ok());
        assert!(result
            .unwrap()
            .to_string_lossy()
            .contains("engines/shop/app/views"));

        crate::serve::engine_loader::reset_engine_context();
    }

    #[test]
    fn test_resolve_main_view_preferred_over_engine() {
        // If a view exists in both main views and engine views, main wins
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let views = root.join("app/views/shop");
        fs::create_dir_all(&views).unwrap();
        fs::write(views.join("index.html.slv"), "<h1>Main Shop</h1>").unwrap();

        let engine_views = root.join("engines/shop/app/views/shop");
        fs::create_dir_all(&engine_views).unwrap();
        fs::write(engine_views.join("index.html.slv"), "<h1>Engine Shop</h1>").unwrap();

        register_test_engine("shop", root, "/shop");

        let cache = TemplateCache::new(root.join("app/views"));
        let result = cache.resolve_template_path("shop/index").unwrap();
        // Main views dir should win
        assert!(result.to_string_lossy().contains("app/views/shop"));
        assert!(!result.to_string_lossy().contains("engines"));

        crate::serve::engine_loader::reset_engine_context();
    }

    #[test]
    fn test_render_md_partial() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        fs::write(views.join("_note.md"), "**Important:** remember this.").unwrap();

        let cache = TemplateCache::new(&views);
        let result = cache.render_partial("note", &Value::Null).unwrap();
        assert!(result.contains("<strong>Important:</strong>"));
    }
}
