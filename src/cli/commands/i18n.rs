//! `soli i18n:export` and `soli i18n:missing`: the application's side of the
//! built-in validation-message catalog.

use std::path::Path;
use std::process;

use solilang::interpreter::builtins::i18n::helpers;

/// `soli i18n:export <locale> [folder] [--force]` — write the messages Soli
/// ships for `locale` to `config/locales/soli.<locale>.yml`, to edit. A key
/// the application keeps there overrides the built-in one; a key it deletes
/// falls back to it, so the file may be trimmed to what is changed.
pub fn run_i18n_export(locale: &str, folder: &str, force: bool) {
    let Some(source) = helpers::builtin_catalog_source(locale) else {
        eprintln!(
            "i18n:export: Soli ships no messages for \"{locale}\" (it ships {}). \
             For another language, copy one of those and translate it: \
             soli i18n:export en, then rename the file and its top-level key.",
            helpers::builtin_message_locales().join(", ")
        );
        process::exit(1);
    };
    let dir = Path::new(folder).join("config").join("locales");
    let target = dir.join(format!("soli.{locale}.yml"));
    if target.exists() && !force {
        eprintln!(
            "i18n:export: {} exists; pass --force to overwrite it",
            target.display()
        );
        process::exit(1);
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("i18n:export: cannot create {}: {e}", dir.display());
        process::exit(1);
    }
    let header = "# Soli's built-in messages, exported by `soli i18n:export`. Edit any\n\
                  # key; delete the ones you keep as they are (they fall back to Soli's).\n";
    // The catalog's own header comment is about the binary; replace it.
    let body: String = source
        .lines()
        .skip_while(|line| line.starts_with('#'))
        .map(|line| format!("{line}\n"))
        .collect();
    if let Err(e) = std::fs::write(&target, format!("{header}{body}")) {
        eprintln!("i18n:export: cannot write {}: {e}", target.display());
        process::exit(1);
    }
    println!("Wrote {}", target.display());
}

/// `soli i18n:missing <locale> [folder] [--from <locale>]` — the keys the
/// reference locale (`--from`, default the application's default locale, or
/// `en`) has and `locale` lacks, counting Soli's built-in messages on both
/// sides. Plural forms count as one key. Exits 1 when something is missing,
/// so a CI step can hold a release on it.
pub fn run_i18n_missing(locale: &str, folder: &str, from: Option<&str>) {
    let store = helpers::parse_locales(&Path::new(folder).join("config"));
    let reference = from
        .map(str::to_string)
        .or_else(|| std::env::var("SOLI_DEFAULT_LOCALE").ok())
        .unwrap_or_else(|| "en".to_string());
    if reference == locale {
        eprintln!("i18n:missing: {locale} is the reference locale; pass --from <locale>");
        process::exit(64);
    }

    let keys_of = |l: &str| {
        let mut keys = store
            .get(l)
            .map(helpers::translation_keys)
            .unwrap_or_default();
        if let Some(tree) = helpers::builtin_catalog_tree(l) {
            keys.extend(helpers::translation_keys(&tree));
        }
        keys
    };
    let wanted = keys_of(&reference);
    let present = keys_of(locale);
    if !store.contains_key(locale) && helpers::builtin_catalog_source(locale).is_none() {
        println!(
            "No config/locales file declares \"{locale}\", and Soli ships no messages for it."
        );
    }
    let missing: Vec<&String> = wanted.difference(&present).collect();
    if missing.is_empty() {
        println!(
            "{locale}: nothing missing against {reference} ({} keys)",
            wanted.len()
        );
        return;
    }
    println!(
        "{locale}: {} of {} keys of {reference} missing",
        missing.len(),
        wanted.len()
    );
    for key in missing {
        println!("  {key}");
    }
    process::exit(1);
}
