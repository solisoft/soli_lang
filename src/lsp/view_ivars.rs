//! `@x` in a view that nothing sets (#820).
//!
//! A view reads its controller's instance variables: `<%= @post.title %>` in
//! `app/views/posts/show.html.slv` reads what `PostsController#show` set. One
//! the action never sets reads nil, silently. This finds the instance
//! variables a view can count on — set by an action that renders it, by a
//! method that action calls, by a `before_action` (any `static { … }` code of
//! the controller), by a parent controller, declared as a field, or set by a
//! helper or a middleware — and reports a read of any other.
//!
//! False positives are worse than silence, so every doubt is silence
//! (`None`): a layout, a partial or a component (rendered by many actions);
//! a view with no controller of its own, or none of whose actions renders it;
//! a controller that renders a computed name, sets instance variables by
//! reflection (`instance_variable_set`, `send`…), includes a module, or
//! inherits from a controller this cannot read.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::expr::{Argument, ExprKind};
use crate::ast::stmt::{ClassDecl, StmtKind};
use crate::ast::{Expr, Program, Stmt};

/// The instance variables a view may read, and the actions that render it
/// (for the message). `None` when the view cannot be judged.
pub(crate) struct ViewIvars {
    pub set: BTreeSet<String>,
    pub actions: Vec<String>,
    pub controller: String,
}

/// What [`view_ivars`] reads: a file's text, from an open buffer or disk.
pub(crate) type ReadFile<'a> = &'a dyn Fn(&Path) -> Option<String>;

/// Names whose call means the class sets instance variables in ways the
/// source does not show.
const REFLECTION: [&str; 6] = [
    "instance_variable_set",
    "send",
    "public_send",
    "define_method",
    "class_eval",
    "instance_eval",
];

/// The instance variables `view_path` (an `app/views/<dir>/<name>.slv`) may
/// read. See the module comment for when it answers `None`.
pub(crate) fn view_ivars(view_path: &Path, read: ReadFile) -> Option<ViewIvars> {
    let text = view_path.to_string_lossy().replace('\\', "/");
    let at = text.rfind("/app/views/")?;
    let root = PathBuf::from(&text[..at]);
    let relative = &text[at + "/app/views/".len()..];
    let (dir, file) = relative.rsplit_once('/')?;
    let stem = file.split('.').next()?;
    let first_dir = dir.split('/').next().unwrap_or(dir);
    if stem.is_empty()
        || stem.starts_with('_')
        || matches!(
            first_dir,
            "layouts" | "components" | "shared" | "partials" | "mailers"
        )
    {
        return None;
    }

    let controller_path = root
        .join("app/controllers")
        .join(format!("{dir}_controller.sl"));
    let program = crate::parse(&read(&controller_path)?).ok()?;
    let class = controller_class(&program)?;
    if !class.includes.is_empty() || !class.extends.is_empty() || uses_reflection(class) {
        return None;
    }

    // The actions that render the view: by convention the one of its name,
    // and any that names it in `render("…")`. A computed name could be it.
    let full = format!("{dir}/{stem}");
    let mut actions: Vec<String> = Vec::new();
    for method in &class.methods {
        let mut renders_it = method.name == stem;
        let mut dynamic = false;
        for_each_call(&method.body, &mut |name, arguments| {
            // A bare read of `render` (the callee of the call below) is no
            // render of its own.
            let (Some(arguments), "render") = (arguments, name) else {
                return;
            };
            match arguments.first() {
                Some(Argument::Positional(first)) => match &first.kind {
                    ExprKind::StringLiteral(target) => {
                        if target == &full || target == stem {
                            renders_it = true;
                        }
                    }
                    _ => dynamic = true,
                },
                _ => dynamic = true,
            }
        });
        if dynamic {
            return None;
        }
        if renders_it {
            actions.push(method.name.clone());
        }
    }
    if actions.is_empty() {
        return None;
    }

    let mut set = BTreeSet::new();
    for field in &class.fields {
        set.insert(field.name.clone());
    }
    // The rendering actions and the methods they call, transitively.
    let mut pending: Vec<&str> = actions.iter().map(String::as_str).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name) {
            continue;
        }
        let Some(method) = class.methods.iter().find(|m| m.name == name) else {
            continue;
        };
        assigned_ivars(&method.body, &mut set);
        for_each_call(&method.body, &mut |callee, _| {
            if let Some(next) = class.methods.iter().find(|m| m.name == callee) {
                pending.push(next.name.as_str());
            }
        });
    }
    // `before_action` hooks and other class-level code.
    if let Some(block) = &class.static_block {
        assigned_ivars(block, &mut set);
    }
    assigned_ivars(&class.class_statements, &mut set);

    // Parent controllers: everything they set, as they may set it for any
    // action of a subclass.
    let mut parent = class.superclass.clone();
    let mut depth = 0;
    while let Some(name) = parent {
        if name == "Controller" {
            break;
        }
        depth += 1;
        if depth > 16 {
            return None;
        }
        let path = root
            .join("app/controllers")
            .join(format!("{}.sl", snake_case(&name)));
        let program = crate::parse(&read(&path)?).ok()?;
        let class = program_classes(&program).find(|c| c.name == name)?;
        if !class.includes.is_empty() || uses_reflection(class) {
            return None;
        }
        all_ivars_of_class(class, &mut set);
        parent = class.superclass.clone();
    }

    // Helpers and middleware may set them for every view.
    for dir in ["app/helpers", "app/middleware"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "sl") {
                if let Some(program) = read(&path).and_then(|text| crate::parse(&text).ok()) {
                    assigned_ivars(&program.statements, &mut set);
                }
            }
        }
    }

    Some(ViewIvars {
        set,
        actions,
        controller: dir.to_string(),
    })
}

