use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Context, Key, RichText, ViewportCommand};
use global_hotkey::GlobalHotKeyManager;

use crate::capture::{Captor, CaptureOutcome, ClipboardSnapshot};
use crate::config::{self, Config};
use crate::diff;
use crate::history::{self, HistoryEntry};
use crate::hotkey;
use crate::llm::{self, ChatMsg, LlmEvent, ModelsRequest, StreamRequest};
use crate::prompt::{self, Tone};
use crate::tray;
use crate::ui_event::{SharedEvents, UiEvent};

const POPUP_SIZE: egui::Vec2 = egui::vec2(880.0, 700.0);
const SETTINGS_SIZE: egui::Vec2 = egui::vec2(640.0, 700.0);
const HISTORY_SIZE: egui::Vec2 = egui::vec2(760.0, 720.0);
const WIZARD_SIZE: egui::Vec2 = egui::vec2(640.0, 560.0);

const DEMO_TEXT: &str = "hey can you fix this text real quick?? its kinda rough and i want it \
to sound better for my boss, thx!!";

const DEMO_RESULT: &str = "Hey, could you fix this text quickly? It's a bit rough, and I'd \
like it to sound better for my boss. Thanks!";

const THINKING_OPTIONS: &[(&str, &str)] = &[
    ("off", "Off (fastest)"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High (deepest)"),
];

fn thinking_label(level: &str) -> &'static str {
    THINKING_OPTIONS
        .iter()
        .find(|(v, _)| *v == level.trim().to_ascii_lowercase())
        .map(|(_, l)| *l)
        .unwrap_or("Medium")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Hidden,
    Popup,
    Settings,
    History,
    Wizard,
}

/// Persistent health, surfaced on the tray icon/tooltip and in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Health {
    Ready,
    Unconfigured,
    HotkeyFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ResultView {
    #[default]
    Sbs,
    Diff,
    Result,
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Editing,
    Streaming,
    Done,
    Failed(String),
}

/// An in-flight LLM request. The popup-attached run feeds the UI; runs that
/// outlive their popup session keep streaming in the background and land in
/// history when they finish (nothing is cancelled except on close/Cancel).
struct ActiveRun {
    rx: Receiver<LlmEvent>,
    source: String,
    tone: String,
    instruction: String,
    acc: String,
}

pub struct TextGlowApp {
    cfg: Config,
    screen: Screen,

    // popup state
    captured: String,
    instruction: String,
    tone: Tone,
    phase: Phase,
    result: String,
    history: Vec<ChatMsg>,
    run: Option<ActiveRun>,
    background: Vec<ActiveRun>,
    status: String,
    /// How to show the finished rewrite.
    result_view: ResultView,
    focus_instruction: bool,
    instruction_focused: bool,
    source_focused: bool,

    // the run currently in the popup, for SBS/diff and history entries
    run_source: String,
    run_tone: String,

    // history of past runs (newest first)
    run_history: Vec<HistoryEntry>,
    /// Whether the global hotkey registered successfully at startup.
    hotkey_ok: bool,
    /// Last health state pushed to the tray, to avoid re-setting every frame.
    last_health: Option<Health>,
    /// Which history entries are expanded (by index).
    history_expanded: HashSet<usize>,
    /// Synchronized scrolling state of the side-by-side panes.
    sbs_scroll: SyncScroll,
    /// Memoized word diff so the O(n*m) pass does not rerun every frame.
    diff_cache: diff::DiffCache,

    /// Set in new() when the wizard should auto-open (no config file).
    wizard_needed: bool,
    // first-run wizard
    wizard_step: u8,
    wizard_test_rx: Option<Receiver<Result<String, String>>>,
    wizard_test_status: String,
    /// Config snapshot taken when the wizard opened (Skip restores it).
    wizard_original: Option<Config>,

    // settings state
    models_rx: Option<Receiver<Result<Vec<String>, String>>>,
    models_filter: String,
    /// Waiting for the user to press a new hotkey combo (remap UI).
    hotkey_capture_rx: Option<Receiver<Result<hotkey::HotkeySpec, String>>>,
    models: Vec<String>,
    models_status: String,
    api_key_draft: String,
    api_key_saved: bool,
    autostart_enabled: bool,
    settings_status: String,

    // infra (main thread only)
    events: SharedEvents,
    tray: Option<tray::TrayHandles>,
    /// Kept alive so the hotkey registration stays active.
    _hotkey_mgr: GlobalHotKeyManager,
    hotkey_spec: hotkey::HotkeySpec,
    hotkey_label: String,
    captor: Option<Captor>,
    clipboard_backup: Option<ClipboardSnapshot>,
    /// Font size currently applied to the egui style (re-applied on change).
    applied_font_size: f32,
    frames: u64,
    smoke: bool,
    demo: bool,
    demo_settings: bool,
    demo_sbs: bool,
    demo_diff: bool,
    demo_history: bool,
}

