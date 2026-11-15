//! Templates for `soli generate privacy` — export and erasure of what the app
//! holds about a user.

pub const PRIVACY_SERVICE: &str = include_str!("privacy/privacy_service.sl");
pub const PRIVACY_CONTROLLER: &str = include_str!("privacy/privacy_controller.sl");
pub const PRIVACY_SHOW_VIEW: &str = include_str!("privacy/privacy_show.html.slv");

pub const ROUTES_MARKER: &str = "# --- soli generate privacy ---";

pub fn routes_snippet() -> String {
    format!(
        r#"
{ROUTES_MARKER}
get("/privacy", "privacy#show")
post("/privacy/export", "privacy#download")
post("/privacy/erase", "privacy#erase")
"#
    )
}