/// The first top-level class of the file whose name ends in `Controller`.
fn controller_class(program: &Program) -> Option<&ClassDecl> {
    program_classes(program).find(|class| class.name.ends_with("Controller"))
}

fn program_classes(program: &Program) -> impl Iterator<Item = &ClassDecl> {
    program
        .statements
        .iter()
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Class(class) => Some(class.as_ref()),
            _ => None,
        })
}

/// Every instance variable a class sets anywhere: its fields, methods,
/// constructor and class-level code.
fn all_ivars_of_class(class: &ClassDecl, set: &mut BTreeSet<String>) {
    for field in &class.fields {
        set.insert(field.name.clone());
    }
    for method in &class.methods {
        assigned_ivars(&method.body, set);
    }
    if let Some(constructor) = &class.constructor {
        assigned_ivars(&constructor.body, set);
    }
    if let Some(block) = &class.static_block {
        assigned_ivars(block, set);
    }
    assigned_ivars(&class.class_statements, set);
}

/// Whether any code of the class calls one of the [`REFLECTION`] names.
fn uses_reflection(class: &ClassDecl) -> bool {
    let mut found = false;
    let mut check = |stmts: &[Stmt]| {
        for_each_call(stmts, &mut |name, _| {
            if REFLECTION.contains(&name) {
                found = true;
            }
        });
    };
    for method in &class.methods {
        check(&method.body);
    }
    if let Some(block) = &class.static_block {
        check(block);
    }
    check(&class.class_statements);
    found
}

/// `@x = …`, `@x ||= …`, `this.x = …` in `stmts`, lambdas included.
fn assigned_ivars(stmts: &[Stmt], set: &mut BTreeSet<String>) {
    let mut on_stmt = |_: &Stmt| {};
    let mut on_expr = |expr: &Expr| {
        let target = match &expr.kind {
            ExprKind::Assign { target, .. } | ExprKind::CompoundAssign { target, .. } => target,
            _ => return,
        };
        if let ExprKind::Member { object, name } = &target.kind {
            if matches!(object.kind, ExprKind::This) {
                set.insert(name.clone());
            }
        }
    };
    crate::ast::walk::walk_stmts(stmts, &mut on_stmt, &mut on_expr);
}