impl TextGlowApp {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cc: &eframe::CreationContext,
        smoke: bool,
        demo: bool,
        demo_settings: bool,
        demo_sbs: bool,
        demo_diff: bool,
        demo_history: bool,
    ) -> Self {
        // The glow aesthetic reads best on dark, regardless of OS theme.
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        let mut cfg = config::load();
        let font_size = cfg.font_size;
        apply_style(&cc.egui_ctx, font_size);
        let spec = hotkey::parse(&cfg.hotkey_modifiers, &cfg.hotkey_key).unwrap_or_default();
        let hotkey_label = spec.to_string();

        let events: SharedEvents = Arc::new(Mutex::new(Vec::new()));
        let ctx = cc.egui_ctx.clone();

        let hotkey_mgr = GlobalHotKeyManager::new().expect("global hotkey manager");
        let hotkey_ok = match hotkey::register(&hotkey_mgr, spec) {
            Ok(()) => {
                crate::logging::info(&format!("hotkey registered: {hotkey_label}"));
                true
            }
            Err(e) => {
                let msg = format!("hotkey registration failed: {e:#}");
                eprintln!("textglow: {msg}");
                crate::logging::error(&msg);
                false
            }
        };
        hotkey::spawn_watcher(ctx.clone(), events.clone());

        let autostart_enabled = crate::startup::is_enabled();
        let tray_handles = tray::create(autostart_enabled, &format!("TextGlow ({hotkey_label})"))
            .map_err(|e| {
                let msg = format!("tray init failed: {e:#}");
                eprintln!("textglow: {msg}");
                crate::logging::error(&msg);
            })
            .ok();
        if tray_handles.is_some() {
            tray::spawn_watchers(ctx.clone(), events.clone());
        }

        let captor = Captor::new().map_err(|e| {
            let msg = format!("clipboard/input init failed: {e:#}");
            eprintln!("textglow: {msg}");
            crate::logging::error(&msg);
        })
        .ok();

        // First-run wizard: only when there is no config file at all (existing
        // users are never interrupted).
        let show_wizard = !config::config_file_exists();
        if !show_wizard && !cfg.wizard_done {
            cfg.wizard_done = true;
            let _ = config::save(&cfg);
        }
        crate::logging::info(&format!(
            "started (provider: {}, model: {}, first_run: {show_wizard})",
            cfg.provider,
            if cfg.model.is_empty() { "<none>" } else { &cfg.model },
        ));

        // Make sure at least one frame runs even though the window starts hidden.
        ctx.request_repaint();

        Self {
            cfg,
            screen: Screen::Hidden,
            captured: String::new(),
            instruction: String::new(),
            tone: Tone::GlowUp,
            phase: Phase::Editing,
            result: String::new(),
            history: Vec::new(),
            run: None,
            background: Vec::new(),
            status: String::new(),
            result_view: ResultView::Sbs,
            focus_instruction: false,
            instruction_focused: false,
            source_focused: false,
            run_source: String::new(),
            run_tone: String::new(),
            run_history: history::load(),
            history_expanded: HashSet::new(),
            sbs_scroll: SyncScroll::default(),
            diff_cache: diff::DiffCache::default(),
            models_rx: None,
            models_filter: String::new(),
            hotkey_capture_rx: None,
            models: Vec::new(),
            models_status: String::new(),
            api_key_draft: config::load_api_key().unwrap_or_default(),
            api_key_saved: false,
            autostart_enabled,
            settings_status: String::new(),
            events,
            tray: tray_handles,
            _hotkey_mgr: hotkey_mgr,
            hotkey_spec: spec,
            hotkey_ok,
            last_health: None,
            hotkey_label,
            wizard_needed: show_wizard,
            wizard_step: 1,
            wizard_test_rx: None,
            wizard_test_status: String::new(),
            wizard_original: None,
            captor,
            clipboard_backup: None,
            applied_font_size: font_size,
            frames: 0,
            smoke,
            demo,
            demo_settings,
            demo_sbs,
            demo_diff,
            demo_history,
        }
    }

    // ---- event plumbing ---------------------------------------------------

    fn drain_ui_events(&mut self, ctx: &Context) {
        let events = match self.events.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => return,
        };
        for ev in events {
            match ev {
                UiEvent::HotkeyPressed => self.open_popup(ctx, false),
                UiEvent::TrayOpen => self.open_popup(ctx, true),
                UiEvent::TraySettings => self.open_settings(ctx),
                UiEvent::TrayAutostartToggled => {
                    let on = !self.autostart_enabled;
                    self.set_autostart(on);
                }
                UiEvent::TrayQuit => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }

    fn drain_llm(&mut self) {
        // The popup-attached run feeds the UI directly.
        if let Some(mut run) = self.run.take() {
            let mut done: Option<String> = None;
            let mut failed: Option<String> = None;
            let mut pending = false;
            loop {
                match run.rx.try_recv() {
                    Ok(LlmEvent::Delta(d)) => {
                        run.acc.push_str(&d);
                        self.result.push_str(&d);
                    }
                    Ok(LlmEvent::Done(full)) => {
                        done = Some(full);
                        break;
                    }
                    Ok(LlmEvent::Error(e)) => {
                        failed = Some(e);
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        pending = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        failed = Some("stream ended unexpectedly".into());
                        break;
                    }
                }
            }
            if let Some(full) = done {
                self.result = full.clone();
                self.history.push(ChatMsg::assistant(full.clone()));
                self.phase = Phase::Done;
                self.focus_instruction = true;
                self.record_history(&run.tone, &run.instruction, &run.source, &full);
            } else if let Some(e) = failed {
                self.phase = Phase::Failed(e);
                // Keep the metadata around so Retry can reuse it.
                self.run = Some(run);
            } else if pending {
                self.run = Some(run);
            }
        }

        // Background runs (popup moved on / window hidden) finish silently
        // into the history; nothing cancels them except Quit.
        let mut i = 0;
        while i < self.background.len() {
            let mut finished: Option<String> = None;
            let mut drop_run = false;
            match self.background[i].rx.try_recv() {
                Ok(LlmEvent::Delta(d)) => self.background[i].acc.push_str(&d),
                Ok(LlmEvent::Done(full)) => finished = Some(full),
                Ok(LlmEvent::Error(_)) => drop_run = true,
                Err(std::sync::mpsc::TryRecvError::Empty) => i += 1,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => drop_run = true,
            }
            if finished.is_some() || drop_run {
                let run = self.background.remove(i);
                if let Some(full) = finished {
                    self.record_history(&run.tone, &run.instruction, &run.source, &full);
                }
            }
        }
    }

    fn drain_models(&mut self) {
        let Some(rx) = self.models_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(list)) => {
                self.models = list;
                self.models_status.clear();
            }
            Ok(Err(e)) => self.models_status = e,
            Err(std::sync::mpsc::TryRecvError::Empty) => self.models_rx = Some(rx),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.models_status = "model fetch ended unexpectedly".into();
            }
        }
    }

    // ---- window management --------------------------------------------------

    fn position_centered(&self, ctx: &Context, size: egui::Vec2) -> egui::Pos2 {
        // Prefer the monitor under the cursor (multi-monitor users invoke the
        // hotkey where they are working); fall back to the window's monitor.
        #[cfg(windows)]
        if let Some((mx, my, mw, mh)) = cursor_monitor_work_area() {
            return egui::pos2(
                mx + ((mw - size.x) / 2.0).max(0.0),
                my + ((mh - size.y) / 2.0).max(0.0),
            );
        }
        let mon = ctx
            .input(|i| i.viewport().monitor_size)
            .unwrap_or(egui::vec2(1920.0, 1080.0));
        egui::pos2(
            ((mon.x - size.x) / 2.0).max(0.0),
            ((mon.y - size.y) / 2.0).max(0.0),
        )
    }

    fn resize_and_show(&mut self, ctx: &Context, size: egui::Vec2) {
        let pos = self.position_centered(ctx, size);
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos));
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn show_only(&mut self, ctx: &Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn hide_window(&mut self, ctx: &Context) {
        self.screen = Screen::Hidden;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    // ---- popup ----------------------------------------------------------------

    fn open_popup(&mut self, ctx: &Context, from_tray: bool) {
        if self.screen == Screen::Popup {
            self.show_only(ctx);
            return;
        }
        // Any in-flight run keeps streaming into the history instead of
        // being cut by the new capture.
        self.detach_run();
        let mut warn = None;
        match (&mut self.captor, from_tray) {
            (Some(c), false) => {
                let wait_keys = hotkey::wait_keys(self.hotkey_spec);
                let (outcome, snap) = c.capture_selection(&wait_keys);
                match &outcome {
                    CaptureOutcome::Clipboard(_) => {
                        warn = Some("No selection found. Using clipboard text.");
                    }
                    CaptureOutcome::Empty => {
                        warn = Some("Nothing captured. Type or paste text below.");
                    }
                    CaptureOutcome::Selection(_) => {}
                }
                self.captured = outcome.text().unwrap_or_default().to_string();
                self.clipboard_backup = Some(snap);
            }
            (Some(c), true) => {
                self.captured = c.read_text().unwrap_or_default();
                self.clipboard_backup = None;
            }
            (None, _) => {
                self.captured = String::new();
                self.clipboard_backup = None;
                warn = Some("Clipboard/input init failed. Paste text manually.");
            }
        }
        self.reset_popup();
        if let Some(w) = warn {
            self.status = w.into();
        }
        self.screen = Screen::Popup;
        self.resize_and_show(ctx, POPUP_SIZE);
        self.focus_instruction = true;
    }

    fn health(&self) -> Health {
        if !self.hotkey_ok {
            Health::HotkeyFailed
        } else if self.cfg.base_url.trim().is_empty() || self.cfg.model.trim().is_empty() {
            Health::Unconfigured
        } else {
            Health::Ready
        }
    }

    fn health_text(&self) -> &'static str {
        match self.health() {
            Health::Ready => "Ready",
            Health::Unconfigured => "Not configured (open Settings)",
            Health::HotkeyFailed => "Hotkey not registered (open Settings)",
        }
    }

    /// Push health changes to the tray icon color + tooltip (cheap no-op when
    /// nothing changed; called every logic frame).
    fn push_health_to_tray(&mut self) {
        let health = self.health();
        if self.last_health != Some(health) {
            if let Some(t) = &self.tray {
                t.set_state(
                    match health {
                        Health::Ready => tray::TrayState::Ready,
                        Health::Unconfigured => tray::TrayState::Unconfigured,
                        Health::HotkeyFailed => tray::TrayState::HotkeyFailed,
                    },
                    &self.hotkey_label,
                );
            }
            self.last_health = Some(health);
        }
    }

    /// Non-UI drain for the wizard's connection test.
    fn drain_wizard_test(&mut self) {
        let Some(rx) = self.wizard_test_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(reply)) => {
                self.wizard_test_status = format!("Connection OK (model replied: {reply})");
            }
            Ok(Err(e)) => self.wizard_test_status = format!("Connection failed: {e}"),
            Err(std::sync::mpsc::TryRecvError::Empty) => self.wizard_test_rx = Some(rx),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.wizard_test_status = "Connection test ended unexpectedly".into();
            }
        }
        if self.wizard_test_rx.is_some() {
            self.wizard_test_status = "Testing connection...".into();
        }
    }

    fn finish_wizard(&mut self, ctx: &Context, keep_changes: bool) {
        if !keep_changes {
            if let Some(original) = self.wizard_original.take() {
                self.cfg = original;
            }
        }
        self.cfg.wizard_done = true;
        match config::save(&self.cfg).and_then(|_| config::store_api_key(self.api_key_draft.trim()))
        {
            Ok(()) => crate::logging::info("wizard finished (config saved)"),
            Err(e) => crate::logging::error(&format!("wizard save failed: {e:#}")),
        }
        self.wizard_needed = false;
        self.wizard_original = None;
        self.api_key_saved = !self.api_key_draft.trim().is_empty();
        self.hide_window(ctx);
    }

    // ---- first-run wizard ---------------------------------------------------------

    /// Three guided steps: provider, API key + model (with a connection test),
    /// confirm. Esc or Skip keeps nothing and marks the wizard done.
    fn wizard_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.finish_wizard(&ctx, false);
            return;
        }

        let mut back = false;
        let mut next = false;
        let mut finish = false;
        let mut skip = false;
        let mut test = false;
        let mut fetch = false;

        let frame = panel_frame(ui, 22);
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Welcome to TextGlow");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Skip").clicked() {
                        skip = true;
                    }
                });
            });
            ui.weak(format!("Step {} of 3", self.wizard_step));
            ui.add_space(10.0);

            match self.wizard_step {
                1 => {
                    ui.weak("Pick an AI provider. All of them speak the same protocol.");
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .id_salt("tg-wizard-providers")
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for p in crate::providers::PRESETS {
                                let selected = self.cfg.provider == p.id;
                                let summary = if p.needs_key {
                                    format!("{} (needs an API key)", p.label)
                                } else if p.base_url.is_empty() {
                                    format!("{} (your own endpoint)", p.label)
                                } else {
                                    format!("{} (local, no key)", p.label)
                                };
                                if ui
                                    .selectable_label(selected, summary)
                                    .on_hover_text(p.base_url)
                                    .clicked()
                                {
                                    self.cfg.provider = p.id.into();
                                    self.cfg.base_url = p.base_url.into();
                                    self.cfg.model = p.default_model.into();
                                    self.models.clear();
                                    self.models_status.clear();
                                }
                            }
                        });
                }
                2 => {
                    egui::Grid::new("tg-wizard-cred")
                        .num_columns(2)
                        .spacing([14.0, 10.0])
                        .show(ui, |ui| {
                            ui.strong("API key");
                            let needs_key = crate::providers::find(&self.cfg.provider)
                                .map(|p| p.needs_key)
                                .unwrap_or(false);
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.api_key_draft)
                                        .password(true)
                                        .desired_width(280.0)
                                        .hint_text(if needs_key {
                                            "paste your API key"
                                        } else {
                                            "not needed for this provider"
                                        }),
                                );
                                if self.api_key_saved {
                                    ui.weak(RichText::new("saved").small());
                                }
                            });
                            ui.end_row();

                            ui.strong("Model");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.cfg.model)
                                        .desired_width(240.0)
                                        .hint_text("model id"),
                                );
                                if ui.button("Fetch models").clicked() {
                                    fetch = true;
                                }
                            });
                            ui.end_row();
                        });
                    if !self.models.is_empty() {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.models_filter)
                                .desired_width(280.0)
                                .hint_text("filter by name..."),
                        );
                        let filtered = filter_models(&self.models, &self.models_filter);
                        egui::ScrollArea::vertical()
                            .id_salt("tg-wizard-models")
                            .max_height(120.0)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for m in filtered.iter().take(100) {
                                        if ui
                                            .selectable_label(self.cfg.model == *m, m)
                                            .clicked()
                                        {
                                            self.cfg.model = m.clone();
                                        }
                                    }
                                });
                            });
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Test connection").clicked() {
                            test = true;
                        }
                        ui.weak(RichText::new(&self.wizard_test_status).small());
                    });
                }
                _ => {
                    let provider = crate::providers::find(&self.cfg.provider)
                        .map(|p| p.label)
                        .unwrap_or("Custom");
                    egui::Grid::new("tg-wizard-summary")
                        .num_columns(2)
                        .spacing([14.0, 10.0])
                        .show(ui, |ui| {
                            ui.strong("Provider");
                            ui.label(provider);
                            ui.end_row();
                            ui.strong("Model");
                            ui.label(&self.cfg.model);
                            ui.end_row();
                            ui.strong("API key");
                            ui.label(if self.api_key_draft.trim().is_empty() {
                                "none"
                            } else {
                                "saved to Windows Credential Manager"
                            });
                            ui.end_row();
                            ui.strong("Hotkey");
                            ui.label(&self.hotkey_label);
                            ui.end_row();
                        });
                    ui.add_space(8.0);
                    ui.weak(
                        "Select text anywhere, press the hotkey, then press Enter. \
                         Everything can be changed later in Settings.",
                    );
                }
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if self.wizard_step > 1 && ui.button("Back").clicked() {
                        back = true;
                    }
                    if self.wizard_step < 3 {
                        if ui.button("Next").clicked() {
                            next = true;
                        }
                    } else if ui.button("Save and finish").clicked() {
                        finish = true;
                    }
                });
            });
        });

        if fetch {
            self.models_status = "fetching...".into();
            self.models_rx = Some(llm::spawn_fetch_models(ModelsRequest {
                base_url: self.cfg.base_url.clone(),
                api_key: (!self.api_key_draft.trim().is_empty())
                    .then(|| self.api_key_draft.clone()),
            }));
            ctx.request_repaint();
        }
        if test {
            self.wizard_test_status = "Testing connection...".into();
            self.wizard_test_rx = Some(llm::spawn_test(StreamRequest {
                provider: self.cfg.provider.clone(),
                base_url: self.cfg.base_url.clone(),
                api_key: (!self.api_key_draft.trim().is_empty())
                    .then(|| self.api_key_draft.clone()),
                model: self.cfg.model.clone(),
                thinking: "off".to_string(),
                temperature: self.cfg.temperature,
                messages: vec![],
            }));
            ctx.request_repaint();
        }
        if back {
            self.wizard_step -= 1;
        }
        if next {
            self.wizard_step += 1;
        }
        if finish {
            self.finish_wizard(&ctx, true);
        }
        if skip {
            self.finish_wizard(&ctx, false);
        }
        resize_grip(ui);
    }

    /// Drain the remap capture: on a captured combo, swap the registered
    /// hotkey (reverting to the old one if registration fails) and persist.
    fn drain_hotkey_capture(&mut self) {
        let Some(rx) = self.hotkey_capture_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(spec)) => {
                hotkey::unregister(&self._hotkey_mgr, self.hotkey_spec);
                match hotkey::register(&self._hotkey_mgr, spec) {
                    Ok(()) => {
                        self.cfg.hotkey_modifiers = hotkey::modifiers_to_config(spec.modifiers);
                        self.cfg.hotkey_key = hotkey::key_label(spec.key);
                        self.hotkey_spec = spec;
                        self.hotkey_label = spec.to_string();
                        if let Err(e) = config::save(&self.cfg) {
                            crate::logging::error(&format!("config save failed: {e:#}"));
                        }
                        self.last_health = None;
                        crate::logging::info(&format!("hotkey remapped: {spec}"));
                        self.status = format!("Hotkey set to {spec}");
                    }
                    Err(e) => {
                        // Roll back to the old combo so the app keeps working.
                        let _ = hotkey::register(&self._hotkey_mgr, self.hotkey_spec);
                        self.status = format!("Could not register {spec}: {e}");
                        crate::logging::error(&format!("hotkey re-register failed: {e:#}"));
                    }
                }
            }
            Ok(Err(e)) => self.status = e,
            Err(std::sync::mpsc::TryRecvError::Empty) => self.hotkey_capture_rx = Some(rx),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
        }
    }

    fn reset_popup(&mut self) {
        self.instruction.clear();
        self.tone = Tone::GlowUp;
        self.phase = Phase::Editing;
        self.result.clear();
        self.history.clear();
        self.status.clear();
        self.result_view = ResultView::Sbs;
        self.sbs_scroll = SyncScroll::default();
    }

    /// Store the finished run in the configurable history (and on disk).
    fn record_history(&mut self, tone: &str, instruction: &str, source: &str, result: &str) {
        if self.cfg.history_size == 0 {
            if !self.run_history.is_empty() {
                self.run_history.clear();
                let _ = history::save(&self.run_history);
            }
            return;
        }
        let entry_ts = now_ts();
        let entry = HistoryEntry {
            ts: entry_ts,
            tone: tone.to_string(),
            instruction: instruction.to_string(),
            source: source.to_string(),
            result: result.to_string(),
        };
        history::push(&mut self.run_history, entry, self.cfg.history_size);
        if let Err(e) = history::save(&self.run_history) {
            let msg = format!("failed to save history: {e:#}");
            eprintln!("textglow: {msg}");
            crate::logging::error(&msg);
        }
    }

    /// Move the attached run to the background so it finishes into history
    /// instead of being cancelled when the popup moves on.
    fn detach_run(&mut self) {
        if let Some(run) = self.run.take() {
            // Cap detached runs: spamming the hotkey must not pile up live
            // HTTP streams; the oldest one is dropped (cancelled).
            while self.background.len() >= 4 {
                self.background.remove(0);
            }
            self.background.push(run);
        }
    }

    fn start_stream(&mut self, ctx: &Context, refine: bool) {
        let instruction = self.instruction.trim().to_string();
        if refine && !self.history.is_empty() {
            if instruction.is_empty() {
                self.status = "Type a refinement instruction first.".into();
                return;
            }
            let msg = prompt::refine_message(&self.instruction, self.cfg.de_slop);
            self.history.push(msg);
        } else {
            if self.captured.trim().is_empty() {
                self.status = "Nothing to rewrite. Enter some text first.".into();
                return;
            }
            let sys = prompt::system_prompt(&self.cfg.system_prompt).to_string();
            self.run_source = self.captured.clone();
            self.run_tone = self.tone.label().to_string();
            self.history = prompt::build_first_messages(
                &sys,
                &self.captured,
                self.tone,
                Some(&self.instruction),
                self.cfg.de_slop,
            );
        }
        self.instruction.clear();
        self.spawn_from_history(ctx, instruction);
    }

    fn spawn_from_history(&mut self, ctx: &Context, instruction: String) {
        if self.history.is_empty() {
            return;
        }
        if self.cfg.base_url.trim().is_empty() || self.cfg.model.trim().is_empty() {
            self.phase = Phase::Failed(
                "No provider/model configured. Open Settings in the popup header or tray menu."
                    .into(),
            );
            return;
        }
        self.result.clear();
        self.status.clear();
        self.phase = Phase::Streaming;
        let thinking = if self.cfg.fast_no_thinking {
            "off".to_string()
        } else {
            self.cfg.thinking.clone()
        };
        let req = StreamRequest {
            provider: self.cfg.provider.clone(),
            base_url: self.cfg.base_url.clone(),
            api_key: (!self.api_key_draft.trim().is_empty()).then(|| self.api_key_draft.clone()),
            model: self.cfg.model.clone(),
            thinking,
            temperature: self.cfg.temperature,
            messages: self.history.clone(),
        };
        // Refine rounds keep the original tone; retries keep the previous
        // metadata. Fresh runs start with what the popup shows.
        let (tone, instruction) = match self.run.take() {
            Some(old) => (
                old.tone,
                if instruction.is_empty() {
                    old.instruction
                } else {
                    instruction
                },
            ),
            None => (
                if self.run_tone.is_empty() {
                    "Glow up".to_string()
                } else {
                    self.run_tone.clone()
                },
                instruction,
            ),
        };
        crate::logging::info(&format!(
            "stream start: {} / {}",
            self.cfg.provider, self.cfg.model
        ));
        self.run = Some(ActiveRun {
            rx: llm::spawn_stream(req),
            source: self.run_source.clone(),
            tone,
            instruction,
            acc: String::new(),
        });
        ctx.request_repaint();
    }

    fn replace(&mut self, ctx: &Context) {
        if self.result.trim().is_empty() {
            return;
        }
        let Some(captor) = &mut self.captor else {
            self.status = "Paste unavailable.".into();
            return;
        };
        let result = std::mem::take(&mut self.result);
        // Hide first so focus returns to the source app, then paste over the
        // (still selected) original text.
        self.screen = Screen::Hidden;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        std::thread::sleep(Duration::from_millis(90));
        if let Err(e) = captor.set_text(&result) {
            let msg = format!("clipboard write failed: {e:#}");
            eprintln!("textglow: {msg}");
            crate::logging::error(&msg);
        }
        crate::logging::info("replace: pasted rewrite");
        captor.paste();
        if let Some(snap) = self.clipboard_backup.take() {
            crate::capture::restore_snapshot_in_background(snap);
        }
        self.reset_popup();
    }

    fn copy_result(&mut self) {
        if let Some(c) = &mut self.captor {
            if c.set_text(&self.result).is_ok() {
                self.status = "Copied to clipboard".into();
            }
        }
    }

    fn back_to_edit(&mut self) {
        self.phase = Phase::Editing;
        self.result.clear();
        self.history.clear();
        self.status.clear();
    }

    fn cancel_stream(&mut self) {
        self.run = None;
        self.back_to_edit();
    }

    fn set_autostart(&mut self, on: bool) {
        let result = crate::startup::set(on);
        self.autostart_enabled = crate::startup::is_enabled();
        if let Some(t) = &self.tray {
            let _ = t.autostart_item.set_checked(self.autostart_enabled);
        }
        match result {
            Ok(()) => {
                self.settings_status = if self.autostart_enabled {
                    "Starts with Windows: on".into()
                } else {
                    "Starts with Windows: off".into()
                };
            }
            Err(e) => self.settings_status = format!("autostart failed: {e:#}"),
        }
    }

    // ---- settings ----------------------------------------------------------------

    fn open_settings(&mut self, ctx: &Context) {
        if self.screen != Screen::Settings {
            self.api_key_draft = config::load_api_key().unwrap_or_default();
            self.api_key_saved = !self.api_key_draft.is_empty();
            self.autostart_enabled = crate::startup::is_enabled();
            self.settings_status.clear();
        }
        self.screen = Screen::Settings;
        self.resize_and_show(ctx, SETTINGS_SIZE);
    }

    // ---- history -------------------------------------------------------------------

    /// Return to the popup/input view without touching any running stream.
    fn back_to_input(&mut self, ctx: &Context) {
        self.screen = Screen::Popup;
        self.resize_and_show(ctx, POPUP_SIZE);
    }

    /// Dev helper (`--demo-sbs` / `--demo-diff`): show the popup with a
    /// finished run so the result views can be inspected without an API call.
    fn open_done_demo(&mut self, ctx: &Context, view: ResultView) {
        self.captured = DEMO_TEXT.into();
        self.run_source = DEMO_TEXT.into();
        self.run_tone = "Glow up".into();
        self.reset_popup();
        self.result = DEMO_RESULT.into();
        self.phase = Phase::Done;
        self.result_view = view;
        self.screen = Screen::Popup;
        self.resize_and_show(ctx, POPUP_SIZE);
    }

    fn open_history(&mut self, ctx: &Context) {
        self.screen = Screen::History;
        self.resize_and_show(ctx, HISTORY_SIZE);
    }

    fn history_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            // Esc just returns to the input view; only the × closes.
            self.back_to_input(&ctx);
            return;
        }

        let mut close_window = false;
        let mut back_to_input = false;
        let mut clear_all = false;
        let mut delete_idx: Option<usize> = None;
        let mut load_idx: Option<usize> = None;
        let mut copy_idx: Option<usize> = None;

        let frame = panel_frame(ui, 22);
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                let heading = ui.heading("History");
                let mut cluster_left = f32::INFINITY;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let btn = ui.button("×");
                    if btn.clicked() {
                        close_window = true;
                    }
                    let info = ui.weak(
                        RichText::new(format!(
                            "{} run{} · Settings > History to change the limit",
                            self.run_history.len(),
                            if self.run_history.len() == 1 { "" } else { "s" }
                        ))
                        .small(),
                    );
                    cluster_left = info.rect.left();
                });
                header_drag_gap(
                    ui,
                    "tg-drag-history",
                    heading.rect.left(),
                    cluster_left - 4.0,
                    heading.rect.top(),
                    heading.rect.bottom(),
                );
            });
            ui.add_space(8.0);

            if self.run_history.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.weak("No runs yet. Every finished glow-up lands here.");
                });
            }
            egui::ScrollArea::vertical()
                .id_salt("tg-history")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let now = now_ts();
                    for (idx, e) in self.run_history.iter().enumerate() {
                        let open = self.history_expanded.contains(&idx);
                        let snippet: String = {
                            let s: String = e.source.chars().take(60).collect();
                            if e.source.chars().count() > 60 {
                                format!("{s}…")
                            } else {
                                s
                            }
                        };
                        let title = if e.instruction.is_empty() {
                            format!(
                                "{} · {} · {}",
                                history::format_relative(e.ts, now),
                                e.tone,
                                snippet
                            )
                        } else {
                            format!(
                                "{} · {} · \"{}\" · {}",
                                history::format_relative(e.ts, now),
                                e.tone,
                                e.instruction,
                                snippet
                            )
                        };
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(
                                    open,
                                    RichText::new(format!(
                                        "{} {}",
                                        if open { "▼" } else { "▶" },
                                        title
                                    ))
                                    .small(),
                                )
                                .clicked()
                            {
                                if open {
                                    self.history_expanded.remove(&idx);
                                } else {
                                    self.history_expanded.insert(idx);
                                }
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("Delete").clicked() {
                                        delete_idx = Some(idx);
                                    }
                                },
                            );
                        });
                        ui.add_space(2.0);
                        if open {
                            ui.columns(2, |cols| {
                                cols[0].weak(RichText::new("Source").small());
                                text_pane(
                                    &mut cols[0],
                                    150.0,
                                    &format!("hist-src-{idx}"),
                                    egui::Label::new(&e.source).selectable(true),
                                    None,
                                );
                                cols[1].weak(RichText::new("Result").small());
                                text_pane(
                                    &mut cols[1],
                                    150.0,
                                    &format!("hist-res-{idx}"),
                                    egui::Label::new(&e.result).selectable(true),
                                    None,
                                );
                            });
                            ui.horizontal(|ui| {
                                if ui.button("Load into editor").clicked() {
                                    load_idx = Some(idx);
                                }
                                if ui.button("Copy result").clicked() {
                                    copy_idx = Some(idx);
                                }
                            });
                            ui.add_space(6.0);
                        }
                    }
                });

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Clear all").clicked() {
                        clear_all = true;
                    }
                    if ui.button("Back").clicked() {
                        back_to_input = true;
                    }
                });
            });
        });

        if let Some(i) = load_idx {
            if let Some(e) = self.run_history.get(i) {
                self.captured = e.source.clone();
            }
            self.detach_run();
            self.reset_popup();
            self.screen = Screen::Popup;
            self.resize_and_show(&ctx, POPUP_SIZE);
        }
        if let Some(i) = copy_idx {
            if let Some(e) = self.run_history.get(i) {
                if let Some(c) = &mut self.captor {
                    let _ = c.set_text(&e.result);
                }
            }
        }
        if let Some(i) = delete_idx {
            if i < self.run_history.len() {
                self.run_history.remove(i);
                self.history_expanded.clear();
                let _ = history::save(&self.run_history);
            }
        }
        if clear_all {
            self.run_history.clear();
            self.history_expanded.clear();
            let _ = history::save(&self.run_history);
        }
        if back_to_input {
            self.back_to_input(&ctx);
        }
        if close_window {
            self.hide_window(&ctx);
        }
        resize_grip(ui);
    }
    // ---- popup ui ------------------------------------------------------------------

    fn popup_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            // Esc only hides; a running stream keeps going (and lands in
            // history); only closing or quitting cuts it.
            self.hide_window(&ctx);
            return;
        }

        let mut close = false;
        let mut open_settings = false;
        let mut open_history = false;
        let mut run = false;
        let mut replace = false;
        let mut copy = false;
        let mut back = false;
        let mut retry = false;
        let mut cancel = false;

        let frame = panel_frame(ui, 18);
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                let heading = ui.heading("TextGlow");
                // right_to_left layouts consume the whole remaining width, so
                // the drag gap must end at the leftmost widget in the cluster.
                let mut cluster_left = f32::INFINITY;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("×").clicked() {
                        close = true;
                    }
                    if ui.button("Settings").clicked() {
                        open_settings = true;
                    }
                    if ui.button("History").clicked() {
                        open_history = true;
                    }
                    let label = ui.weak(RichText::new(&self.hotkey_label).small());
                    cluster_left = label.rect.left();
                });
                header_drag_gap(
                    ui,
                    "tg-drag-popup",
                    heading.rect.left(),
                    cluster_left - 4.0,
                    heading.rect.top(),
                    heading.rect.bottom(),
                );
            });

            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                for t in Tone::ALL {
                    let selected = self.tone == t;
                    let label = ui.selectable_label(selected, t.label());
                    if label.clicked() {
                        self.tone = t;
                    }
                    if !selected {
                        // selectable_label draws no border in the resting
                        // state; stroke it directly so it reads as a button.
                        ui.painter().rect_stroke(
                            label.rect,
                            4.0,
                            egui::Stroke::new(1.0, egui::Color32::from_gray(95)),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            });
            ui.checkbox(&mut self.cfg.de_slop, "De-slop").on_hover_text(
                "Extra pass in every prompt that strips AI tells: em-dash overuse, \
                stock words like \u{201c}delve\u{201d} or \u{201c}moreover\u{201d}, \
                formulaic phrases, uniform sentence rhythm\u{2026}",
            );

            ui.add_space(4.0);
            let hint = if matches!(self.phase, Phase::Editing) {
                "Optional instructions (Enter to glow up)"
            } else {
                "Type a follow-up to refine (Enter to apply)"
            };
            let inst_frame = egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(8, 6))
                .fill(ui.style().visuals.extreme_bg_color)
                .stroke(ui.style().visuals.widgets.inactive.bg_stroke);
            let inst = egui::TextEdit::singleline(&mut self.instruction)
                .id(egui::Id::new("tg-instruction"))
                .hint_text(hint)
                .frame(inst_frame);
            let inst_resp = ui.add_sized([ui.available_width(), 0.0], inst);
            if self.focus_instruction {
                inst_resp.request_focus();
                self.focus_instruction = false;
            }
            self.instruction_focused = inst_resp.has_focus();
            if inst_resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                run = true;
            }

            ui.add_space(2.0);
            ui.separator();

            let footer_reserve = 46.0;
            match &self.phase {
                Phase::Editing => {
                    ui.horizontal(|ui| {
                        ui.weak(RichText::new("Text").small());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak(
                                RichText::new(format!("{} chars", self.captured.chars().count()))
                                    .small(),
                            );
                        });
                    });
                    let h = (ui.available_height() - footer_reserve - 2.0).max(60.0);
                    let src = egui::TextEdit::multiline(&mut self.captured)
                        .desired_rows(4)
                        .hint_text(
                            "Select text anywhere and press the hotkey, or paste/type here…",
                        );
                    let src_resp = ui.add_sized([ui.available_width(), h], src);
                    self.source_focused = src_resp.has_focus();
                }
                Phase::Streaming => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("streaming…");
                    });
                    result_area(ui, &self.result, footer_reserve + 24.0);
                }
                Phase::Done => {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.result_view, ResultView::Sbs, "Side by side");
                        ui.radio_value(&mut self.result_view, ResultView::Diff, "Diff");
                        ui.radio_value(&mut self.result_view, ResultView::Result, "Result only");
                    });
                    let reserve = footer_reserve + 28.0;
                    match self.result_view {
                        ResultView::Result => result_area(ui, &self.result, reserve),
                        ResultView::Sbs => sbs_area(
                            ui,
                            &self.run_source,
                            &self.result,
                            reserve,
                            &mut self.sbs_scroll,
                        ),
                        ResultView::Diff => {
                            diff_area(
                                ui,
                                &self.run_source,
                                &self.result,
                                reserve,
                                &mut self.diff_cache,
                            )
                        }
                    }
                }
                Phase::Failed(err) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 120, 120),
                        format!("Failed: {err}"),
                    );
                }
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                match &self.phase {
                    Phase::Editing => {
                        if ui.button("Glow up (Enter)").clicked() {
                            run = true;
                        }
                    }
                    Phase::Streaming => {
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    }
                    Phase::Done => {
                        if ui.button("Replace (Enter)").clicked() {
                            replace = true;
                        }
                        if ui.button("Copy").clicked() {
                            copy = true;
                        }
                        if ui.button("Back").clicked() {
                            back = true;
                        }
                    }
                    Phase::Failed(_) => {
                        if ui.button("Retry (Enter)").clicked() {
                            retry = true;
                        }
                        if ui.button("Back").clicked() {
                            back = true;
                        }
                    }
                }
                if ui
                    .checkbox(&mut self.cfg.fast_no_thinking, "Fast (no thinking)")
                    .on_hover_text(
                        "Skip model reasoning for this request, whatever tone is selected",
                    )
                    .changed()
                {
                    // Persist the preference right away.
                    if let Err(e) = config::save(&self.cfg) {
                        crate::logging::error(&format!("config save failed: {e:#}"));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    resize_grip(ui);
                    ui.weak(RichText::new(&self.status).small());
                });
            });
        });
        resize_grip(ui);

        if open_settings {
            self.open_settings(&ctx);
            return;
        }
        if open_history {
            self.open_history(&ctx);
            return;
        }
        if close {
            // Closing the popup window is the one thing that cancels a run.
            self.run = None;
            self.hide_window(&ctx);
            return;
        }
        if run {
            self.start_stream(&ctx, !self.history.is_empty());
            return;
        }
        if replace {
            self.replace(&ctx);
            return;
        }
        if copy {
            self.copy_result();
        }
        if back {
            self.back_to_edit();
        }
        if retry {
            self.spawn_from_history(&ctx, String::new());
        }
        if cancel {
            self.cancel_stream();
        }

        // Enter outside the text fields drives the primary action.
        if !self.instruction_focused
            && !self.source_focused
            && ctx.input(|i| i.key_pressed(Key::Enter))
        {
            match self.phase {
                Phase::Editing => self.start_stream(&ctx, false),
                Phase::Done => self.replace(&ctx),
                Phase::Failed(_) => self.spawn_from_history(&ctx, String::new()),
                Phase::Streaming => {}
            }
        }
    }

    // ---- settings ui ----------------------------------------------------------------

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            // Esc just returns to the input view; only the × closes.
            self.back_to_input(&ctx);
            return;
        }

        let mut close_window = false;
        let mut back = false;
        let mut save = false;
        let mut fetch = false;
        let mut clear_key = false;
        let mut autostart_change: Option<bool> = None;

        let frame = panel_frame(ui, 22);
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                let heading = ui.heading("TextGlow settings");
                let mut cluster_left = f32::INFINITY;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let btn = ui.button("×");
                    if btn.clicked() {
                        close_window = true;
                    }
                    cluster_left = btn.rect.left();
                });
                header_drag_gap(
                    ui,
                    "tg-drag-settings",
                    heading.rect.left(),
                    cluster_left - 4.0,
                    heading.rect.top(),
                    heading.rect.bottom(),
                );
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.strong("State");
                let color = match self.health() {
                    Health::Ready => egui::Color32::from_rgb(140, 235, 160),
                    Health::Unconfigured => egui::Color32::from_rgb(255, 210, 120),
                    Health::HotkeyFailed => egui::Color32::from_rgb(255, 130, 130),
                };
                ui.label(RichText::new(self.health_text()).color(color));
            });
            ui.add_space(4.0);

            egui::Grid::new("tg-settings")
                .num_columns(2)
                .spacing([18.0, 14.0])
                .show(ui, |ui| {
                    ui.strong("Provider");
                    egui::ComboBox::from_id_salt("tg-provider")
                        .selected_text(
                            crate::providers::find(&self.cfg.provider)
                                .map(|p| p.label)
                                .unwrap_or("Custom"),
                        )
                        .show_ui(ui, |ui| {
                            for p in crate::providers::PRESETS {
                                if ui
                                    .selectable_label(self.cfg.provider == p.id, p.label)
                                    .clicked()
                                {
                                    self.cfg.provider = p.id.into();
                                    self.cfg.base_url = p.base_url.into();
                                    self.cfg.model = p.default_model.into();
                                    self.models.clear();
                                    self.models_status.clear();
                                }
                            }
                        });
                    ui.end_row();

                    ui.strong("Base URL");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.cfg.base_url)
                            .desired_width(340.0)
                            .hint_text("https://…/v1"),
                    );
                    ui.end_row();

                    ui.strong("API key");
                    ui.horizontal(|ui| {
                        let needs_key = crate::providers::find(&self.cfg.provider)
                            .map(|p| p.needs_key)
                            .unwrap_or(false);
                        let hint = if self.api_key_saved && self.api_key_draft.is_empty() {
                            "saved, type to replace"
                        } else if needs_key {
                            "API key (stored in Windows Credential Manager)"
                        } else {
                            "API key (usually not needed for local servers)"
                        };
                        ui.add(
                            egui::TextEdit::singleline(&mut self.api_key_draft)
                                .password(true)
                                .desired_width(250.0)
                                .hint_text(hint),
                        );
                        if ui.button("Clear saved").clicked() {
                            clear_key = true;
                        }
                        if self.api_key_saved {
                            ui.weak(RichText::new("saved").small());
                        }
                    });
                    ui.end_row();

                    ui.strong("Model");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.cfg.model)
                                .desired_width(250.0)
                                .hint_text("model id, e.g. gpt-4o-mini"),
                        );
                        if ui.button("Fetch models").clicked() {
                            fetch = true;
                        }
                    });
                    ui.end_row();

                    ui.strong("Thinking");
                    egui::ComboBox::from_id_salt("tg-thinking")
                        .selected_text(thinking_label(&self.cfg.thinking))
                        .show_ui(ui, |ui| {
                            for (value, label) in THINKING_OPTIONS {
                                if ui
                                    .selectable_label(self.cfg.thinking == *value, *label)
                                    .clicked()
                                {
                                    self.cfg.thinking = (*value).to_string();
                                }
                            }
                        });
                    ui.end_row();

                    ui.strong("Temperature");
                    ui.add(
                        egui::DragValue::new(&mut self.cfg.temperature)
                            .range(0.0..=2.0)
                            .speed(0.05),
                    );
                    ui.end_row();

                    ui.strong("System prompt");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.cfg.system_prompt)
                            .desired_rows(4)
                            .desired_width(340.0)
                            .hint_text("Empty = built-in glow-up prompt"),
                    );
                    ui.end_row();

                    ui.strong("Hotkey");
                    ui.horizontal(|ui| {
                        ui.label(&self.hotkey_label);
                        if self.hotkey_capture_rx.is_some() {
                            ui.weak(RichText::new("press any key combo...").small());
                        } else if ui.button("Change").clicked() {
                            let (tx, rx) = std::sync::mpsc::channel();
                            hotkey::spawn_capture(
                                std::time::Duration::from_secs(10),
                                tx,
                            );
                            self.hotkey_capture_rx = Some(rx);
                            self.status = "Press the new key combo".into();
                        }
                    });
                    ui.end_row();

                    ui.strong("Startup");
                    let mut auto = self.autostart_enabled;
                    if ui.checkbox(&mut auto, "Start with Windows").changed() {
                        autostart_change = Some(auto);
                    }
                    ui.end_row();

                    ui.strong("History");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.cfg.history_size)
                                .range(0..=200)
                                .suffix(" runs"),
                        );
                        ui.weak(RichText::new("source + result kept (0 = off)").small());
                    });
                    ui.end_row();

                    ui.strong("Font size");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.cfg.font_size)
                                .range(11.0..=20.0)
                                .speed(0.5)
                                .suffix(" pt"),
                        );
                        ui.weak(RichText::new("applied immediately").small());
                    });
                    ui.end_row();
                });

            if !self.models.is_empty() {
                ui.add_space(4.0);
                ui.weak(RichText::new("Available models (click to use)").small());
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.models_filter)
                            .desired_width(280.0)
                            .hint_text("filter by name..."),
                    );
                    ui.weak(
                        RichText::new(format!(
                            "{} of {} shown",
                            filter_models(&self.models, &self.models_filter).len(),
                            self.models.len()
                        ))
                        .small(),
                    );
                });
                let filtered = filter_models(&self.models, &self.models_filter);
                egui::ScrollArea::vertical()
                    .id_salt("tg-models")
                    .max_height(150.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            // Cap rendering: huge catalogs (300+) get sluggish.
                            for m in filtered.iter().take(100) {
                                if ui
                                    .selectable_label(self.cfg.model == *m, m)
                                    .on_hover_text("click to select")
                                    .clicked()
                                {
                                    self.cfg.model = m.clone();
                                }
                            }
                        });
                    });
            }
            if !self.models_status.is_empty() {
                ui.weak(RichText::new(&self.models_status).small());
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                    if ui.button("Back").clicked() {
                        back = true;
                    }
                    ui.weak(RichText::new(&self.settings_status).small());
                });
            });
        });

        if clear_key {
            let _ = config::store_api_key("");
            self.api_key_draft.clear();
            self.api_key_saved = false;
            self.settings_status = "Saved API key removed".into();
        }
        if fetch {
            self.models_status = "fetching…".into();
            self.models_rx = Some(llm::spawn_fetch_models(ModelsRequest {
                base_url: self.cfg.base_url.clone(),
                api_key: (!self.api_key_draft.trim().is_empty())
                    .then(|| self.api_key_draft.clone()),
            }));
            ctx.request_repaint();
        }
        if let Some(on) = autostart_change {
            self.set_autostart(on);
        }
        if save {
            match config::save(&self.cfg)
                .and_then(|_| config::store_api_key(self.api_key_draft.trim()))
            {
                Ok(()) => {
                    self.api_key_saved = !self.api_key_draft.trim().is_empty();
                    self.settings_status = "Saved".into();
                }
                Err(e) => self.settings_status = format!("save failed: {e:#}"),
            }
        }
        if back {
            self.back_to_input(&ctx);
        }
        if close_window {
            // The only way to dismiss the window from settings.
            self.hide_window(&ctx);
        }
        resize_grip(ui);
    }
}

