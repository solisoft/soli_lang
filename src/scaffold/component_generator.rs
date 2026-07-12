//! `soli generate component <name>` — scaffold a view component into
//! `app/views/components/<name>.html.slv`.
//!
//! ```text
//! soli generate component stats_card
//!   -> app/views/components/stats_card.html.slv
//!
//! soli generate component cards/stat      # subdirectory
//!   -> app/views/components/cards/stat.html.slv
//! ```

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::write_file;
use crate::scaffold::utils::to_snake_case;

/// Generate a view component into the app at `folder`.
pub fn create_component(folder: &str, name: &str) -> Result<(), String> {
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

    println!("\nGenerated component:");
    println!("  app/views/components/{file_rel}");
    println!("\nRender it from a view:");
    println!("  <%- component(\"{rel}\", {{ }}) %>");
    println!(
        "  <%- component \"{rel}\" do |c| %> ... <%- end %>   # block form (default + named slots)"
    );
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
        create_component(app.to_str().unwrap(), "StatsCard").unwrap();
        let file = app.join("app/views/components/stats_card.html.slv");
        assert!(file.exists());
        assert!(fs::read_to_string(&file).unwrap().contains("stats_card"));
    }

    #[test]
    fn creates_subdirectory_component() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path();
        fs::create_dir_all(app.join("app")).unwrap();
        create_component(app.to_str().unwrap(), "cards/Stat").unwrap();
        assert!(app
            .join("app/views/components/cards/stat.html.slv")
            .exists());
    }

    #[test]
    fn rejects_non_app_dir() {
        let dir = tempfile::tempdir().unwrap();
        let err = create_component(dir.path().to_str().unwrap(), "x").unwrap_err();
        assert!(err.contains("does not look like a Soli app"));
    }
}