/// Calls `f(name, Some(arguments))` for every call in `stmts` whose callee
/// is a bare name, `@name(…)` or `this.name(…)` — and `f(name, None)` for
/// every bare `name` / `@name` read, which may call a method too.
fn for_each_call(stmts: &[Stmt], f: &mut dyn FnMut(&str, Option<&[Argument]>)) {
    let mut on_stmt = |_: &Stmt| {};
    let mut on_expr = |expr: &Expr| match &expr.kind {
        ExprKind::Call { callee, arguments } => match &callee.kind {
            ExprKind::Variable(name) => f(name, Some(arguments)),
            ExprKind::Member { object, name } => match &object.kind {
                ExprKind::This => f(name, Some(arguments)),
                // `obj.send(…)` on anything is reflection too.
                _ if REFLECTION.contains(&name.as_str()) => f(name, Some(arguments)),
                _ => {}
            },
            _ => {}
        },
        ExprKind::Member { object, name } if matches!(object.kind, ExprKind::This) => f(name, None),
        ExprKind::Variable(name) => f(name, None),
        _ => {}
    };
    crate::ast::walk::walk_stmts(stmts, &mut on_stmt, &mut on_expr);
}

/// `ApplicationController` → `application_controller`.
fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// One `@name` read in a view's code: the name and its byte range in the
/// view's text (the `@` included).
pub(crate) struct IvarRead {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

/// The `@name` reads inside the `<% … %>` tags of `text` (not in `<%# %>`
/// comments, not in string literals, not `email@host`), and the names the
/// view assigns itself (`<% @title = "x" %>`), which it may then read.
pub(crate) fn ivar_reads(text: &str) -> (Vec<IvarRead>, HashSet<String>) {
    let bytes = text.as_bytes();
    let mut reads = Vec::new();
    let mut assigned = HashSet::new();
    let mut i = 0;
    while let Some(open) = text[i..].find("<%") {
        let start = i + open + 2;
        let Some(close) = text[start..].find("%>") else {
            break;
        };
        let end = start + close;
        i = end + 2;
        if bytes.get(start) == Some(&b'#') {
            continue;
        }
        let mut quote: Option<u8> = None;
        let mut j = start;
        while j < end {
            let b = bytes[j];
            if let Some(q) = quote {
                if b == b'\\' {
                    j += 2;
                    continue;
                }
                if b == q {
                    quote = None;
                }
                j += 1;
                continue;
            }
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'#' => break, // a comment to the end of the tag
                b'@' => {
                    let before = if j > 0 { bytes[j - 1] } else { b' ' };
                    let starts_name = bytes
                        .get(j + 1)
                        .is_some_and(|c| c.is_ascii_lowercase() || *c == b'_');
                    if starts_name
                        && !(before.is_ascii_alphanumeric()
                            || before == b'_'
                            || before == b'@'
                            || before == b'.')
                    {
                        let mut k = j + 1;
                        while k < end && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_') {
                            k += 1;
                        }
                        // `?` / `!` end a method name: `@ready?` calls one.
                        let method = matches!(bytes.get(k), Some(b'?' | b'!' | b'('));
                        let name = text[j + 1..k].to_string();
                        let rest = text[k..end].trim_start();
                        let assigns = (rest.starts_with('=') && !rest.starts_with("=="))
                            || ["||=", "&&=", "??=", "+=", "-=", "<<"]
                                .iter()
                                .any(|op| rest.starts_with(op));
                        if assigns {
                            assigned.insert(name);
                        } else if !method {
                            reads.push(IvarRead {
                                name,
                                start: j,
                                end: k,
                            });
                        }
                        j = k;
                        continue;
                    }
                }
                _ => {}
            }
            j += 1;
        }
    }
    (reads, assigned)
}

