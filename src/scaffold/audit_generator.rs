//! `soli generate audit [--chain]` — a log of who changed what.
//!
//! Two files of plain Soli over what the ORM already has (callbacks, dirty
//! tracking, concerns): the `Audited` concern a model includes, and the
//! `Audit` model its entries are. Written only when absent; the migration
//! once. It needs no auth scaffold — without a signed-in user the actor is
//! "system".

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{write_if_absent, write_migration_once};
use crate::scaffold::templates::audit;

/// Generate the audit files into the application at `folder`. `chained`
/// links the entries by hash (`soli generate audit --chain`).
pub fn create_audit(folder: &str, chained: bool) -> Result<(), String> {
    let app_path = Path::new(folder);
    if !app_path.exists() {
        return Err(format!("Directory '{folder}' does not exist"));
    }
    if !app_path.join("app").is_dir() {
        return Err(format!(
            "'{folder}' does not look like a Soli app (no app/ directory). \
             Run this inside a project created with `soli new`."
        ));
    }
    for dir in ["app/models/concerns", "db/migrations"] {
        let path = app_path.join(dir);
        if !path.exists() {
            fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
        }
    }
    write_if_absent(
        app_path,
        "app/models/concerns/audited.sl",
        audit::AUDITED_CONCERN,
    )?;
    write_if_absent(
        app_path,
        "app/models/audit.sl",
        &audit::audit_model(chained),
    )?;
    write_migration_once(
        app_path,
        "create_audits",
        0,
        &audit::audits_migration(chained),
    )?;
    Ok(())
}

/// Next steps after `soli generate audit`.
pub fn print_audit_success_message(chained: bool) {
    println!();
    println!("  \x1b[32m\x1b[1mAudit log:\x1b[0m add \x1b[36minclude Audited\x1b[0m to the models to log.");
    println!();
    println!("  1. \x1b[36msoli db:migrate up\x1b[0m");
    println!(
        "  2. In a model:  \x1b[36mclass Invoice < Model\x1b[0m / \x1b[36m  include Audited\x1b[0m"
    );
    println!("  3. Read it:     \x1b[36minvoice.audits\x1b[0m, \x1b[36mAudit.by_actor(user)\x1b[0m, \x1b[36mAudit.between(from, to)\x1b[0m");
    println!();
    println!("  Values of \x1b[36mencrypts\x1b[0m fields and of \x1b[36mpassword_digest\x1b[0m are withheld;");
    println!("  override \x1b[36maudit_redacted\x1b[0m / \x1b[36maudit_ignored\x1b[0m in a model for more.");
    if chained {
        println!("  Entries are chained by hash: \x1b[36mAudit.verify\x1b[0m returns nil while the log is intact.");
        println!("  Set \x1b[36mAUDIT_CHAIN_KEY\x1b[0m (32+ characters) so the hashes are keyed, and keep");
        println!("  \x1b[36mAudit.head\x1b[0m outside the database for \x1b[36mAudit.verify(head)\x1b[0m: without them a writer");
        println!("  of the table can redo the chain, or cut its tail, unnoticed.");
    } else {
        println!("  To notice an edited or removed entry, generate with \x1b[36m--chain\x1b[0m.");
    }
    println!("  Not logged: update_all, delete_all, increment/decrement and raw queries.");
    println!("  Docs: /docs/database/models#audit-log");
    println!();
}
