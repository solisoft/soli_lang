//! ERB-style template engine for Soli MVC.
//!
//! Supports:
//! - `<%= expr %>` - HTML-escaped output
//! - `<%- expr %>` - Raw/unescaped output (no HTML escaping)
//! - `<% if/for/end %>` - Control flow
//! - `<%= yield %>` - Layout content insertion point
//! - `<%= render 'partial' %>` - Partial rendering

pub mod layout;
pub mod parser;
pub mod renderer;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use indexmap::IndexMap;

use crate::interpreter::value::{HashKey, Value};
use parser::parse_template;
use renderer::render_nodes_with_path;
use std::rc::Rc;

/// A cached template with its parsed AST and modification time.
#[derive(Debug, Clone)]
struct CachedTemplate {
    nodes: Rc<Vec<parser::TemplateNode>>,
    modified: SystemTime,
}

/// Maximum size for path cache to prevent unbounded memory growth.
const PATH_CACHE_MAX_SIZE: usize = 1000;

/// Maximum size for template cache to prevent unbounded memory growth.
const TEMPLATE_CACHE_MAX_SIZE: usize = 500;

/// Template cache that stores parsed templates and tracks file changes.
pub struct TemplateCache {
    /// Base directory for views (e.g., app/views)
    views_dir: PathBuf,
    /// Cached parsed templates (path -> nodes)
    cache: RefCell<HashMap<String, CachedTemplate>>,
    /// Cached path resolutions (template_name -> resolved_path)
    path_cache: RefCell<HashMap<String, PathBuf>>,
}

impl TemplateCache {
    /// Create a new template cache for the given views directory.
    pub fn new(views_dir: impl Into<PathBuf>) -> Self {
        Self {
            views_dir: views_dir.into(),
            cache: RefCell::new(HashMap::new()),
            path_cache: RefCell::new(HashMap::new()),
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
        // Get the template file path
        let template_path = self.resolve_template_path(template_name)?;
        let template_path_str = template_path.to_string_lossy().to_string();

        // Get template from cache
        let nodes = self.get_or_load_template(&template_path)?;

        // Create partial renderer closure
        let partial_renderer =
            |name: &str, ctx: &Value| -> Result<String, String> { self.render_partial(name, ctx) };

        // Render the template content with path for error reporting
        let content = render_nodes_with_path(
            &nodes,
            data,
            Some(&partial_renderer),
            Some(&template_path_str),
        )?;

        // Apply layout if specified
        match layout {
            Some(Some(layout_name)) => {
                // Use specified layout
                self.render_with_named_layout(&content, data, layout_name, &partial_renderer)
            }
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
                    self.render_with_named_layout(&content, data, "application", &partial_renderer)
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
        let template_path_str = template_path.to_string_lossy().to_string();
        let nodes = self.get_or_load_template(&template_path)?;

        let partial_renderer =
            |n: &str, ctx: &Value| -> Result<String, String> { self.render_partial(n, ctx) };

        render_nodes_with_path(
            &nodes,
            data,
            Some(&partial_renderer),
            Some(&template_path_str),
        )
    }

    /// Render content with a named layout.
    fn render_with_named_layout(
        &self,
        content: &str,
        data: &Value,
        layout_name: &str,
        partial_renderer: &dyn Fn(&str, &Value) -> Result<String, String>,
    ) -> Result<String, String> {
        // Strip "layouts/" prefix if present to avoid double prefixing
        let layout_name = layout_name.trim_start_matches("layouts/");
        let layout_name = layout_name.trim_start_matches("layouts");

        // Use the cache for layouts too (layouts/name.html.erb)
        let layout_template = format!("layouts/{}", layout_name);

        match self.resolve_template_path(&layout_template) {
            Ok(layout_path) => {
                let layout_path_str = layout_path.to_string_lossy().to_string();
                let layout_nodes = self.get_or_load_template(&layout_path)?;
                layout::render_layout_nodes_with_path(
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
    fn resolve_template_path(&self, name: &str) -> Result<PathBuf, String> {
        // Check path cache first
        {
            let path_cache = self.path_cache.borrow();
            if let Some(path) = path_cache.get(name) {
                return Ok(path.clone());
            }
        }

        // Cache miss - do file system lookup
        let resolved = self.do_resolve_template_path(name)?;

        // Cache the result (with eviction if cache is too large)
        {
            let mut path_cache = self.path_cache.borrow_mut();
            if path_cache.len() >= PATH_CACHE_MAX_SIZE {
                path_cache.clear();
            }
            path_cache.insert(name.to_string(), resolved.clone());
        }

        Ok(resolved)
    }

    /// Actually resolve the template path (file system lookup).
    fn do_resolve_template_path(&self, name: &str) -> Result<PathBuf, String> {
        // Try with .html.erb extension
        let path = self.views_dir.join(format!("{}.html.erb", name));
        if path.exists() {
            return Ok(path);
        }

        // Try with .erb extension
        let path = self.views_dir.join(format!("{}.erb", name));
        if path.exists() {
            return Ok(path);
        }

        // Try as-is (already has extension)
        let path = self.views_dir.join(name);
        if path.exists() {
            return Ok(path);
        }

        Err(format!(
            "Template '{}' not found in {}",
            name,
            self.views_dir.display()
        ))
    }

    /// Get a template from cache or load and parse it.
    fn get_or_load_template(&self, path: &Path) -> Result<Rc<Vec<parser::TemplateNode>>, String> {
        let path_str = path.to_string_lossy().to_string();

        // Check cache first (fast path - no file I/O)
        {
            let cache = self.cache.borrow();
            if let Some(cached) = cache.get(&path_str) {
                return Ok(cached.nodes.clone()); // Rc clone is O(1)
            }
        }

        // Cache miss - load and parse template
        let source = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read template '{}': {}", path.display(), e))?;

        let modified = fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let nodes = Rc::new(parse_template(&source)?);

        // Update cache (with eviction if cache is too large)
        {
            let mut cache = self.cache.borrow_mut();
            if cache.len() >= TEMPLATE_CACHE_MAX_SIZE {
                cache.clear();
            }
            cache.insert(
                path_str,
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
        self.cache.borrow_mut().clear();
        self.path_cache.borrow_mut().clear();
    }

    /// Check if any tracked templates have changed.
    pub fn has_changes(&self) -> bool {
        let cache = self.cache.borrow();
        for (path_str, cached) in cache.iter() {
            let path = Path::new(path_str);
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
    // Inject live reload script if enabled
    let body = if crate::serve::live_reload::is_live_reload_enabled() {
        crate::serve::live_reload::inject_live_reload_script(&body)
    } else {
        body
    };

    let mut headers: IndexMap<HashKey, Value> = IndexMap::new();
    headers.insert(
        HashKey::String("Content-Type".to_string()),
        Value::String("text/html; charset=utf-8".to_string()),
    );

    let mut result: IndexMap<HashKey, Value> = IndexMap::new();
    result.insert(HashKey::String("status".to_string()), Value::Int(status));
    result.insert(
        HashKey::String("headers".to_string()),
        Value::Hash(Rc::new(RefCell::new(headers))),
    );
    result.insert(HashKey::String("body".to_string()), Value::String(body));

    Value::Hash(Rc::new(RefCell::new(result)))
}

// Integration tests requiring filesystem would go here.
// These are better tested via integration tests or with tempfile dev-dependency.
