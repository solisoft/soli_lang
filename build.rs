//! Link the `soli` executable as a fixed-address (non-PIE) binary on Linux.
//!
//! As a position-independent executable it carried 160,000 relative
//! relocations, and the loader wrote every one of them at each start: 3.3 MB of
//! `.data.rel.ro`, about 800 copy-on-write page faults, 2.5 ms before `main`.
//! That was most of the start of a script (`soli -e 1`: 6.8 ms → 2.8 ms with
//! the type checker's own fix) and of every `soli build tool.sl` executable.
//! Linked at a fixed address there is nothing to relocate, and pages load
//! when first touched.
//!
//! The trade-off, chosen deliberately: the executable's own image is no
//! longer randomised (ASLR). Shared libraries, the stack, the heap and JIT
//! memory still are. The code is still compiled position-independent; only
//! the final link changes, and only for the binaries — the library and its
//! tests are untouched. macOS (which requires PIE on arm64) and Windows are
//! not affected.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-no-pie");
    }
}
