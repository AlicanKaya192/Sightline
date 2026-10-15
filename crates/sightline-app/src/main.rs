#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = cli::parse_args(std::env::args().skip(1))?;
    let window = AppWindow::new()?;
    // Slint picks the system language on its own; the flag only overrides it.
    if let Some(language) = &options.language {
        slint::select_bundled_translation(language)?;
    }
    window.run()?;
    Ok(())
}
