// `soli test tests/colis/a_spec.sl`, run from the project root: the spec path
// and its folder are relative, and soli.toml sits in the working directory.
// Walking up from `tests/colis`, `Package::find` returned a bare `soli.toml`,
// whose parent — the project root — was the empty path; every relative import
// then failed with an empty "Module not found: ". A test binary of its own,
// because it changes the process's working directory.
use std::path::Path;

use solilang::module::ModuleResolver;

#[test]
fn relative_import_from_a_spec_folder_run_from_the_project_root() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path();
    std::fs::write(
        proj.join("soli.toml"),
        "[package]\nname = \"proj\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(proj.join("tests/colis")).unwrap();
    std::fs::create_dir_all(proj.join("tests/support")).unwrap();
    std::fs::write(
        proj.join("tests/support/db.sl"),
        "export def answer\n  return 42\nend\n",
    )
    .unwrap();
    let spec = "import \"../support/db.sl\"\nanswer()\n";
    std::fs::write(proj.join("tests/colis/a_spec.sl"), spec).unwrap();
    std::env::set_current_dir(proj).unwrap();

    let program = solilang::parse(spec).unwrap();
    let mut resolver = ModuleResolver::new(Path::new("tests/colis"));
    resolver
        .resolve(program, Path::new("tests/colis/a_spec.sl"))
        .expect("a spec in a subfolder, given as a relative path, imports ../support");
}
