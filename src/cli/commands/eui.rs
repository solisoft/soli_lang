//! `soli eui <url> [--allow cap,cap]` — open an EUI application in a native
//! window from this binary, the way the standalone `eui` client would.
//! Needs a soli built with `--features eui-desktop`; without it the command
//! says so rather than pretending.

use std::process;

/// Open `url` in a window, granting the named capabilities if the
/// application's manifest asks for them.
pub fn open(url: &str, allow: &[String]) {
    #[cfg(feature = "eui-desktop")]
    {
        let mut allowed = 0u32;
        for name in allow {
            match eui_proto::caps::from_name(name) {
                Some(bit) => allowed |= bit,
                None => {
                    eprintln!(
                        "Unknown capability '{}'. Known: {}",
                        name,
                        eui_proto::caps::NAMES
                            .iter()
                            .map(|(n, _)| *n)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    process::exit(64);
                }
            }
        }
        if let Err(e) = eui_client::app::run(url.to_string(), allowed) {
            eprintln!("eui: {}", e);
            process::exit(1);
        }
    }
    #[cfg(not(feature = "eui-desktop"))]
    {
        let _ = allow;
        eprintln!(
            "This soli was built without the EUI window. Rebuild with `cargo build --features eui-desktop`, \
             or run the standalone client: eui {}",
            url
        );
        process::exit(1);
    }
}
