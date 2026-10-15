/*!
The main entry point into ripgrep.

VeloTerm vendor note: the actual logic lives in `crates/core/lib.rs`
(exposed as `ripgrep::rg_main()`); this file is the thin entry for the
standalone `rg` binary.
*/

fn main() {
    std::process::exit(ripgrep::rg_main());
}
