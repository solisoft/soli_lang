//! Soli CLI: Execute files or run the REPL.

mod cli;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() {
    cli::run();
}