#[cfg(windows)]
fn cursor_monitor_work_area() -> Option<(f32, f32, f32, f32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        if GetCursorPos(&mut pt) == 0 {
            return None;
        }
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..std::mem::zeroed()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let mut dpi_x: u32 = 0;
        let mut dpi_y: u32 = 0;
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = if dpi_x == 0 { 1.0 } else { dpi_x as f32 / 96.0 };
        let x = info.rcWork.left as f32 / scale;
        let y = info.rcWork.top as f32 / scale;
        let w = (info.rcWork.right - info.rcWork.left) as f32 / scale;
        let h = (info.rcWork.bottom - info.rcWork.top) as f32 / scale;
        Some((x, y, w, h))
    }
}

/// Case-insensitive substring filter for the models list.
fn filter_models(models: &[String], query: &str) -> Vec<String> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return models.to_vec();
    }
    models
        .iter()
        .filter(|m| m.to_ascii_lowercase().contains(&q))
        .cloned()
        .collect()
}

fn panel_frame(ui: &egui::Ui, margin: i8) -> egui::Frame {
    let visuals = &ui.style().visuals;
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(margin))
        .fill(visuals.panel_fill)
        .stroke(egui::Stroke::new(
            1.0,
            visuals.widgets.inactive.bg_stroke.color,
        ))
}