/// The reads of `text` that `ivars` does not set, the view's own
/// assignments aside.
pub(crate) fn undefined_reads(text: &str, ivars: &ViewIvars) -> Vec<IvarRead> {
    let (reads, assigned) = ivar_reads(text);
    reads
        .into_iter()
        .filter(|read| !ivars.set.contains(&read.name) && !assigned.contains(&read.name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn reader(files: &[(&str, &str)]) -> impl Fn(&Path) -> Option<String> {
        let map: HashMap<PathBuf, String> = files
            .iter()
            .map(|(path, text)| (PathBuf::from(path), text.to_string()))
            .collect();
        move |path: &Path| map.get(path).cloned()
    }

    const VIEW: &str = "/site/app/views/posts/show.html.slv";

    fn missing(controller: &str, view: &str) -> Option<Vec<String>> {
        let read = reader(&[("/site/app/controllers/posts_controller.sl", controller)]);
        let ivars = view_ivars(Path::new(VIEW), &read)?;
        Some(
            undefined_reads(view, &ivars)
                .into_iter()
                .map(|read| read.name)
                .collect(),
        )
    }

    #[test]
    fn a_read_the_action_never_sets_is_reported() {
        let controller = "class PostsController < Controller\n  def show\n    @post = Post.find(params[\"id\"])\n    @_load\n  end\n\n  private\n  def _load\n    @tags = []\n  end\nend\n";
        let view = "<h1><%= @post.title %></h1>\n<%= @tags.length %> <%= @author.name %>\n<% @title = \"x\" %><%= @title %>\n<%# @nope %> <%= \"a@b.c @x\" %>";
        assert_eq!(missing(controller, view), Some(vec!["author".to_string()]));
    }

    #[test]
    fn before_actions_fields_and_explicit_renders_count() {
        let controller = "class PostsController < Controller\n  current: Any = nil\n  static {\n    this.before_action(:show) = fn(req) {\n      @post = Post.find(params[\"id\"])\n      req\n    }\n  }\n  def preview\n    @draft = true\n    render(\"posts/show\")\n  end\nend\n";
        let view = "<%= @post %><%= @draft %><%= @current %>";
        assert_eq!(missing(controller, view), Some(vec![]));
    }

    #[test]
    fn doubt_is_silence() {
        let view = "<%= @anything %>";
        // No action renders the view.
        assert!(missing(
            "class PostsController < Controller\n  def index\n  end\nend\n",
            view
        )
        .is_none());
        // A computed render.
        assert!(missing(
            "class PostsController < Controller\n  def show\n    render(\"posts/\" + params[\"v\"])\n  end\nend\n",
            view
        )
        .is_none());
        // Reflection.
        assert!(missing(
            "class PostsController < Controller\n  def show\n    instance_variable_set(\"@x\", 1)\n  end\nend\n",
            view
        )
        .is_none());
        // A parent controller this cannot read.
        assert!(missing(
            "class PostsController < ApplicationController\n  def show\n  end\nend\n",
            view
        )
        .is_none());
        // A partial and a layout.
        let read = reader(&[(
            "/site/app/controllers/posts_controller.sl",
            "class PostsController < Controller\n  def show\n  end\nend\n",
        )]);
        assert!(view_ivars(Path::new("/site/app/views/posts/_card.html.slv"), &read).is_none());
        assert!(view_ivars(
            Path::new("/site/app/views/layouts/application.html.slv"),
            &read
        )
        .is_none());
    }

    #[test]
    fn a_parent_controller_counts() {
        let read = reader(&[
            (
                "/site/app/controllers/posts_controller.sl",
                "class PostsController < ApplicationController\n  def show\n  end\nend\n",
            ),
            (
                "/site/app/controllers/application_controller.sl",
                "class ApplicationController < Controller\n  def _who\n    @user = 1\n  end\nend\n",
            ),
        ]);
        let ivars = view_ivars(Path::new(VIEW), &read).expect("judged");
        let names: Vec<String> = undefined_reads("<%= @user %><%= @nope %>", &ivars)
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(names, vec!["nope".to_string()]);
    }
}
