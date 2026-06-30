//! CLI: render a PDF from a JSON template + data, optionally embedding a
//! Factur-X CII XML to produce a PDF/A-3b invoice.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use soli_pdf::{facturx, render_with_warnings, FacturxMetadata, Invoice, Profile, RenderOptions};
use time::OffsetDateTime;

#[derive(Parser)]
#[command(
    name = "render_pdf",
    about = "Render a JSON template + data to a PDF, optionally embedding Factur-X XML."
)]
struct Args {
    /// Path to the JSON layout template.
    #[arg(long)]
    template: PathBuf,
    /// Path to the JSON data document (for free-form template + data renders).
    #[arg(long, required_unless_present = "invoice")]
    data: Option<PathBuf>,
    /// Optional Factur-X CII XML to embed (produces a PDF/A-3b invoice).
    #[arg(long)]
    xml: Option<PathBuf>,
    /// Path to a typed invoice JSON. Drives both the PDF and a computed,
    /// consistent EN 16931 CII XML — no separate --data/--xml needed.
    #[arg(long, conflicts_with_all = ["data", "xml"])]
    invoice: Option<PathBuf>,
    /// Factur-X profile (minimum, basicwl, basic, en16931, extended).
    #[arg(long, default_value = "en16931")]
    profile: String,
    /// Output PDF path.
    #[arg(long, short)]
    out: PathBuf,
    /// Do not fetch http(s) images (skip them instead).
    #[arg(long)]
    no_images: bool,
    /// Document title for Factur-X metadata.
    #[arg(long)]
    title: Option<String>,
    /// Directory of fonts to load (repeatable). No fonts are bundled, so at
    /// least one font must be available. Defaults to ./fonts and ./font.
    #[arg(long = "font-dir")]
    font_dir: Vec<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(warnings) => {
            for w in &warnings {
                eprintln!("warning: {w}");
            }
            eprintln!("wrote {}", args.out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> soli_pdf::Result<Vec<soli_pdf::RenderWarning>> {
    let template = std::fs::read(&args.template)?;
    let font_dirs = if args.font_dir.is_empty() {
        vec![PathBuf::from("fonts"), PathBuf::from("font")]
    } else {
        args.font_dir.clone()
    };
    let opts = RenderOptions {
        fetch_images: !args.no_images,
        font_dirs,
        ..Default::default()
    };

    let profile = Profile::parse(&args.profile).unwrap_or_default();
    let meta = FacturxMetadata {
        title: args.title.clone().unwrap_or_else(|| "Invoice".to_string()),
        created: OffsetDateTime::now_utc(),
        ..Default::default()
    };

    // Single-source path: a typed invoice drives both the PDF and its CII XML.
    if let Some(invoice_path) = &args.invoice {
        let invoice = Invoice::parse(&std::fs::read(invoice_path)?)?;
        let data = serde_json::to_vec(&invoice.to_render_data())?;
        let rendered = render_with_warnings(&template, &data, &opts)?;
        let xml = invoice.to_cii_xml(profile)?;
        let pdf = facturx::embed_facturx(&rendered.pdf, xml.as_bytes(), profile, &meta)?;
        std::fs::write(&args.out, &pdf)?;
        return Ok(rendered.warnings);
    }

    // Free-form path: template + data, optionally embedding caller-provided XML.
    let data = std::fs::read(args.data.as_ref().expect("clap requires data here"))?;
    let rendered = render_with_warnings(&template, &data, &opts)?;
    let pdf = match &args.xml {
        Some(xml_path) => {
            let xml = std::fs::read(xml_path)?;
            facturx::embed_facturx(&rendered.pdf, &xml, profile, &meta)?
        }
        None => rendered.pdf.clone(),
    };
    std::fs::write(&args.out, &pdf)?;
    Ok(rendered.warnings)
}
