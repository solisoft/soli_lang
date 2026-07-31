//! The directory tree behind the sidebar.
//!
//! Walked once and memoized for a second, then rendered with real box-drawing
//! glyphs (`├─`, `└─`, `│`) rather than chevrons or CSS borders — the sidebar
//! is a `tree(1)` listing, not a docs nav, and saying so typographically is
//! the point. The vertical rail lights up along the chain of ancestors of the
//! page you are on, so the glyphs carry your position rather than decorate it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use crate::template::renderer::html_escape;

/// Entries walked before the tree stops growing.
///
/// A sidebar is a navigation aid, not a database browser — nobody scans a
/// thousand rows, and every one of them is weight on every page (point
/// `soli serve` at a folder with `node_modules/` to see why there is a cap at
/// all). Past this the walk stops and the page says so rather than silently
/// pretending it listed everything. Folder index pages are unaffected: they
/// read their directory directly and are never truncated.
pub(crate) const ENTRY_CAP: usize = 1000;

/// How long a walked tree stays good. Long enough that a burst of requests
/// for one page (HTML + its assets) walks once, short enough that a file you
/// just created shows up on the next reload.
const CACHE_TTL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub name: String,
    /// Path relative to the served root, `/`-separated. Empty for the root.
    pub rel: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub children: Vec<Node>,
}

#[derive(Debug)]
pub(crate) struct Tree {
    pub root: Node,
    /// True when [`ENTRY_CAP`] stopped the walk short.
    pub truncated: bool,
}

impl Node {
    /// Find a descendant by its root-relative path (`""` is the root itself).
    pub fn find(&self, rel: &str) -> Option<&Node> {
        if rel.is_empty() {
            return Some(self);
        }
        let (head, tail) = match rel.split_once('/') {
            Some((head, tail)) => (head, tail),
            None => (rel, ""),
        };
        let child = self.children.iter().find(|c| c.name == head)?;
        if tail.is_empty() {
            Some(child)
        } else {
            child.find(tail)
        }
    }
}

/// The last walk: when it happened, which root it was of, and the result.
///
/// A server has exactly one root for its lifetime, so keying by root buys
/// nothing at runtime — but an unkeyed global would hand one directory's tree
/// to another, which is the kind of latent footgun that only ever shows up
/// under test or in some future embedding that serves two folders.
type CachedWalk = Mutex<Option<(Instant, PathBuf, Arc<Tree>)>>;

/// The tree for `root`, walked at most once per [`CACHE_TTL`].
pub(crate) fn cached(root: &Path) -> Arc<Tree> {
    static CACHE: OnceLock<CachedWalk> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));

    if let Ok(guard) = cache.lock() {
        if let Some((stamp, cached_root, tree)) = guard.as_ref() {
            if cached_root == root && stamp.elapsed() < CACHE_TTL {
                return tree.clone();
            }
        }
    }

    let tree = Arc::new(walk(root));
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), root.to_path_buf(), tree.clone()));
    }
    tree
}

fn walk(root: &Path) -> Tree {
    let mut budget = ENTRY_CAP;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let children = walk_dir(root, "", &mut budget);
    Tree {
        root: Node {
            name,
            rel: String::new(),
            is_dir: true,
            size: 0,
            modified: None,
            children,
        },
        truncated: budget == 0,
    }
}

fn walk_dir(dir: &Path, prefix: &str, budget: &mut usize) -> Vec<Node> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut nodes: Vec<Node> = Vec::new();
    for entry in entries.flatten() {
        if *budget == 0 {
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // Dotfiles are invisible in file mode — see `files::locate`. Listing
        // them in the sidebar would advertise exactly what the resolver
        // refuses to serve.
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        *budget -= 1;

        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        let is_dir = meta.is_dir();
        let children = if is_dir {
            walk_dir(&entry.path(), &rel, budget)
        } else {
            Vec::new()
        };

        nodes.push(Node {
            name,
            rel,
            is_dir,
            size: if is_dir { 0 } else { meta.len() },
            modified: meta.modified().ok(),
            children,
        });
    }

    sort(&mut nodes);
    nodes
}

