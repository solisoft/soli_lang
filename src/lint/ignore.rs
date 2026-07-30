//! Files the linter skips during a directory walk.
//!
//! Translation tables are data that happens to be written in Soli — long
//! sentences in a hash literal, one key per line. Running style rules over
//! them produces hundreds of `style/line-length` hits that nobody will ever
//! act on, which drowns out the real findings in the rest of the app.
//!
//! Two shapes are recognised as locale files:
//!
//! - anything under a directory named `locales/` (the framework's own
//!   `config/locales/` convention), and
//! - a file whose stem is `locale_<tag>` or `<tag>_locale`, where `<tag>`
//!   looks like a locale code — `locale_fr.sl`, `locale_zh.sl`,
//!   `pt_BR_locale.sl`.
//!
//! The tag check is deliberately narrow so that helpers *about* locales keep
//! being linted: `locale_helper.sl` and `locale_switcher.sl` are code, not
//! data. Skips only apply while walking a directory — naming a file
//! explicitly (`soli lint app/helpers/locale_fr.sl`) always lints it.

use std::path::Path;

/// True when `path` looks like a translation table rather than app code.
pub fn is_locale_file(path: &Path) -> bool {
    if path
        .parent()
        .into_iter()
        .flat_map(|parent| parent.components())
        .any(|c| c.as_os_str().eq_ignore_ascii_case("locales"))
    {
        return true;
    }

    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };

    // `.html.slv` and friends leave a second extension on the stem — strip it
    // so `locale_fr.html.slv` is judged on `locale_fr`.
    let stem = stem.split('.').next().unwrap_or(stem);

    if let Some(tag) = stem.strip_prefix("locale_") {
        return is_locale_tag(tag);
    }
    if let Some(tag) = stem.strip_suffix("_locale") {
        return is_locale_tag(tag);
    }
    false
}

/// A permissive BCP-47 shape check: a 2–3 letter language subtag, optionally
/// followed by one `_`/`-` separated script or region subtag of 2–4
/// alphanumerics. Matches `fr`, `zh`, `fil`, `pt_BR`, `zh-Hans`, `es-419`;
/// rejects `helper`, `switcher`, `table`.
fn is_locale_tag(tag: &str) -> bool {
    let mut parts = tag.split(['_', '-']);

    let Some(language) = parts.next() else {
        return false;
    };
    if !(2..=3).contains(&language.len()) || !language.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }

    match parts.next() {
        None => true,
        Some(subtag) => {
            let well_formed = (2..=4).contains(&subtag.len())
                && subtag.chars().all(|c| c.is_ascii_alphanumeric());
            // At most one subtag — `locale_en_us_backup` is not a locale.
            well_formed && parts.next().is_none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn skipped(p: &str) -> bool {
        is_locale_file(&PathBuf::from(p))
    }

    #[test]
    fn skips_files_under_a_locales_directory() {
        assert!(skipped("config/locales/en.sl"));
        assert!(skipped("config/locales/nested/deep.sl"));
        assert!(skipped("locales/fr.sl"));
    }

    #[test]
    fn skips_locale_prefixed_and_suffixed_stems() {
        assert!(skipped("app/helpers/locale_fr.sl"));
        assert!(skipped("app/helpers/locale_zh.sl"));
        assert!(skipped("app/i18n/pt_BR_locale.sl"));
        assert!(skipped("app/i18n/locale_zh-Hans.sl"));
        assert!(skipped("app/i18n/locale_es-419.sl"));
        assert!(skipped("app/i18n/locale_fil.sl"));
    }

    #[test]
    fn lints_helpers_that_merely_mention_locales() {
        assert!(!skipped("app/helpers/locale_helper.sl"));
        assert!(!skipped("app/helpers/locale_switcher.sl"));
        assert!(!skipped("app/helpers/locale_table_builder.sl"));
        assert!(!skipped("app/helpers/locale.sl"));
        assert!(!skipped("app/services/locale_en_us_backup.sl"));
    }

    #[test]
    fn lints_ordinary_files() {
        assert!(!skipped("app/controllers/home_controller.sl"));
        assert!(!skipped("app/models/post.sl"));
        assert!(!skipped("app/views/posts/index.html.slv"));
    }

    #[test]
    fn a_locales_named_file_is_not_a_locales_directory() {
        // Only *directory* components count — a file called `locales.sl` is
        // more likely a registry of available locales than a table.
        assert!(!skipped("app/helpers/locales.sl"));
    }

    #[test]
    fn strips_double_extensions_before_matching() {
        assert!(skipped("app/views/locale_fr.html.slv"));
        assert!(!skipped("app/views/locale_helper.html.slv"));
    }
}
