use std::sync::{Arc, Mutex};

/// Events from background sources (global hotkey, tray) that must be handled
/// on the egui main thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEvent {
    /// The global glow-up hotkey was pressed.
    HotkeyPressed,
    /// Tray: open popup with clipboard text.
    TrayOpen,
    /// Tray: open settings.
    TraySettings,
    /// Tray: "Start with Windows" checkbox toggled.
    TrayAutostartToggled,
    /// Tray: quit.
    TrayQuit,
}

pub type SharedEvents = Arc<Mutex<Vec<UiEvent>>>;

pub fn push(events: &SharedEvents, ev: UiEvent) {
    if let Ok(mut q) = events.lock() {
        q.push(ev);
    }
}
