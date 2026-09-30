//! CLI: render a PDF from a JSON template + data, optionally embedding a
//! Factur-X CII XML to produce a PDF/A-3b invoice.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use soli_pdf::{facturx, render_with_warnings, FacturxMetadata, Invoice, Profile, RenderOptions};
use time::OffsetDateTime;

#[derive(Parser)]
#[command(
    name = "render_pdf",
    version,
    about = "Render a JSON template + data to a PDF, optionally embedding Factur-X XML."
)]
struct Args {
    /// Path to the JSON layout template (`-` reads stdin).
    #[arg(long)]
    template: PathBuf,
    /// Path to the JSON data document (`-` reads stdin) for free-form template + data renders.
    #[arg(long, required_unless_present = "invoice")]
    data: Option<PathBuf>,
    /// Optional Factur-X CII XML to embed (produces a PDF/A-3b invoice).
    #[arg(long)]
    xml: Option<PathBuf>,
    /// Path to a typed invoice JSON (`-` reads stdin). Drives both the PDF and a computed,
    /// consistent EN 16931 CII XML — no separate --data/--xml needed.
    #[arg(long, conflicts_with_all = ["data", "xml"])]
    invoice: Option<PathBuf>,
    /// Factur-X profile (minimum, basicwl, basic, en16931, extended).
    #[arg(long, default_value = "en16931")]
    profile: String,
    /// Output PDF path (`-` writes the PDF to stdout).
    #[arg(long, short)]
    out: PathBuf,
    /// Do not fetch http(s) images (skip them instead).
    #[arg(long)]
    no_images: bool,
    /// Document title (PDF Info dictionary; also used for Factur-X metadata).
    #[arg(long)]
    title: Option<String>,
    /// Document author (PDF Info dictionary).
    #[arg(long)]
    author: Option<String>,
    /// Document subject (PDF Info dictionary).
    #[arg(long)]
    subject: Option<String>,
    /// Letterhead PDF drawn beneath every page's content. Page 1 uses the
    /// letterhead's first page; later pages use its second page when present.
    #[arg(long)]
    stationery: Option<PathBuf>,
    /// Embed a file as an attachment (repeatable). Name = the file's basename;
    /// MIME guessed from the extension.
    #[arg(long = "attach")]
    attach: Vec<PathBuf>,
    /// User password (AES-128): required to open the document. Incompatible
    /// with Factur-X (--invoice/--xml).
    #[arg(long)]
    password: Option<String>,
    /// Owner password: lifts restrictions (defaults to --password).
    #[arg(long)]
    owner_password: Option<String>,
    /// Directory of fonts to load (repeatable). No fonts are bundled, so at
    /// least one font must be available. Defaults to ./fonts and ./font.
    #[arg(long = "font-dir")]
    font_dir: Vec<PathBuf>,
    /// Directory local images may be read from (repeatable). The working
    /// directory is always allowed; a path outside every allowed directory is
    /// skipped with a warning.
    #[arg(long = "image-dir")]
    image_dir: Vec<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    if let Err(e) = image_policy::install(&args.image_dir) {
        eprintln!("error: --image-dir: {e}");
        return ExitCode::FAILURE;
    }
    match run(&args) {
        Ok(warnings) => {
            for w in &warnings {
                eprintln!("warning: {w}");
            }
            if !is_stdio(&args.out) {
                eprintln!("wrote {}", args.out.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn is_stdio(path: &Path) -> bool {
    path.as_os_str() == "-"
}

/// Read a file, or stdin when the path is `-`.
fn read_input(path: &Path) -> std::io::Result<Vec<u8>> {
    if !is_stdio(path) {
        return std::fs::read(path);
    }
    let mut bytes = Vec::new();
    std::io::stdin().lock().read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Write a file, or stdout when the path is `-`.
fn write_output(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if !is_stdio(path) {
        return std::fs::write(path, bytes);
    }
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(bytes)?;
    stdout.flush()
}

fn run(args: &Args) -> soli_pdf::Result<Vec<soli_pdf::RenderWarning>> {
    let stdin_inputs = [
        Some(&args.template),
        args.data.as_ref(),
        args.invoice.as_ref(),
    ]
    .into_iter()
    .flatten()
    .filter(|path| is_stdio(path))
    .count();
    if stdin_inputs > 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "stdin (`-`) can only feed one of --template, --data and --invoice",
        )
        .into());
    }

    let template = read_input(&args.template)?;
    let font_dirs = if args.font_dir.is_empty() {
        vec![PathBuf::from("fonts"), PathBuf::from("font")]
    } else {
        args.font_dir.clone()
    };
    let opts = RenderOptions {
        fetch_images: !args.no_images,
        font_dirs,
        title: args.title.clone(),
        author: args.author.clone(),
        subject: args.subject.clone(),
        stationery: match &args.stationery {
            Some(path) => Some(std::fs::read(path)?),
            None => None,
        },
        encrypt: args
            .password
            .as_ref()
            .or(args.owner_password.as_ref())
            .map(|_| soli_pdf::EncryptOptions {
                user_password: args.password.clone().unwrap_or_default(),
                owner_password: args.owner_password.clone().unwrap_or_default(),
                allow: Vec::new(),
            }),
        attachments: args
            .attach
            .iter()
            .map(|path| {
                let bytes = std::fs::read(path)?;
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "attachment".to_string());
                let mime = match name.rsplit('.').next() {
                    Some("xml") => "text/xml",
                    Some("csv") => "text/csv",
                    Some("json") => "application/json",
                    Some("txt") => "text/plain",
                    Some("pdf") => "application/pdf",
                    _ => "application/octet-stream",
                }
                .to_string();
                Ok(soli_pdf::Attachment { name, mime, bytes })
            })
            .collect::<soli_pdf::Result<Vec<_>>>()?,
        ..Default::default()
    };

    let profile = Profile::parse(&args.profile).unwrap_or_default();
    let meta = FacturxMetadata {
        title: args.title.clone().unwrap_or_else(|| "Invoice".to_string()),
        author: args.author.clone().unwrap_or_default(),
        subject: args.subject.clone().unwrap_or_default(),
        created: OffsetDateTime::now_utc(),
        ..Default::default()
    };

    // Single-source path: a typed invoice drives both the PDF and its CII XML.
    if let Some(invoice_path) = &args.invoice {
        let invoice = Invoice::parse(&read_input(invoice_path)?)?;
        let data = serde_json::to_vec(&invoice.to_render_data())?;
        let rendered = render_with_warnings(&template, &data, &opts)?;
        let xml = invoice.to_cii_xml(profile)?;
        let pdf = facturx::embed_facturx(&rendered.pdf, xml.as_bytes(), profile, &meta)?;
        write_output(&args.out, &pdf)?;
        return Ok(rendered.warnings);
    }

    // Free-form path: template + data, optionally embedding caller-provided XML.
    let data = read_input(args.data.as_ref().expect("clap requires data here"))?;
    let rendered = render_with_warnings(&template, &data, &opts)?;
    let pdf = match &args.xml {
        Some(xml_path) => {
            let xml = std::fs::read(xml_path)?;
            facturx::embed_facturx(&rendered.pdf, &xml, profile, &meta)?
        }
        None => rendered.pdf.clone(),
    };
    write_output(&args.out, &pdf)?;
    Ok(rendered.warnings)
}

/// Which image sources the binary may load.
///
/// The library refuses every file and network source until its host installs
/// a policy: `soli` installs its SSRF validator and app-root jail, and this
/// binary installed nothing, so it could only draw `data:` URIs. Its policy
/// mirrors soli's, because a template often carries user data and an image
/// source is the field most likely to be user-influenced:
///
/// - a local path is read only if it resolves (symlinks included) inside the
///   working directory or an `--image-dir`, so `/etc/passwd` or `../../secret`
///   cannot end up embedded in a document;
/// - an `http(s)` URL is fetched only if its host resolves to public addresses,
///   and the connection is pinned to the address that was checked (a second
///   DNS answer could point at 127.0.0.1). Redirects are followed by hand, each
///   hop checked the same way.
mod image_policy {
    use std::io::Read as _;
    use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::Duration;

    const MAX_REDIRECTS: usize = 5;

    static ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();

    /// Install the policy. Fails if the working directory or an `--image-dir`
    /// does not exist: a typo there would otherwise skip every image quietly.
    pub fn install(image_dirs: &[PathBuf]) -> std::io::Result<()> {
        let mut roots = vec![std::env::current_dir()?.canonicalize()?];
        for dir in image_dirs {
            let root = dir
                .canonicalize()
                .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", dir.display())))?;
            roots.push(root);
        }
        let _ = ROOTS.set(roots);
        soli_pdf::images::set_image_source_guards(check_url, resolve_path);
        soli_pdf::images::set_image_fetcher(fetch);
        Ok(())
    }

    fn resolve_path(src: &str) -> Result<PathBuf, String> {
        let path = Path::new(src);
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path)
        };
        let resolved = absolute.canonicalize().map_err(|e| e.to_string())?;
        let roots = ROOTS.get().map(Vec::as_slice).unwrap_or(&[]);
        if roots.iter().any(|root| resolved.starts_with(root)) {
            Ok(resolved)
        } else {
            Err(format!(
                "{} is outside the working directory and every --image-dir",
                resolved.display()
            ))
        }
    }

