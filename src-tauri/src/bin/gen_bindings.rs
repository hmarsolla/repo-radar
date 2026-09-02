//! Regenerate `src/bindings.ts` from the Rust command and event
//! definitions. Run by `npm run bindings` and by CI (which then checks
//! `git diff --exit-code src/bindings.ts`). Kept separate from the library
//! so `cargo test` never links the Tauri runtime into a test harness.

fn main() {
    match repo_radar_lib::export_bindings() {
        Ok(()) => {
            eprintln!("wrote {}", repo_radar_lib::BINDINGS_PATH);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
