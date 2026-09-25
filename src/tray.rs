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

    let icon = Icon::from_rgba(glow_icon_rgba(32), 32, 32)?;
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

/// A simple gradient "glow" square as the tray icon (no bundled assets).
pub fn glow_icon_rgba(size: u32) -> Vec<u8> {
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
                118.0 + (255.0 - 118.0) * t,
                60.0 + (64.0 - 60.0) * t,
                230.0 + (150.0 - 230.0) * t,
            ];
            // Soft glow highlight off-center.
            let gx = px - s * 0.62;
            let gy = py - s * 0.68;
            let glow = (-((gx * gx + gy * gy) / (s * s * 0.06))).exp() * 0.65;
            let col = [
                base[0] + (255.0 - base[0]) * glow,
                base[1] + (215.0 - base[1]) * glow,
                base[2] + (238.0 - base[2]) * glow,
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
