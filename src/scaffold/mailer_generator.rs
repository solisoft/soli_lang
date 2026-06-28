//! `soli generate mailer <Name> <action...>` — scaffold a mailer class plus a
//! matching HTML view per action.
//!
//! ```text
//! soli generate mailer User welcome reset_password
//!   -> app/mailers/user_mailer.sl                (class UserMailer < Mailer)
//!      app/views/user_mailer/welcome.html.slv
//!      app/views/user_mailer/reset_password.html.slv
//! ```

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::write_file;
use crate::scaffold::utils::{to_pascal_case, to_snake_case};

/// Generate a mailer (and its views) into the app at `folder`.
pub fn create_mailer(folder: &str, name: &str, actions: &[String]) -> Result<(), String> {
    let app_path = Path::new(folder);
    if !app_path.join("app").is_dir() {
        return Err(format!(
            "'{}' does not look like a Soli app (no app/ directory). \
             Run this inside a project created with `soli new`.",
            folder
        ));
    }

    // Accept "User", "user", or "UserMailer" — normalize to a base name.
    let mut base = to_snake_case(name);
    if let Some(stripped) = base.strip_suffix("_mailer") {
        base = stripped.to_string();
    }
    if base.is_empty() {
        return Err("mailer name must not be empty".to_string());
    }
    let class_name = format!("{}Mailer", to_pascal_case(&base));
    let mailer_snake = format!("{base}_mailer");

    let actions: Vec<String> = if actions.is_empty() {
        vec!["welcome".to_string()]
    } else {
        actions.iter().map(|a| to_snake_case(a)).collect()
    };

    let mailers_dir = app_path.join("app/mailers");
    fs::create_dir_all(&mailers_dir).map_err(|e| format!("Failed to create app/mailers: {e}"))?;
    let views_dir = app_path.join("app/views").join(&mailer_snake);
    fs::create_dir_all(&views_dir)
        .map_err(|e| format!("Failed to create app/views/{mailer_snake}: {e}"))?;

    // Mailer class.
    let mut body = String::new();
    body.push_str(&format!("# app/mailers/{mailer_snake}.sl\n"));
    body.push_str(&format!("class {class_name} < Mailer\n"));
    for (i, action) in actions.iter().enumerate() {
        if i > 0 {
            body.push('\n');
        }
        let humanized = humanize(action);
        body.push_str(&format!("  def {action}(user)\n"));
        body.push_str("    @user = user\n");
        body.push_str(&format!(
            "    # Renders app/views/{mailer_snake}/{action}.html.slv with @user.\n"
        ));
        body.push_str(&format!(
            "    this.mail(to: user[\"email\"], subject: \"{humanized}\")\n"
        ));
        body.push_str("  end\n");
    }
    body.push_str("end\n");
    write_if_absent(&mailers_dir.join(format!("{mailer_snake}.sl")), &body)?;

    // One HTML view per action.
    for action in &actions {
        let humanized = humanize(action);
        let view = format!(
            "<h1>{humanized}</h1>\n\
             <p>Hi <%= h(user[\"name\"]) %>,</p>\n\n\
             <p>Edit <code>app/views/{mailer_snake}/{action}.html.slv</code> to write this email.</p>\n"
        );
        write_if_absent(&views_dir.join(format!("{action}.html.slv")), &view)?;
    }

    println!("\nGenerated mailer {class_name}:");
    println!("  app/mailers/{mailer_snake}.sl");
    for action in &actions {
        println!("  app/views/{mailer_snake}/{action}.html.slv");
    }
    println!("\nSend it from a controller or job:");
    println!("  {class_name}.{}(user).deliver_later", actions[0]);
    println!("\nConfigure delivery in config/application.sl (Mailer.configure({{ ... }})).");
    Ok(())
}

fn write_if_absent(path: &Path, content: &str) -> Result<(), String> {
    if path.exists() {
        println!("  skip (already exists) {}", path.display());
        return Ok(());
    }
    write_file(path, content)
}

/// `reset_password` -> `Reset Password`.
fn humanize(action: &str) -> String {
    let mut out = String::new();
    for (i, word) in action.split('_').filter(|w| !w.is_empty()).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}