/// Apply the configured font size plus the overall spacing/padding.
fn apply_style(ctx: &Context, size: f32) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let mut style = (*ctx.style_of(theme)).clone();
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(size));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(size));
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional((size - 3.0).max(9.0)),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::proportional(size + 6.0),
        );
        // Breathing room: chunkier buttons and more space between elements.
        style.spacing.button_padding = egui::vec2(14.0, 7.0);
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        ctx.set_style_of(theme, style);
    }
}

fn result_area(ui: &mut egui::Ui, result: &str, footer_reserve: f32) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .max_height((ui.available_height() - footer_reserve).max(60.0))
        .show(ui, |ui| {
            ui.add(egui::Label::new(result).selectable(true));
        });
}

/// Original left, rewrite right: equal framed heights, scrollable, with
/// synchronized scrolling so the two panes can be skimmed together.
fn sbs_area(
    ui: &mut egui::Ui,
    original: &str,
    result: &str,
    reserve: f32,
    scroll: &mut SyncScroll,
) {
    let h = (ui.available_height() - reserve).max(110.0);
    // Only the pane that FOLLOWS gets a forced offset this frame, so the pane
    // the user is dragging always behaves naturally; the other catches up.
    let force_l = scroll.left_force();
    let force_r = scroll.right_force();
    let mut l_now = scroll.sync;
    let mut r_now = scroll.sync;
    ui.columns(2, |cols| {
        cols[0].weak(egui::RichText::new("Original").small());
        l_now = text_pane(
            &mut cols[0],
            h,
            "tg-sbs-l",
            egui::Label::new(original).selectable(true),
            force_l,
        );
        cols[1].weak(egui::RichText::new("Glowed up").small());
        r_now = text_pane(
            &mut cols[1],
            h,
            "tg-sbs-r",
            egui::Label::new(result).selectable(true),
            force_r,
        );
    });
    scroll.update(l_now, r_now);
}

