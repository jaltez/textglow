use std::time::{Duration, Instant};

use arboard::{Clipboard, ImageData};
use device_query::{DeviceQuery as _, Keycode};
use enigo::Direction::{Click, Press, Release};
use enigo::{Enigo, Keyboard, Key};

#[derive(Clone, Default)]
pub struct ClipboardSnapshot {
    pub text: Option<String>,
    pub image: Option<ImageData<'static>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureOutcome {
    /// Fresh text copied from the current selection via Ctrl+C.
    Selection(String),
    /// No selection change detected; the pre-existing clipboard text.
    Clipboard(String),
    /// Nothing usable was found.
    Empty,
}

impl CaptureOutcome {
    pub fn text(&self) -> Option<&str> {
        match self {
            CaptureOutcome::Selection(t) | CaptureOutcome::Clipboard(t) => Some(t),
            CaptureOutcome::Empty => None,
        }
    }
}

/// Clipboard access plus Ctrl+C/Ctrl+V simulation for selection capture and
/// paste-back. Lives on the main/UI thread.
pub struct Captor {
    clipboard: Clipboard,
    enigo: Enigo,
    device: device_query::DeviceState,
}

impl Captor {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            clipboard: Clipboard::new()?,
            enigo: Enigo::new(&enigo::Settings::default())?,
            device: device_query::DeviceState::new(),
        })
    }

    pub fn read_text(&mut self) -> Option<String> {
        self.clipboard.get_text().ok()
    }

    pub fn set_text(&mut self, text: &str) -> anyhow::Result<()> {
        self.clipboard.set_text(text.to_string())?;
        Ok(())
    }

    /// Capture the current selection from the focused app.
    ///
    /// Sequence: snapshot the user's clipboard, wait until the hotkey keys are
    /// physically released (otherwise the app sees Ctrl+<held-modifier>+C and
    /// ignores it), settle briefly, simulate Ctrl+C, then poll for fresh
    /// clipboard text — resending Ctrl+C once for slow apps.
    ///
    /// Returns the captured outcome plus a snapshot of the user's clipboard
    /// (kept for the final restore after a paste-back).
    pub fn capture_selection(
        &mut self,
        wait_keys: &[Keycode],
    ) -> (CaptureOutcome, ClipboardSnapshot) {
        let snap = self.snapshot();
        let before = snap.text.clone();

        self.wait_hotkey_released(wait_keys);
        std::thread::sleep(Duration::from_millis(30));
        self.send_ctrl_char('c');

        let deadline = Instant::now() + Duration::from_millis(600);
        let mut resent = false;
        let mut found: Option<String> = None;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            if !resent && Instant::now() >= deadline - Duration::from_millis(350) {
                resent = true;
                self.send_ctrl_char('c');
            }
            if let Ok(cur) = self.clipboard.get_text() {
                let changed = before.as_deref() != Some(cur.as_str());
                if changed && !cur.trim().is_empty() {
                    found = Some(cur);
                    break;
                }
            }
        }

        let outcome = match found {
            Some(t) => CaptureOutcome::Selection(t),
            None => match before.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                Some(t) => CaptureOutcome::Clipboard(t.to_string()),
                None => CaptureOutcome::Empty,
            },
        };

        // Put the user's clipboard back right away; the selection is already
        // in our buffer. The snapshot is still returned so the caller can
        // restore it once more after a paste-back.
        let snap_to_keep = snap.clone();
        apply_snapshot(&mut self.clipboard, &snap);
        (outcome, snap_to_keep)
    }

    /// Block until the hotkey combo is physically released (max 400 ms).
    fn wait_hotkey_released(&self, wait_keys: &[Keycode]) {
        let deadline = Instant::now() + Duration::from_millis(400);
        while !wait_keys.is_empty() && Instant::now() < deadline {
            let held = self.device.get_keys();
            if !wait_keys.iter().any(|k| held.contains(k)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn snapshot(&mut self) -> ClipboardSnapshot {
        ClipboardSnapshot {
            text: self.clipboard.get_text().ok(),
            image: self.clipboard.get_image().ok().map(|img| img.to_owned_img()),
        }
    }

    /// Send Ctrl+V into the currently focused window.
    pub fn paste(&mut self) {
        self.send_ctrl_char('v');
    }

    fn send_ctrl_char(&mut self, c: char) {
        let _ = self.enigo.key(Key::Control, Press);
        let _ = self.enigo.key(Key::Unicode(c), Click);
        let _ = self.enigo.key(Key::Control, Release);
    }
}

fn apply_snapshot(clipboard: &mut Clipboard, snap: &ClipboardSnapshot) {
    // arboard replaces clipboard content per call, so restore the most
    // significant format the user had rather than trying to merge formats.
    match (&snap.text, &snap.image) {
        (Some(t), _) => {
            let _ = clipboard.set_text(t.clone());
        }
        (None, Some(img)) => {
            let _ = clipboard.set_image(img.clone());
        }
        (None, None) => {
            let _ = clipboard.clear();
        }
    }
}

/// Restore the user's clipboard a moment after a paste-back, off the UI
/// thread, so the target app has time to read the pasted text.
pub fn restore_snapshot_in_background(snap: ClipboardSnapshot) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        if let Ok(mut cb) = Clipboard::new() {
            apply_snapshot(&mut cb, &snap);
        }
    });
}
