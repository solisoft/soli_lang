//! `Spreadsheet.excel`, `excel_sheets`, `excel_write` and `csv_write` go
//! through the same jail as `File`: once a jail is installed (what `soli
//! serve` and the job worker do at boot), a relative path resolves under the
//! app root, a path outside it is refused, and a symlink planted inside it is
//! not followed — on both engines. The workbook readers also refuse a file
//! over the size cap and turn a malformed one into an error.
//!
//! Lives in its own test binary on purpose: the jail is process state and
//! first-write-wins, so it must not leak into other tests.
#![cfg(feature = "office")]

use std::fs;
use std::path::PathBuf;

use solilang::interpreter::builtins::file::set_file_jail;
use solilang::{run_script, Engine};

fn run(source: &str, engine: Engine) -> Result<(), String> {
    run_script(source, None, false, engine).map_err(|e| e.to_string())
}

fn refused(source: &str, engine: Engine, needle: &str) {
    match run(source, engine) {
        Ok(()) => panic!("{engine:?}: expected a refusal for:\n{source}"),
        Err(message) => assert!(
            message.contains(needle),
            "{engine:?}: expected {needle:?} in {message:?} for:\n{source}"
        ),
    }
}

#[test]
fn spreadsheet_paths_honour_the_app_root_jail_on_both_engines() {
    std::env::set_var("SOLI_VM_OPTIONAL_LET", "1");
    let base: PathBuf =
        std::env::temp_dir().join(format!("soli_sheet_jail_{}", std::process::id()));
    let app = base.join("app");
    fs::create_dir_all(app.join("exports")).expect("create the app root");
    let base = fs::canonicalize(&base).expect("canonical temp dir");
    let app = base.join("app");
    let outside = base.join("outside.xlsx");
    fs::write(&outside, b"must survive").expect("write the outside file");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, app.join("exports/link.xlsx")).expect("symlink");
    fs::write(app.join("exports/bad.xlsx"), b"PK\x03\x04 not really a zip")
        .expect("write a malformed workbook");

    set_file_jail(app.clone());

    for engine in [Engine::Tree, Engine::Vm] {
        // Inside the jail: write, then read back through both readers.
        let inside = r#"
Spreadsheet.excel_write([{"name": "Ada", "age": 36}], "exports/people.xlsx")
rows = Spreadsheet.excel("exports/people.xlsx")
throw "excel gave #{rows}" unless rows[0]["name"] == "Ada"
sheets = Spreadsheet.excel_sheets("exports/people.xlsx")
throw "excel_sheets gave #{sheets}" unless sheets["Sheet1"].length == 1
Spreadsheet.csv_write([{"name": "Ada"}], "exports/people.csv")
"#;
        if let Err(message) = run(inside, engine) {
            panic!("{engine:?}: writing and reading inside the jail failed: {message}");
        }

        let outside_path = outside.display().to_string();
        for (call, who) in [
            (
                format!("Spreadsheet.excel({outside_path:?})"),
                "Spreadsheet.excel",
            ),
            (
                format!("Spreadsheet.excel_sheets({outside_path:?})"),
                "Spreadsheet.excel_sheets",
            ),
            (
                format!("Spreadsheet.excel_write([{{\"a\": 1}}], {outside_path:?})"),
                "Spreadsheet.excel_write",
            ),
            (
                format!("Spreadsheet.csv_write([{{\"a\": 1}}], {outside_path:?})"),
                "Spreadsheet.csv_write",
            ),
            (
                "Spreadsheet.excel(\"../outside.xlsx\")".to_string(),
                "Spreadsheet.excel",
            ),
            (
                "Spreadsheet.excel_write([{\"a\": 1}], \"../outside.xlsx\")".to_string(),
                "Spreadsheet.excel_write",
            ),
        ] {
            refused(&call, engine, "escapes the app-root jail");
            refused(&call, engine, who);
        }

        // A symlink inside the jail that points out of it is not followed,
        // for reading or for writing.
        #[cfg(unix)]
        {
            refused(
                "Spreadsheet.excel(\"exports/link.xlsx\")",
                engine,
                "Spreadsheet.excel",
            );
            refused(
                "Spreadsheet.excel_write([{\"a\": 1}], \"exports/link.xlsx\")",
                engine,
                "Spreadsheet.excel_write",
            );
        }

        // A malformed workbook is an error of its own call, not a panic.
        refused(
            "Spreadsheet.excel(\"exports/bad.xlsx\")",
            engine,
            "Spreadsheet.excel",
        );
    }

    assert_eq!(
        fs::read(&outside).expect("the outside file is still there"),
        b"must survive",
        "no write may reach the file outside the jail"
    );

    // The size caps: a workbook over either one is refused before parsing.
    std::env::set_var("SOLI_SPREADSHEET_MAX_BYTES", "64");
    refused(
        "Spreadsheet.excel(\"exports/people.xlsx\")",
        Engine::Tree,
        "SOLI_SPREADSHEET_MAX_BYTES",
    );
    std::env::remove_var("SOLI_SPREADSHEET_MAX_BYTES");
    std::env::set_var("SOLI_SPREADSHEET_MAX_INFLATED_BYTES", "64");
    refused(
        "Spreadsheet.excel_sheets(\"exports/people.xlsx\")",
        Engine::Tree,
        "SOLI_SPREADSHEET_MAX_INFLATED_BYTES",
    );
    std::env::remove_var("SOLI_SPREADSHEET_MAX_INFLATED_BYTES");
    run("Spreadsheet.excel(\"exports/people.xlsx\")", Engine::Tree)
        .expect("back under the default caps");

    let _ = fs::remove_dir_all(&base);
}
