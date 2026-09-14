//! EUI scaffolding templates, written by `soli new <app> --eui`.
//!
//! Only compiled with the `eui` feature: a build that cannot serve a
//! component has no use for the 8 000 lines of catalogue, and the flag
//! that writes them is refused there.

/// The reference catalogue, spec/03-widgets.md §4 — 343 widgets composed
/// from the primitives, all of it plain Soli.
///
/// Four files, byte for byte the copies in the EUI repository under
/// `examples/demo-app/app/controllers/`, which is where they are edited;
/// `scripts/sync-catalogue.sh` there copies them here. They load into one
/// namespace and none depends on which loads first, so the order below is
/// only the order a new application sees them listed in.
pub const EUI_BUILDERS: [(&str, &str); 4] = [
    ("eui_builders.sl", include_str!("eui/eui_builders.sl")),
    (
        "eui_builders_forms.sl",
        include_str!("eui/eui_builders_forms.sl"),
    ),
    (
        "eui_builders_charts.sl",
        include_str!("eui/eui_builders_charts.sl"),
    ),
    (
        "eui_builders_feed.sl",
        include_str!("eui/eui_builders_feed.sl"),
    ),
];

/// A first component: a counter, its handler and its view.
pub const EUI_CONTROLLER: &str = include_str!("eui/eui_controller.sl");

/// Appended to `config/routes.sl`: the `router_eui` line for that
/// component, and how to point a client at it.
pub const EUI_ROUTES: &str = include_str!("eui/routes_eui.sl");
