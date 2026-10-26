use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::ui_event::{push, SharedEvents, UiEvent};

pub const ID_OPEN: &str = "tg-open";
pub const ID_SETTINGS: &str = "tg-settings";
pub const ID_CHECK_UPDATES: &str = "tg-check-updates";
pub const ID_AUTOSTART: &str = "tg-autostart";
pub const ID_QUIT: &str = "tg-quit";

pub struct TrayHandles {
    /// Kept alive: dropping it removes the tray icon.
    pub _icon: TrayIcon,
    pub autostart_item: CheckMenuItem,
}

pub fn create(initial_autostart: bool, tooltip: &str) -> anyhow::Result<TrayHandles> {
    let menu = Menu::new();
    let open = MenuItem::with_id(ID_OPEN, "Open popup", true, None);
    let settings = MenuItem::with_id(ID_SETTINGS, "Settings", true, None);
    let check_updates = MenuItem::with_id(ID_CHECK_UPDATES, "Check for updates", true, None);
    let autostart = CheckMenuItem::with_id(
        ID_AUTOSTART,
        "Start with Windows",
        true,
        initial_autostart,
        None,
    );
    let separator = PredefinedMenuItem::separator();
    let quit = MenuItem::with_id(ID_QUIT, "Quit", true, None);
    menu.append_items(&[
        &open,
        &settings,
        &check_updates,
        &autostart,
        &separator,
        &quit,
    ])?;

    let icon = Icon::from_rgba(crate::brand::icon_rgba(32, &crate::brand::READY), 32, 32)?;
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(tooltip)
        .with_icon(icon)
        .build()?;
    Ok(TrayHandles {
        _icon: icon,
        autostart_item: autostart,
    })
}

/// Watch menu + icon events on background threads and wake the egui loop.
pub fn spawn_watchers(ctx: egui::Context, events: SharedEvents) {
    {
        let ctx = ctx.clone();
        let events = events.clone();
        std::thread::spawn(move || loop {
            match MenuEvent::receiver().recv() {
                Ok(ev) => {
                    let translated = match ev.id.0.as_str() {
                        ID_OPEN => Some(UiEvent::TrayOpen),
                        ID_SETTINGS => Some(UiEvent::TraySettings),
                        ID_CHECK_UPDATES => Some(UiEvent::TrayCheckUpdates),
                        ID_AUTOSTART => Some(UiEvent::TrayAutostartToggled),
                        ID_QUIT => Some(UiEvent::TrayQuit),
                        _ => None,
                    };
                    if let Some(ui_ev) = translated {
                        push(&events, ui_ev);
                        ctx.request_repaint();
                    }
                }
                Err(_) => break,
            }
        });
    }
    std::thread::spawn(move || loop {
        match TrayIconEvent::receiver().recv() {
            Ok(ev) => {
                if matches!(
                    ev,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                ) {
                    push(&events, UiEvent::TrayOpen);
                    ctx.request_repaint();
                }
            }
            Err(_) => break,
        }
    });
}

/// Health of the app, surfaced on the tray icon color and tooltip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Ready,
    Unconfigured,
    HotkeyFailed,
}

impl TrayState {
    fn theme(&self) -> &'static crate::brand::IconTheme {
        match self {
            // Amber: the brand look.
            TrayState::Ready => &crate::brand::READY,
            // Gray: not configured yet.
            TrayState::Unconfigured => &crate::brand::UNCONFIGURED,
            // Red: something needs attention (hotkey conflict).
            TrayState::HotkeyFailed => &crate::brand::HOTKEY_FAILED,
        }
    }
}

impl TrayHandles {
    /// Swap icon color and tooltip when the app health changes.
    pub fn set_state(&self, state: TrayState, hotkey_label: &str) {
        let tip = match state {
            TrayState::Ready => format!("TextGlow ({hotkey_label}): ready"),
            TrayState::Unconfigured => {
                format!("TextGlow ({hotkey_label}): not configured, open Settings")
            }
            TrayState::HotkeyFailed => {
                format!("TextGlow ({hotkey_label}): hotkey not registered, open Settings")
            }
        };
        if let Ok(icon) = Icon::from_rgba(crate::brand::icon_rgba(32, state.theme()), 32, 32) {
            let _ = self._icon.set_icon(Some(icon));
        }
        let _ = self._icon.set_tooltip(Some(tip));
    }
}
