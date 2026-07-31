//! The generated page for a folder.
//!
//! Laid out as the index of a book: name, leader dots, then what you actually
//! want to know about the entry. Folders first. No icons — the extension and
//! the trailing slash already say what kind of thing each row is, and a column
//! of coloured glyphs would say it a second time, louder.
//!
//! When the folder has a `README.md`, it is rendered above the listing, so a
//! documented directory reads as a page with its contents underneath rather
//! than as a bare file dump.

use std::path::Path;

use hyper::{Response, StatusCode};

use crate::template::renderer::html_escape;

use super::{html_page, human_age, human_size, markdown, readme_in, shell, tree, ResponseBody};

pub(crate) fn page(root: &Path, dir: &Path, rel: &str, dev_mode: bool) -> Response<ResponseBody> {
    let mut entries = read_entries(dir);
    tree::sort(&mut entries);

    let dirs = entries.iter().filter(|e| e.is_dir).count();
    let files = entries.len() - dirs;
    let bytes: u64 = entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();

    let mut body = String::new();

    // The folder's own words come before its contents.
    if let Some(readme) = readme_in(dir) {
        if let Ok(source) = std::fs::read_to_string(&readme) {
            body.push_str(&format!(
                "<article class=\"prose readme\">{}</article>",
                markdown::to_html(&source)
            ));
        }
    }

    if entries.is_empty() {
        body.push_str(
            "<div class=\"state\">\
<p class=\"lead\">This folder is empty.</p>\
<p class=\"dir\">Drop a <code>.md</code> file here and reload.</p>\
</div>",
        );
    } else {
        body.push_str("<div class=\"index\">");
        for entry in &entries {
            let href = format!(
                "{}{}",
                tree::url_path(&entry.rel),
                if entry.is_dir { "/" } else { "" }
            );
            let detail = if entry.is_dir {
                let count = count_entries(&dir.join(&entry.name));
                format!("{} {}", count, if count == 1 { "entry" } else { "entries" })
            } else {
                human_size(entry.size)
            };
            body.push_str(&format!(
                "<a class=\"e\" href=\"{href}\">\
<span class=\"nm\">{name}{slash}</span>\
<span class=\"fill\" aria-hidden=\"true\"></span>\
<span class=\"size\">{detail}</span>\
<span class=\"age\">{age}</span>\
</a>",
                href = href,
                name = html_escape(&entry.name),
                slash = if entry.is_dir {
                    "<span class=\"x\">/</span>"
                } else {
                    ""
                },
                detail = html_escape(&detail),
                age = html_escape(&human_age(entry.modified)),
            ));
        }
        body.push_str("</div>");
    }

    let meta = format!(
        "{} {} · {} {} · {}",
        dirs,
        if dirs == 1 { "folder" } else { "folders" },
        files,
        if files == 1 { "file" } else { "files" },
        human_size(bytes)
    );

    let title = if rel.is_empty() {
        format!("{}/", folder_name(root))
    } else {
        format!("{}/", rel)
    };

    let html = shell::render(
        root,
        shell::Page {
            title,
            current: rel,
            meta,
            body,
            prose: false,
            outline: String::new(),
        },
        dev_mode,
    );
    html_page(html, StatusCode::OK)
}

/// One level of the directory, as tree nodes (so sorting and rendering match
/// the sidebar exactly).
fn read_entries(dir: &Path) -> Vec<tree::Node> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let rel = relative_to_root(&entry.path());
        out.push(tree::Node {
            name,
            rel,
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified: meta.modified().ok(),
            children: Vec::new(),
        });
    }
    out
}

/// Path relative to the served root, `/`-separated — the form links and the
/// sidebar both use.
fn relative_to_root(path: &Path) -> String {
    let Some(root) = super::files_root() else {
        return path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
    };
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(&canonical_root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Visible entries directly inside `dir`.
fn count_entries(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|read| {
            read.flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .count()
        })
        .unwrap_or(0)
}

fn folder_name(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_entries_skips_dotfiles() {
        let dir = std::env::temp_dir().join(format!("soli_index_count_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "a").unwrap();
        std::fs::write(dir.join(".hidden"), "x").unwrap();
        assert_eq!(count_entries(&dir), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
