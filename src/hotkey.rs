use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyManager, GlobalHotKeyEvent};

use crate::ui_event::{push, SharedEvents, UiEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeySpec {
    pub modifiers: Modifiers,
    pub key: Code,
}

impl Default for HotkeySpec {
    fn default() -> Self {
        Self {
            modifiers: Modifiers::SUPER,
            key: Code::F8,
        }
    }
}

impl std::fmt::Display for HotkeySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show every modifier in the combo, not just the first.
        let mut parts: Vec<&str> = Vec::new();
        if self.modifiers.contains(Modifiers::SUPER) {
            parts.push("Win");
        }
        if self.modifiers.contains(Modifiers::CONTROL) {
            parts.push("Ctrl");
        }
        if self.modifiers.contains(Modifiers::ALT) {
            parts.push("Alt");
        }
        if self.modifiers.contains(Modifiers::SHIFT) {
            parts.push("Shift");
        }
        let key = key_label(self.key);
        parts.push(key.as_str());
        write!(f, "{}", parts.join("+"))
    }
}

pub fn key_label(code: Code) -> String {
    // Only the labels we accept in the config; keeps Display in sync with parse.
    match code {
        Code::F1 => "F1",
        Code::F2 => "F2",
        Code::F3 => "F3",
        Code::F4 => "F4",
        Code::F5 => "F5",
        Code::F6 => "F6",
        Code::F7 => "F7",
        Code::F8 => "F8",
        Code::F9 => "F9",
        Code::F10 => "F10",
        Code::F11 => "F11",
        Code::F12 => "F12",
        Code::Space => "Space",
        Code::Tab => "Tab",
        Code::Enter => "Enter",
        Code::KeyA => "A",
        Code::KeyB => "B",
        Code::KeyC => "C",
        Code::KeyD => "D",
        Code::KeyE => "E",
        Code::KeyF => "F",
        Code::KeyG => "G",
        Code::KeyH => "H",
        Code::KeyI => "I",
        Code::KeyJ => "J",
        Code::KeyK => "K",
        Code::KeyL => "L",
        Code::KeyM => "M",
        Code::KeyN => "N",
        Code::KeyO => "O",
        Code::KeyP => "P",
        Code::KeyQ => "Q",
        Code::KeyR => "R",
        Code::KeyS => "S",
        Code::KeyT => "T",
        Code::KeyU => "U",
        Code::KeyV => "V",
        Code::KeyW => "W",
        Code::KeyX => "X",
        Code::KeyY => "Y",
        Code::KeyZ => "Z",
        Code::Digit0 => "0",
        Code::Digit1 => "1",
        Code::Digit2 => "2",
        Code::Digit3 => "3",
        Code::Digit4 => "4",
        Code::Digit5 => "5",
        Code::Digit6 => "6",
        Code::Digit7 => "7",
        Code::Digit8 => "8",
        Code::Digit9 => "9",
        _ => "?",
    }
    .to_string()
}

