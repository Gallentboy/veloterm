/*!
The main entry point into frawk.

VeloTerm vendor note: the actual logic lives in `src/lib.rs`
(exposed as `frawk::awk_main()`); this file is the thin entry for the
standalone `frawk` binary.
*/

fn main() {
    std::process::exit(frawk::awk_main());
}
