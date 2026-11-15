//! `soli generate privacy` — export and erasure of what the app holds about a
//! user: a register of the models that hold personal data
//! (`app/services/privacy.sl`), a page where a user downloads or erases
//! theirs, and routes. No file of the auth scaffold is edited.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{append_routes_once, write_if_absent};
use crate::scaffold::templates::privacy;

/// Generate the privacy files into an app that has the auth scaffold.
pub fn create_privacy(folder: &str) -> Result<(), String> {
    let app_path = Path::new(folder);
    if !app_path.join("app/models/user.sl").exists() {
        return Err(
            "Privacy requires `soli generate auth` first (missing app/models/user.sl).".into(),
        );
    }
    for dir in [
        "app/services",
        "app/controllers",
        "app/views/privacy",
        "config",
    ] {
        let path = app_path.join(dir);
        if !path.exists() {
            fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
        }
    }
    for (rel, contents) in [
        ("app/services/privacy.sl", privacy::PRIVACY_SERVICE),
        (
            "app/controllers/privacy_controller.sl",
            privacy::PRIVACY_CONTROLLER,
        ),
        (
            "app/views/privacy/show.html.slv",
            privacy::PRIVACY_SHOW_VIEW,
        ),
    ] {
        write_if_absent(app_path, rel, contents)?;
    }
    append_routes_once(
        app_path,
        privacy::ROUTES_MARKER,
        &privacy::routes_snippet(),
        "privacy",
    )
}

/// Next steps after `soli generate privacy`.
pub fn print_privacy_success_message() {
    println!();
    println!("  \x1b[32m\x1b[1mPrivacy:\x1b[0m users download or erase their data at \x1b[36m/privacy\x1b[0m.");
    println!();
    println!("  \x1b[1mList your models\x1b[0m in \x1b[36mPRIVACY_MODELS\x1b[0m (app/services/privacy.sl): each one that");
    println!("  holds personal data, its owner field, and whether an erasure deletes,");
    println!("  anonymizes or keeps it. A model that is not listed is neither exported");
    println!("  nor erased.");
    println!();
    println!(
        "  From a script: \x1b[36mPrivacy.export_data(user)\x1b[0m, \x1b[36mPrivacy.erase(user)\x1b[0m."
    );
    println!("  Docs: /docs/security/privacy");
    println!();
}
