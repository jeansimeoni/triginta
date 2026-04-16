use anyhow::Result;

fn main() -> Result<()> {
    // In C this would usually be `int main(void)` plus explicit error codes.
    // Rust programs commonly return `Result` from `main`, which lets the `?`
    // operator propagate failures instead of manually checking every call.
    triginta::app::run(debug_run_options())
}

#[cfg(debug_assertions)]
fn debug_run_options() -> triginta::app::RunOptions {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let force_ascii = args.iter().any(|arg| arg == "--ascii");
    let force_short_timer = args.iter().any(|arg| arg == "--short-timer");
    let reset_data = args.iter().any(|arg| arg == "--reset-data");
    let dry_run_sync = args.iter().any(|arg| arg == "--dry-run-sync");
    triginta::app::RunOptions {
        force_ascii,
        force_short_timer,
        reset_data,
        dry_run_sync,
    }
}

#[cfg(not(debug_assertions))]
fn debug_run_options() -> triginta::app::RunOptions {
    triginta::app::RunOptions::default()
}