    fn check_url(src: &str) -> Result<(), String> {
        public_target(src).map(|_| ())
    }

    /// The URL, its host, and a checked public address to connect to.
    fn public_target(src: &str) -> Result<(reqwest::Url, String, SocketAddr), String> {
        let url = reqwest::Url::parse(src).map_err(|e| e.to_string())?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("unsupported scheme {}", url.scheme()));
        }
        let host = url.host_str().ok_or("URL has no host")?.to_string();
        let port = url.port_or_known_default().ok_or("URL has no port")?;
        let lookup_host = host.trim_start_matches('[').trim_end_matches(']');
        let addrs: Vec<SocketAddr> = (lookup_host, port)
            .to_socket_addrs()
            .map_err(|e| format!("{host}: {e}"))?
            .collect();
        let first = *addrs
            .first()
            .ok_or_else(|| format!("{host} does not resolve"))?;
        if let Some(blocked) = addrs.iter().find(|addr| !is_public(addr.ip())) {
            return Err(format!(
                "{host} resolves to {}, which is not a public address",
                blocked.ip()
            ));
        }
        Ok((url, host, first))
    }

    fn fetch(src: &str, timeout: Duration, max_bytes: usize) -> Result<Vec<u8>, String> {
        let mut current = src.to_string();
        for _ in 0..=MAX_REDIRECTS {
            let (url, host, addr) = public_target(&current)?;
            let client = reqwest::blocking::Client::builder()
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .resolve(&host, addr)
                .build()
                .map_err(|e| e.to_string())?;
            let response = client.get(url.clone()).send().map_err(|e| e.to_string())?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or("redirect without a Location header")?;
                current = url.join(location).map_err(|e| e.to_string())?.to_string();
                continue;
            }
            let response = response.error_for_status().map_err(|e| e.to_string())?;
            if response
                .content_length()
                .is_some_and(|len| len > max_bytes as u64)
            {
                return Err(format!("image exceeds {max_bytes} bytes"));
            }
            let mut body = Vec::new();
            response
                .take(max_bytes as u64 + 1)
                .read_to_end(&mut body)
                .map_err(|e| e.to_string())?;
            if body.len() > max_bytes {
                return Err(format!("image exceeds {max_bytes} bytes"));
            }
            return Ok(body);
        }
        Err(format!("more than {MAX_REDIRECTS} redirects"))
    }

    /// Whether an address is reachable on the public internet: not loopback,
    /// private, link-local (cloud metadata lives at 169.254.169.254), shared
    /// (100.64/10), documentation, multicast or unspecified.
    fn is_public(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => {
                let [a, b, ..] = v4.octets();
                !(v4.is_private()
                    || v4.is_loopback()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
                    || v4.is_documentation()
                    || v4.is_multicast()
                    || a == 0
                    || a >= 240
                    || (a == 100 && (b & 0xc0) == 64))
            }
            IpAddr::V6(v6) => {
                if let Some(v4) = v6.to_ipv4_mapped() {
                    return is_public(IpAddr::V4(v4));
                }
                let [first, second, ..] = v6.segments();
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    || (first & 0xfe00) == 0xfc00
                    || (first & 0xffc0) == 0xfe80
                    || (first == 0x2001 && second == 0x0db8))
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::is_public;

        #[test]
        fn classifies_addresses() {
            for blocked in [
                "127.0.0.1",
                "10.1.2.3",
                "172.16.0.1",
                "192.168.1.1",
                "169.254.169.254",
                "100.64.0.1",
                "0.0.0.0",
                "::1",
                "fd00::1",
                "fe80::1",
                "::ffff:127.0.0.1",
            ] {
                assert!(!is_public(blocked.parse().unwrap()), "{blocked}");
            }
            for public in ["93.184.216.34", "1.1.1.1", "2606:4700::1111"] {
                assert!(is_public(public.parse().unwrap()), "{public}");
            }
        }
    }
}
