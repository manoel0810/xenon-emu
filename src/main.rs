//! xenon-emu entry point. The emulator itself lives in the `xenon_emu` library; `app` wires it together.

mod app;
mod ui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}