/// Shared-scroll bookkeeping for the side-by-side panes: whichever pane the
/// user moved becomes the master; the other is pushed to the same offset on
/// the next frame (and only then, so dragging always feels natural).
#[derive(Default)]
struct SyncScroll {
    last: Option<(f32, f32)>,
    /// Some(true) = right adopts the offset next frame; Some(false) = left.
    apply: Option<bool>,
    sync: f32,
}

impl SyncScroll {
    fn left_force(&self) -> Option<f32> {
        (self.apply == Some(false)).then_some(self.sync)
    }

    fn right_force(&self) -> Option<f32> {
        (self.apply == Some(true)).then_some(self.sync)
    }

    fn update(&mut self, l: f32, r: f32) {
        let was_applying = self.apply.take();
        if was_applying.is_none() {
            let (lp, rp) = self.last.unwrap_or((l, r));
            if (l - lp).abs() > 0.5 {
                self.sync = l;
                self.apply = Some(true);
            } else if (r - rp).abs() > 0.5 {
                self.sync = r;
                self.apply = Some(false);
            }
        }
        self.last = Some((l, r));
    }
}

/// A framed, fixed-height, scrollable read-only text pane. Every pane given
/// the same `h` ends up with exactly the same outer height. Returns the
/// scroll offset after this frame.
fn text_pane(
    ui: &mut egui::Ui,
    h: f32,
    id: &str,
    label: egui::Label,
    scroll_offset: Option<f32>,
) -> f32 {
    let stroke = ui.style().visuals.widgets.inactive.bg_stroke;
    let fill = ui.style().visuals.extreme_bg_color;
    let inner = egui::Frame::NONE
        .fill(fill)
        .stroke(stroke)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_min_size(egui::vec2(ui.available_width(), (h - 20.0).max(60.0)));
            let mut area = egui::ScrollArea::vertical()
                .id_salt(id)
                .auto_shrink([false, false]);
            if let Some(off) = scroll_offset {
                area = area.vertical_scroll_offset(off);
            }
            area.max_height((h - 40.0).max(40.0))
                .show(ui, |ui| ui.add(label))
        })
        .inner;
    inner.state.offset.y
}