/// Folders first, then files, each case-insensitively by name — the order a
/// person scanning a listing expects, and the order `ls --group-directories`
/// produces.
pub(crate) fn sort(nodes: &mut [Node]) {
    nodes.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Percent-encode a root-relative path into a URL path, segment by segment
/// (encoding the whole string would escape the separators too).
pub(crate) fn url_path(rel: &str) -> String {
    let encoded: Vec<String> = rel
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect();
    format!("/{}", encoded.join("/"))
}

/// Render the sidebar: the root's name, then one row per entry with a
/// `tree(1)` prefix.
///
/// `current` is the root-relative path of the page being viewed; every row on
/// its ancestor chain gets `.on`, which is what lights the rail.
pub(crate) fn render_sidebar(tree: &Tree, current: &str) -> String {
    let mut out = String::with_capacity(tree.root.children.len() * 96);

    out.push_str("<nav class=\"tree\" aria-label=\"Files\">");
    out.push_str(&format!(
        "<a class=\"root{}\" href=\"/\"><span class=\"n\">{}</span><span class=\"x\">/</span></a>",
        if current.is_empty() { " cur" } else { "" },
        html_escape(&tree.root.name)
    ));

    let mut flags: Vec<bool> = Vec::new();
    render_rows(&tree.root, &mut flags, current, &mut out);

    if tree.truncated {
        out.push_str(&format!(
            "<p class=\"note\">Tree stops at {} entries. Folder pages list \
             everything.</p>",
            ENTRY_CAP
        ));
    }
    out.push_str("</nav>");
    out
}

fn render_rows(node: &Node, flags: &mut Vec<bool>, current: &str, out: &mut String) {
    let last_index = node.children.len().saturating_sub(1);
    for (i, child) in node.children.iter().enumerate() {
        let is_last = i == last_index;
        let on_path = is_ancestor_or_self(&child.rel, current);
        let is_current = child.rel == current;

        // `--d` is the row's depth, which staggers the rail's lighting so the
        // trace runs root-to-leaf instead of all at once. Only lit rows
        // animate, so only they carry it — on a large tree that is a handful
        // of rows instead of every one of them.
        //
        // No `data-p` either: the filter derives its key from `href`, which
        // already holds the path. Emitting both doubled the cost of every row.
        out.push_str(&format!(
            "<a class=\"row{}{}\" href=\"{}{}\"{}>",
            if on_path { " on" } else { "" },
            if is_current { " cur" } else { "" },
            url_path(&child.rel),
            if child.is_dir { "/" } else { "" },
            if on_path {
                format!(" style=\"--d:{}\"", flags.len())
            } else {
                String::new()
            }
        ));

        // The whole `tree(1)` prefix in one span. A span per depth level would
        // read more structurally, but the rail lights per *row*, not per
        // segment, so the extra elements buy nothing and cost real bytes: on a
        // 2000-entry tree they tripled the size of every page.
        out.push_str("<span class=\"g\">");
        for flag in flags.iter() {
            out.push_str(if *flag {
                "\u{a0}\u{a0}\u{a0}"
            } else {
                "\u{2502}\u{a0}\u{a0}"
            });
        }
        out.push_str(if is_last {
            "\u{2514}\u{2500}\u{a0}"
        } else {
            "\u{251c}\u{2500}\u{a0}"
        });
        out.push_str("</span>");

        out.push_str(&format!(
            "<span class=\"n\">{}</span>",
            html_escape(&child.name)
        ));
        if child.is_dir {
            out.push_str("<span class=\"x\">/</span>");
        }
        out.push_str("</a>");

        if child.is_dir && !child.children.is_empty() {
            flags.push(is_last);
            render_rows(child, flags, current, out);
            flags.pop();
        }
    }
}

/// Is `rel` the current path or one of its ancestors?
fn is_ancestor_or_self(rel: &str, current: &str) -> bool {
    if rel.is_empty() || current.is_empty() {
        return false;
    }
    current == rel || current.starts_with(&format!("{}/", rel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("soli_tree_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("api")).unwrap();
        std::fs::write(dir.join("README.md"), "# root").unwrap();
        std::fs::write(dir.join("api/intro.md"), "# intro").unwrap();
        std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
        dir
    }

    #[test]
    fn walk_skips_dotfiles_and_sorts_dirs_first() {
        let dir = fixture("walk");
        let tree = walk(&dir);
        let names: Vec<&str> = tree.root.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["api", "README.md"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_walks_nested_paths() {
        let dir = fixture("find");
        let tree = walk(&dir);
        assert!(tree.root.find("api/intro.md").is_some());
        assert!(tree.root.find("api").unwrap().is_dir);
        assert!(tree.root.find("nope").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sidebar_lights_the_ancestor_chain() {
        let dir = fixture("sidebar");
        let tree = walk(&dir);
        let html = render_sidebar(&tree, "api/intro.md");
        // The folder on the way down is lit, its sibling file is not.
        assert!(html.contains("class=\"row on\" href=\"/api/\""));
        assert!(html.contains("class=\"row on cur\" href=\"/api/intro.md\""));
        assert!(html.contains("class=\"row\" href=\"/README.md\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ancestry_matches_whole_segments_only() {
        assert!(is_ancestor_or_self("api", "api/intro.md"));
        assert!(is_ancestor_or_self("api/intro.md", "api/intro.md"));
        // `api-v2` is not inside `api`, despite the byte-level prefix.
        assert!(!is_ancestor_or_self("api", "api-v2/intro.md"));
    }

    #[test]
    fn url_path_encodes_each_segment() {
        assert_eq!(url_path("a b/c.md"), "/a%20b/c.md");
        assert_eq!(url_path(""), "/");
    }
}
