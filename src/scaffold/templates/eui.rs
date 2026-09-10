//! EUI scaffolding templates, written by `soli new <app> --eui`.
//!
//! Only compiled with the `eui` feature: a build that cannot serve a
//! component has no use for the 3 000 lines of catalogue, and the flag
//! that writes them is refused there.

/// The reference catalogue, spec/03-widgets.md §4 — 150 widgets composed
/// from the primitives, all of it plain Soli.
///
/// Byte for byte the copy in the EUI repository at
/// `examples/demo-app/app/controllers/eui_builders.sl`, which is where it
/// is edited; `scripts/sync-catalogue.sh` there copies it here.
pub const EUI_BUILDERS: &str = include_str!("eui/eui_builders.sl");

/// A first component: a counter, its handler and its view.
pub const EUI_CONTROLLER: &str = include_str!("eui/eui_controller.sl");

/// Appended to `config/routes.sl`: the `router_eui` line for that
/// component, and how to point a client at it.
pub const EUI_ROUTES: &str = include_str!("eui/routes_eui.sl");