fn diff_area(
    ui: &mut egui::Ui,
    original: &str,
    result: &str,
    reserve: f32,
    cache: &mut diff::DiffCache,
) {
    let h = (ui.available_height() - reserve).max(110.0);
    let key = diff::DiffCache::key_for(original, result);
    let Some(tokens) = cache.get_or_compute(key, || diff::word_diff(original, result)) else {
        ui.weak("Text too large for the diff view. Use Side by side.");
        return;
    };
    let Some(job) = diff_job(ui, tokens) else {
        return;
    };
    text_pane(
        ui,
        h,
        "tg-diff",
        egui::Label::new(job).selectable(true),
        None,
    );
}

/// Drag the window by the header gap between the title and the button
/// cluster. Keeping the drag surface completely disjoint from the buttons
/// avoids egui's click-vs-drag hit-test ambiguity entirely (buttons stay
/// fully clickable and hoverable). The native drag starts once the pointer
/// moves, so clicks land normally.
fn header_drag_gap(ui: &mut egui::Ui, id: &str, left: f32, right: f32, top: f32, bottom: f32) {
    if right <= left {
        return; // no gap (window too narrow)
    }
    let rect = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom));
    let drag = ui
        .interact(rect, egui::Id::new(id), egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab);
    if drag.dragged() && drag.drag_delta().length() > 1.0 {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// Drag handle pinned to the true bottom-right corner of the window (native
/// edge-resize is unreliable without decorations): a filled corner triangle
/// with the diagonal resize cursor. Clamped to a minimum size.
fn resize_grip(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    egui::Area::new(egui::Id::new("tg-resize-grip"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-2.0, -2.0))
        .order(egui::Order::Foreground)
        .show(&ctx, |ui| {
            let size = ui.input(|i| {
                i.viewport()
                    .inner_rect
                    .map(|r| r.size())
                    .unwrap_or(egui::vec2(880.0, 700.0))
            });
            let (resp, painter) = ui.allocate_painter(egui::vec2(22.0, 22.0), egui::Sense::drag());
            let resp = resp.on_hover_cursor(egui::CursorIcon::ResizeNorthWest);
            let visuals = ui.style().visuals.clone();
            let fill = if resp.dragged() {
                visuals.strong_text_color()
            } else if resp.hovered() {
                visuals.text_color()
            } else {
                visuals.weak_text_color()
            };
            // Classic corner triangle: hypotenuse facing up-left.
            let corner = resp.rect.right_bottom() + egui::vec2(-1.0, -1.0);
            let tri = vec![
                corner + egui::vec2(-15.0, 0.0),
                corner + egui::vec2(0.0, -15.0),
                corner,
            ];
            painter.add(egui::Shape::convex_polygon(
                tri,
                egui::Color32::from_black_alpha(120),
                egui::Stroke::new(2.0, fill),
            ));
            if resp.dragged() {
                let d = resp.drag_delta();
                let new = egui::vec2((size.x + d.x).max(640.0), (size.y + d.y).max(500.0));
                ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(new));
            }
        });
}

