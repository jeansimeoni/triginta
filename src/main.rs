use anyhow::Result;

fn main() -> Result<()> {
    // In C this would usually be `int main(void)` plus explicit error codes.
    // Rust programs commonly return `Result` from `main`, which lets the `?`
    // operator propagate failures instead of manually checking every call.
    triginta::app::run()
}
