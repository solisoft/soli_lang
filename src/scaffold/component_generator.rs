//! `soli generate component <name>` — scaffold a view component into
//! `app/views/components/<name>.html.slv`.
//!
//! ```text
//! soli generate component stats_card
//!   -> app/views/components/stats_card.html.slv
//!
//! soli generate component cards/stat      # subdirectory
//!   -> app/views/components/cards/stat.html.slv
//!
//! soli generate component stats_card --class
//!   -> app/views/components/stats_card.html.slv
//!   -> app/components/stats_card_component.sl   (class StatsCardComponent)
//! ```

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::write_file;
use crate::scaffold::utils::to_snake_case;

/// Generate a view component into the app at `folder`.
pub fn create_component(folder: &str, name: &str, with_class: bool) -> Result<(), String> {
    let app_path = Path::new(folder);
    if !app_path.join("app").is_dir() {
        return Err(format!(
            "'{}' does not look like a Soli app (no app/ directory). \
             Run this inside a project created with `soli new`.",
            folder
        ));
    }

    // Accept "stats_card", "StatsCard", or "cards/stat" (subdirectory). Each
    // path segment is normalized to snake_case; the subdirectory layout is kept.
    let rel: String = name
        .split('/')
        .filter(|s| !s.is_empty())
        .map(to_snake_case)
        .collect::<Vec<_>>()
        .join("/");
    if rel.is_empty() {
        return Err("component name must not be empty".to_string());
    }

    let file_rel = format!("{rel}.html.slv");
    let file_path = app_path.join("app/views/components").join(&file_rel);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }

    if file_path.exists() {
        println!("  skip (already exists) app/views/components/{file_rel}");
        return Ok(());
    }

    let class_name = rel.replace('/', "-");
    // Starter markup renders fine via the function form (no bare `yield`, which
    // would need a block's default slot). The comment points at props + slots.
    let body = format!(
        "<%# Component \"{rel}\" — read caller data as bare locals (e.g. `title`).\n\
         \x20   Render:  component(\"{rel}\", {{ ... }})   or a collection with \"collection\".\n\
         \x20   Slots:   component \"{rel}\" do ... end   exposes `yield` (default) + named slots. %>\n\
         <div class=\"{class_name}\">\n\
         \x20 <p>Edit app/views/components/{file_rel}</p>\n\
         </div>\n",
    );

    write_file(&file_path, &body)?;

    if with_class {
        write_component_class(app_path, &rel)?;
    }

    println!("\nGenerated component:");
    println!("  app/views/components/{file_rel}");
    println!("\nRender it from a view:");
    println!("  <%- component(\"{rel}\", {{ }}) %>");
    println!(
        "  <%- component \"{rel}\" do |c| %> ... <%- end %>   # block form (default + named slots)"
    );
    Ok(())
}

/// `app/components/<last segment>_component.sl` with `class <Last>Component`.
/// Flat on purpose: the loader reads the directory without recursing, and the
/// class is found by the last path segment of the component name.
fn write_component_class(app_path: &Path, rel: &str) -> Result<(), String> {
    let last = rel.rsplit('/').next().unwrap_or(rel);
    let class_name: String = last
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<String>()
        + "Component";
    let file_rel = format!("{last}_component.sl");
    let path = app_path.join("app/components").join(&file_rel);
    if path.exists() {
        println!("  skip (already exists) app/components/{file_rel}");
        return Ok(());
    }
    let body = format!(
        "# Paired with app/views/components/{rel}.html.slv. The props passed to\n\
         # component(\"{rel}\", {{ ... }}) become fields; the template reaches the\n\
         # instance as `this` (e.g. `<%= this.heading() %>`).\n\
         class {class_name}\n\
         \x20 title: String\n\
         \n\
         \x20 # Runs before the template renders: derive what the view needs.\n\
         \x20 def before_render\n\
         \x20   @title = \"Untitled\" if @title.blank?\n\
         \x20 end\n\
         \n\
         \x20 def heading\n\
         \x20   @title.upcase\n\
         \x20 end\n\
         end\n"
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    write_file(&path, &body)?;
    println!("  app/components/{file_rel}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_component_file_snake_cased() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path();
        fs::create_dir_all(app.join("app/views")).unwrap();
        create_component(app.to_str().unwrap(), "StatsCard", false).unwrap();
        let file = app.join("app/views/components/stats_card.html.slv");
        assert!(file.exists());
        assert!(fs::read_to_string(&file).unwrap().contains("stats_card"));
    }

    #[test]
    fn creates_subdirectory_component() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path();
        fs::create_dir_all(app.join("app")).unwrap();
        create_component(app.to_str().unwrap(), "cards/Stat", false).unwrap();
        assert!(app
            .join("app/views/components/cards/stat.html.slv")
            .exists());
    }

    #[test]
    fn with_class_writes_the_paired_component_class() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path();
        fs::create_dir_all(app.join("app")).unwrap();
        create_component(app.to_str().unwrap(), "cards/stats_card", true).unwrap();
        let class = fs::read_to_string(app.join("app/components/stats_card_component.sl")).unwrap();
        assert!(class.contains("class StatsCardComponent"), "{class}");
        assert!(class.contains("def before_render"), "{class}");
        // The generated class must parse.
        let tokens = crate::lexer::Scanner::new(&class).scan_tokens().unwrap();
        crate::parser::Parser::new(tokens)
            .parse()
            .expect("generated class parses");
    }

    #[test]
    fn rejects_non_app_dir() {
        let dir = tempfile::tempdir().unwrap();
        let err = create_component(dir.path().to_str().unwrap(), "x", false).unwrap_err();
        assert!(err.contains("does not look like a Soli app"));
    }
}