fn diff_job(ui: &egui::Ui, tokens: &[diff::DiffToken]) -> Option<egui::text::LayoutJob> {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let base_color = ui.style().visuals.text_color();
    let mut job = egui::text::LayoutJob::default();
    for token in tokens {
        let (text, color, bg) = match token {
            diff::DiffToken::Same(s) => (s, base_color, egui::Color32::TRANSPARENT),
            diff::DiffToken::Removed(s) => (
                s,
                egui::Color32::from_rgb(255, 130, 130),
                egui::Color32::from_rgb(90, 32, 32),
            ),
            diff::DiffToken::Added(s) => (
                s,
                egui::Color32::from_rgb(140, 240, 160),
                egui::Color32::from_rgb(26, 74, 40),
            ),
        };
        job.append(
            &text,
            0.0,
            egui::text::TextFormat {
                font_id: font.clone(),
                color,
                background: bg,
                ..Default::default()
            },
        );
    }
    job.wrap.max_width = ui.available_width();
    Some(job)
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl eframe::App for TextGlowApp {
    /// Non-UI logic; also runs while the window is hidden after repaint requests
    /// (hotkey/tray watchers trigger these).
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.frames += 1;

        // eframe unconditionally shows the root window right after the first
        // painted frame (to avoid a white flash); counteract it so the app
        // actually starts hidden in the tray.
        if self.screen == Screen::Hidden && self.frames <= 5 {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            ctx.request_repaint_after(Duration::from_millis(120));
        }

        if self.smoke {
            eprintln!("smoke: frame {}", self.frames);
            if self.frames >= 3 {
                eprintln!("smoke ok");
                std::process::exit(0);
            }
        }

        if self.wizard_needed && self.frames == 1 && self.wizard_original.is_none() {
            self.wizard_original = Some(self.cfg.clone());
            self.wizard_step = 1;
            self.screen = Screen::Wizard;
            self.resize_and_show(ctx, WIZARD_SIZE);
        }
        if self.demo && self.frames == 1 {
            self.captured = DEMO_TEXT.into();
            self.reset_popup();
            self.screen = Screen::Popup;
            self.resize_and_show(ctx, POPUP_SIZE);
            self.focus_instruction = true;
        }
        if self.demo_sbs && self.frames == 1 {
            self.open_done_demo(ctx, ResultView::Sbs);
        }
        if self.demo_diff && self.frames == 1 {
            self.open_done_demo(ctx, ResultView::Diff);
        }
        if self.demo_history && self.frames == 1 {
            let now = now_ts();
            self.run_history = vec![
                HistoryEntry {
                    ts: now.saturating_sub(300),
                    tone: "Professional".into(),
                    instruction: "more formal".into(),
                    source: DEMO_TEXT.into(),
                    result: DEMO_RESULT.into(),
                },
                HistoryEntry {
                    ts: now.saturating_sub(2 * 86400),
                    tone: "Glow up".into(),
                    instruction: String::new(),
                    source: "quick note about the meeting tomorrow".into(),
                    result: "A quick note about tomorrow's meeting.".into(),
                },
            ];
            self.open_history(ctx);
        }
        if self.demo_settings && self.frames == 1 {
            self.open_settings(ctx);
        }

        self.drain_ui_events(ctx);
        self.drain_llm();
        self.drain_models();
        self.drain_wizard_test();
        self.drain_hotkey_capture();
        self.push_health_to_tray();

        if self.run.is_some() || !self.background.is_empty() || self.models_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(32));
        }

        // Live-apply font size changes from Settings.
        if (self.cfg.font_size - self.applied_font_size).abs() > 0.01 {
            apply_style(ctx, self.cfg.font_size);
            self.applied_font_size = self.cfg.font_size;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        match self.screen {
            Screen::Hidden => {
                egui::CentralPanel::default().show(ui, |_| {});
            }
            Screen::Popup => self.popup_ui(ui),
            Screen::Settings => self.settings_ui(ui),
            Screen::History => self.history_ui(ui),
            Screen::Wizard => self.wizard_ui(ui),
        }
    }
}

#[cfg(test)]
mod models_filter_tests {
    use super::*;

    #[test]
    fn filter_is_case_insensitive_and_empty_query_keeps_all() {
        let models = vec![
            "openai/gpt-4o-mini".to_string(),
            "anthropic/claude-sonnet-4".to_string(),
            "meta-llama/llama-3.3-70b".to_string(),
        ];
        assert_eq!(filter_models(&models, "").len(), 3);
        assert_eq!(filter_models(&models, "   ").len(), 3);
        let out = filter_models(&models, "GPT-4O");
        assert_eq!(out, vec!["openai/gpt-4o-mini".to_string()]);
        assert!(filter_models(&models, "zzz").is_empty());
    }
}
