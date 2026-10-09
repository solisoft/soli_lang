//! `soli edge build`: see `solilang::edge_build`.

use std::path::Path;
use std::process;

pub fn build(folder: &str, out: Option<&str>, runtime: Option<&str>) {
    let app_dir = Path::new(folder);
    let out_dir = out
        .map(Path::new)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| app_dir.join("dist").join("edge"));
    let summary = match solilang::edge_build::build(&solilang::edge_build::Options {
        app_dir,
        out_dir: &out_dir,
        runtime_dir: runtime.map(Path::new),
    }) {
        Ok(summary) => summary,
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    };

    println!(
        "Packaged {} source files ({} KB) and {} static assets into {}",
        summary.source_files,
        summary.source_bytes / 1024,
        summary.asset_files,
        summary.out_dir.display()
    );
    println!("Runtime: {:.1} MB of wasm", summary.wasm_bytes as f64 / 1e6);
    for skipped in &summary.skipped {
        println!(
            "  skipped {} (not UTF-8 text; binary files belong in public/)",
            skipped.display()
        );
    }
    if summary.wrote_wrangler_toml {
        println!("Wrote wrangler.toml (later builds keep your edits)");
    }
    println!();
    println!("Next:");
    println!("  cd {}", summary.out_dir.display());
    println!("  npx wrangler dev      # run it locally on workerd");
    println!("  npx wrangler deploy   # ship it");
}
