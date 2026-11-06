//! Desktop entry point. Networking and file operations never run on the UI thread.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod gui;
mod settings;
mod theme;

fn open_window() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([860.0, 680.0])
            .with_min_inner_size([600.0, 480.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        link_resolver::APP_NAME,
        options,
        Box::new(|creation| {
            Ok(Box::new(gui::RedirectApp::new(
                &creation.egui_ctx,
                creation.storage,
            )))
        }),
    )
}

fn main() {
    if let Err(error) = open_window() {
        let message = format!(
            "Could not open {}: {error}\n\nA graphical desktop with OpenGL support is required. You can also use the included link-resolver-cli application.",
            link_resolver::APP_NAME
        );
        eprintln!("{message}");
        rfd::MessageDialog::new()
            .set_title(format!("{} could not start", link_resolver::APP_NAME))
            .set_description(&message)
            .set_level(rfd::MessageLevel::Error)
            .show();
        std::process::exit(1);
    }
}
