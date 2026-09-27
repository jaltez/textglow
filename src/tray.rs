use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::ui_event::{push, SharedEvents, UiEvent};

pub const ID_OPEN: &str = "tg-open";
pub const ID_SETTINGS: &str = "tg-settings";
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
    let autostart =
        CheckMenuItem::with_id(ID_AUTOSTART, "Start with Windows", true, initial_autostart, None);
    let separator = PredefinedMenuItem::separator();
    let quit = MenuItem::with_id(ID_QUIT, "Quit", true, None);
    menu.append_items(&[&open, &settings, &autostart, &separator, &quit])?;

    let icon = Icon::from_rgba(
        glow_icon_rgba(32, [118.0, 60.0, 230.0], [255.0, 64.0, 150.0]),
        32,
        32,
    )?;
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(tooltip)
        .with_icon(icon)
        .build()?;
    Ok(TrayHandles { _icon: icon, autostart_item: autostart })
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
    fn palette(&self) -> ([f32; 3], [f32; 3]) {
        match self {
            // Violet to pink: the brand look.
            TrayState::Ready => ([118.0, 60.0, 230.0], [255.0, 64.0, 150.0]),
            // Gray: not configured yet.
            TrayState::Unconfigured => ([125.0, 125.0, 132.0], [205.0, 205.0, 214.0]),
            // Red: something needs attention (hotkey conflict).
            TrayState::HotkeyFailed => ([225.0, 70.0, 70.0], [255.0, 125.0, 110.0]),
        }
    }
}

impl TrayHandles {
    /// Swap icon color and tooltip when the app health changes.
    pub fn set_state(&self, state: TrayState, hotkey_label: &str) {
        let (low, high) = state.palette();
        let tip = match state {
            TrayState::Ready => format!("TextGlow ({hotkey_label}): ready"),
            TrayState::Unconfigured => {
                format!("TextGlow ({hotkey_label}): not configured, open Settings")
            }
            TrayState::HotkeyFailed => {
                format!("TextGlow ({hotkey_label}): hotkey not registered, open Settings")
            }
        };
        if let Ok(icon) = Icon::from_rgba(glow_icon_rgba(32, low, high), 32, 32) {
            let _ = self._icon.set_icon(Some(icon));
        }
        let _ = self._icon.set_tooltip(Some(tip));
    }
}

/// A simple gradient "glow" square as the tray icon (no bundled assets).
pub fn glow_icon_rgba(size: u32, low: [f32; 3], high: [f32; 3]) -> Vec<u8> {
    let s = size as f32;
    let radius = s * 0.25;
    let half = s / 2.0;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            // Rounded-rect signed distance.
            let dx = (px - half).abs() - (half - radius);
            let dy = (py - half).abs() - (half - radius);
            let dist = (dx.max(0.0).hypot(dy.max(0.0)) + dx.min(0.0).max(dy.min(0.0))) - radius;
            let coverage = (0.5 - dist).clamp(0.0, 1.0);
            // Diagonal gradient violet -> pink.
            let t = (px + py) / (2.0 * s);
            let base = [
                low[0] + (high[0] - low[0]) * t,
                low[1] + (high[1] - low[1]) * t,
                low[2] + (high[2] - low[2]) * t,
            ];
            // Soft glow highlight off-center.
            let gx = px - s * 0.62;
            let gy = py - s * 0.68;
            let glow = (-((gx * gx + gy * gy) / (s * s * 0.06))).exp() * 0.65;
            let col = [
                base[0] + (high[0] - base[0]) * glow,
                base[1] + (high[1] - base[1]) * glow,
                base[2] + (high[2] - base[2]) * glow,
            ];
            rgba.extend_from_slice(&[
                col[0].round() as u8,
                col[1].round() as u8,
                col[2].round() as u8,
                (coverage * 255.0).round() as u8,
            ]);
        }
    }
    rgba
}