/// Parse strings like `SUPER` / `CONTROL|SHIFT` and `F8` / `J` from the config.
pub fn parse(mods: &str, key: &str) -> Option<HotkeySpec> {
    let mut m = Modifiers::empty();
    for part in mods
        .split(['|', '+'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        m |= match part.to_ascii_uppercase().as_str() {
            "SUPER" | "META" | "WIN" | "WINDOWS" => Modifiers::SUPER,
            "CTRL" | "CONTROL" => Modifiers::CONTROL,
            "ALT" => Modifiers::ALT,
            "SHIFT" => Modifiers::SHIFT,
            _ => return None,
        };
    }
    if m.is_empty() {
        return None;
    }
    Some(HotkeySpec { modifiers: m, key: parse_key(key)? })
}

fn parse_key(key: &str) -> Option<Code> {
    let k = key.trim().to_ascii_uppercase();
    Some(match k.as_str() {
        "SPACE" => Code::Space,
        "TAB" => Code::Tab,
        "ENTER" => Code::Enter,
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        _ => {
            let mut chars = k.chars();
            let c = chars.next()?;
            if !c.is_ascii_uppercase() || chars.next().is_some() {
                return None;
            }
            match c {
                'A' => Code::KeyA,
                'B' => Code::KeyB,
                'C' => Code::KeyC,
                'D' => Code::KeyD,
                'E' => Code::KeyE,
                'F' => Code::KeyF,
                'G' => Code::KeyG,
                'H' => Code::KeyH,
                'I' => Code::KeyI,
                'J' => Code::KeyJ,
                'K' => Code::KeyK,
                'L' => Code::KeyL,
                'M' => Code::KeyM,
                'N' => Code::KeyN,
                'O' => Code::KeyO,
                'P' => Code::KeyP,
                'Q' => Code::KeyQ,
                'R' => Code::KeyR,
                'S' => Code::KeyS,
                'T' => Code::KeyT,
                'U' => Code::KeyU,
                'V' => Code::KeyV,
                'W' => Code::KeyW,
                'X' => Code::KeyX,
                'Y' => Code::KeyY,
                'Z' => Code::KeyZ,
                _ => return None,
            }
        }
    })
}

pub fn register(manager: &GlobalHotKeyManager, spec: HotkeySpec) -> anyhow::Result<()> {
    manager.register(HotKey::new(Some(spec.modifiers), spec.key))?;
    Ok(())
}

/// Inverse of `parse` for the modifiers part: a config string such as
/// "SUPER|ALT" that `parse` accepts again.
pub fn modifiers_to_config(mods: Modifiers) -> String {
    let mut parts = Vec::new();
    if mods.contains(Modifiers::SUPER) {
        parts.push("SUPER");
    }
    if mods.contains(Modifiers::CONTROL) {
        parts.push("CONTROL");
    }
    if mods.contains(Modifiers::ALT) {
        parts.push("ALT");
    }
    if mods.contains(Modifiers::SHIFT) {
        parts.push("SHIFT");
    }
    parts.join("|")
}

pub fn unregister(manager: &GlobalHotKeyManager, spec: HotkeySpec) {
    let _ = manager.unregister(HotKey::new(Some(spec.modifiers), spec.key));
}

/// Background capture for the remap UI: waits up to `timeout` for the user to
/// HOLD a key combo (at least one modifier plus a mappable key), then for them
/// to release it, and reports the resulting HotkeySpec. Bare keys are ignored.
pub fn spawn_capture(
    timeout: std::time::Duration,
    tx: std::sync::mpsc::Sender<Result<HotkeySpec, String>>,
) {
    std::thread::spawn(move || {
        use device_query::{DeviceQuery, DeviceState};
        use std::time::{Duration, Instant};

        let device = DeviceState::new();
        let deadline = Instant::now() + timeout;

        let mod_of = |k: &device_query::Keycode| -> Option<Modifiers> {
            Some(match k {
                device_query::Keycode::LMeta | device_query::Keycode::RMeta => Modifiers::SUPER,
                device_query::Keycode::LControl | device_query::Keycode::RControl => {
                    Modifiers::CONTROL
                }
                device_query::Keycode::LAlt | device_query::Keycode::RAlt => Modifiers::ALT,
                device_query::Keycode::LShift | device_query::Keycode::RShift => Modifiers::SHIFT,
                _ => return None,
            })
        };
        let code_of = |k: &device_query::Keycode| -> Option<Code> {
            use device_query::Keycode as K;
            Some(match k {
                K::Space => Code::Space,
                K::Tab => Code::Tab,
                K::Enter => Code::Enter,
                K::F1 => Code::F1,
                K::F2 => Code::F2,
                K::F3 => Code::F3,
                K::F4 => Code::F4,
                K::F5 => Code::F5,
                K::F6 => Code::F6,
                K::F7 => Code::F7,
                K::F8 => Code::F8,
                K::F9 => Code::F9,
                K::F10 => Code::F10,
                K::F11 => Code::F11,
                K::F12 => Code::F12,
                K::A => Code::KeyA,
                K::B => Code::KeyB,
                K::C => Code::KeyC,
                K::D => Code::KeyD,
                K::E => Code::KeyE,
                K::F => Code::KeyF,
                K::G => Code::KeyG,
                K::H => Code::KeyH,
                K::I => Code::KeyI,
                K::J => Code::KeyJ,
                K::K => Code::KeyK,
                K::L => Code::KeyL,
                K::M => Code::KeyM,
                K::N => Code::KeyN,
                K::O => Code::KeyO,
                K::P => Code::KeyP,
                K::Q => Code::KeyQ,
                K::R => Code::KeyR,
                K::S => Code::KeyS,
                K::T => Code::KeyT,
                K::U => Code::KeyU,
                K::V => Code::KeyV,
                K::W => Code::KeyW,
                K::X => Code::KeyX,
                K::Y => Code::KeyY,
                K::Z => Code::KeyZ,
                K::Key0 => Code::Digit0,
                K::Key1 => Code::Digit1,
                K::Key2 => Code::Digit2,
                K::Key3 => Code::Digit3,
                K::Key4 => Code::Digit4,
                K::Key5 => Code::Digit5,
                K::Key6 => Code::Digit6,
                K::Key7 => Code::Digit7,
                K::Key8 => Code::Digit8,
                K::Key9 => Code::Digit9,
                _ => return None,
            })
        };

        while Instant::now() < deadline {
            let held = device.get_keys();
            let mut mods = Modifiers::empty();
            let mut key: Option<Code> = None;
            for k in &held {
                if let Some(m) = mod_of(k) {
                    mods |= m;
                } else if key.is_none() {
                    key = code_of(k);
                }
            }
            if let Some(key) = key {
                if !mods.is_empty() {
                    // Combo held; wait for release so the hotkey does not fire
                    // the moment we register it.
                    let release_deadline = Instant::now() + Duration::from_secs(3);
                    while Instant::now() < release_deadline {
                        let still = device.get_keys();
                        let any = still.iter().any(|k| mod_of(k).is_some() || code_of(k).is_some());
                        if !any {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let _ = tx.send(Ok(HotkeySpec { modifiers: mods, key }));
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        let _ = tx.send(Err("No key combo captured (timed out)".into()));
    });
}

/// device_query keycodes for the hotkey combo, so capture can wait until the
/// user has fully released the keys before simulating Ctrl+C.
pub fn wait_keys(spec: HotkeySpec) -> Vec<device_query::Keycode> {
    let mut keys = Vec::new();
    if spec.modifiers.contains(Modifiers::SUPER) {
        keys.extend([device_query::Keycode::LMeta, device_query::Keycode::RMeta]);
    }
    if spec.modifiers.contains(Modifiers::CONTROL) {
        keys.extend([
            device_query::Keycode::LControl,
            device_query::Keycode::RControl,
        ]);
    }
    if spec.modifiers.contains(Modifiers::ALT) {
        keys.extend([device_query::Keycode::LAlt, device_query::Keycode::RAlt]);
    }
    if spec.modifiers.contains(Modifiers::SHIFT) {
        keys.extend([
            device_query::Keycode::LShift,
            device_query::Keycode::RShift,
        ]);
    }
    use device_query::Keycode as K;
    keys.push(match spec.key {
        Code::Space => K::Space,
        Code::Tab => K::Tab,
        Code::Enter => K::Enter,
        Code::F1 => K::F1,
        Code::F2 => K::F2,
        Code::F3 => K::F3,
        Code::F4 => K::F4,
        Code::F5 => K::F5,
        Code::F6 => K::F6,
        Code::F7 => K::F7,
        Code::F8 => K::F8,
        Code::F9 => K::F9,
        Code::F10 => K::F10,
        Code::F11 => K::F11,
        Code::F12 => K::F12,
        Code::KeyA => K::A,
        Code::KeyB => K::B,
        Code::KeyC => K::C,
        Code::KeyD => K::D,
        Code::KeyE => K::E,
        Code::KeyF => K::F,
        Code::KeyG => K::G,
        Code::KeyH => K::H,
        Code::KeyI => K::I,
        Code::KeyJ => K::J,
        Code::KeyK => K::K,
        Code::KeyL => K::L,
        Code::KeyM => K::M,
        Code::KeyN => K::N,
        Code::KeyO => K::O,
        Code::KeyP => K::P,
        Code::KeyQ => K::Q,
        Code::KeyR => K::R,
        Code::KeyS => K::S,
        Code::KeyT => K::T,
        Code::KeyU => K::U,
        Code::KeyV => K::V,
        Code::KeyW => K::W,
        Code::KeyX => K::X,
        Code::KeyY => K::Y,
        Code::KeyZ => K::Z,
        Code::Digit0 => K::Key0,
        Code::Digit1 => K::Key1,
        Code::Digit2 => K::Key2,
        Code::Digit3 => K::Key3,
        Code::Digit4 => K::Key4,
        Code::Digit5 => K::Key5,
        Code::Digit6 => K::Key6,
        Code::Digit7 => K::Key7,
        Code::Digit8 => K::Key8,
        Code::Digit9 => K::Key9,
        _ => return keys,
    });
    keys
}

/// Wake the egui loop whenever the hotkey fires; the event itself is routed
/// through the shared queue and consumed in `App::update`.
pub fn spawn_watcher(ctx: egui::Context, events: SharedEvents) {
    std::thread::spawn(move || loop {
        match GlobalHotKeyEvent::receiver().recv() {
            Ok(ev) => {
                if ev.state() == global_hotkey::HotKeyState::Pressed {
                    push(&events, UiEvent::HotkeyPressed);
                    ctx.request_repaint();
                }
            }
            Err(_) => break,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defaults_and_aliases() {
        assert_eq!(parse("SUPER", "F8"), Some(HotkeySpec::default()));
        assert_eq!(parse(" win ", "j").map(|s| s.key), Some(Code::KeyJ));
        assert_eq!(parse("CONTROL|SHIFT", "K").is_some(), true);
        assert_eq!(parse("", "F8"), None, "no modifiers");
        assert_eq!(parse("SUPER", "F13"), None, "unsupported key");
        assert_eq!(parse("SUPER", "hello"), None);
    }

    #[test]
    fn display_shows_friendly_label() {
        assert_eq!(HotkeySpec::default().to_string(), "Win+F8");
    }

    #[test]
    fn display_lists_every_modifier() {
        let spec = HotkeySpec {
            modifiers: Modifiers::SUPER | Modifiers::CONTROL | Modifiers::SHIFT,
            key: Code::KeyJ,
        };
        assert_eq!(spec.to_string(), "Win+Ctrl+Shift+J");
    }

    #[test]
    fn modifiers_to_config_roundtrips() {
        let combos = [
            Modifiers::SUPER,
            Modifiers::CONTROL | Modifiers::ALT,
            Modifiers::SHIFT | Modifiers::SUPER | Modifiers::CONTROL,
        ];
        for m in combos {
            let text = modifiers_to_config(m);
            assert_eq!(parse(&text, "F8").map(|s| s.modifiers), Some(m));
        }
    }
}
