use anyhow::Result;

fn main() -> Result<()> {
    // In C this would usually be `int main(void)` plus explicit error codes.
    // Rust programs commonly return `Result` from `main`, which lets the `?`
    // operator propagate failures instead of manually checking every call.
    triginta::app::run(debug_run_options())
}

#[cfg(debug_assertions)]
fn debug_run_options() -> triginta::app::RunOptions {
    let force_ascii = std::env::args().skip(1).any(|arg| arg == "--ascii");
    triginta::app::RunOptions { force_ascii }
}

#[cfg(not(debug_assertions))]
fn debug_run_options() -> triginta::app::RunOptions {
    triginta::app::RunOptions::default()
}
