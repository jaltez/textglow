#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod capture;
mod config;
mod diff;
mod history;
mod hotkey;
mod llm;
mod prompt;
mod providers;
mod startup;
mod tray;
mod ui_event;

fn main() -> Result<(), eframe::Error> {
    let smoke = std::env::args().any(|a| a == "--smoke");
    let demo = std::env::args().any(|a| a == "--demo");
    let demo_settings = std::env::args().any(|a| a == "--demo-settings");
    let demo_sbs = std::env::args().any(|a| a == "--demo-sbs");
    let demo_diff = std::env::args().any(|a| a == "--demo-diff");
    let demo_history = std::env::args().any(|a| a == "--demo-history");

    // Dev/demo runs coexist with a real instance.
    if !(smoke
        || demo
        || demo_settings
        || demo_sbs
        || demo_diff
        || demo_history)
        && !acquire_single_instance_lock()
    {
        eprintln!("TextGlow is already running (see the tray icon).");
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TextGlow")
            .with_decorations(false)
            .with_visible(false)
            .with_resizable(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_inner_size([880.0, 700.0]),
        ..Default::default()
    };

    eframe::run_native(
        "TextGlow",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::TextGlowApp::new(
                cc,
                smoke,
                demo,
                demo_settings,
                demo_sbs,
                demo_diff,
                demo_history,
            )))
        }),
    )
}

/// Named-mutex single instance guard. The handle is intentionally leaked so
/// the mutex lives for the whole process.
#[cfg(windows)]
fn acquire_single_instance_lock() -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = "TextGlow-SingleInstance-Mutex\0".encode_utf16().collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if handle.is_null() {
            return true; // don't block startup on an unexpected failure
        }
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

#[cfg(not(windows))]
fn acquire_single_instance_lock() -> bool {
    // TODO(iteration 2): flock / abstract socket for macOS/Linux.
    true
}
