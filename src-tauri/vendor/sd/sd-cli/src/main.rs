/*!
The main entry point into the sd CLI.

VeloTerm vendor note: the actual logic lives in `sd-cli/src/lib.rs`
(exposed as `sd_cli::sd_main()`); this file is the thin entry for the
standalone `sd` binary.
*/

fn main() {
    std::process::exit(sd_cli::sd_main());
}
