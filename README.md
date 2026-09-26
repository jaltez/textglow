# TextGlow

A tray-resident Windows app (cross-platform-ready) for AI-powered text rewriting.
Select text anywhere, press **Win+F8**, and a popup appears near your cursor with
the text loaded. Press **Enter** for an instant "glow up", or type a short
instruction first ("more professional, add X, drop Y"). The result streams in,
then you can **Replace** it back over your original selection (Gmail-style),
**Copy** it, or keep refining.

## How it works

1. Select text in any app → press `Win+F8`. TextGlow waits until you've fully
   released the hotkey, simulates Ctrl+C to grab the selection (your clipboard
   is restored right after), and opens the popup near your cursor. A fresh
   selection always wins; if nothing was selected it falls back to your
   clipboard text and says so in the popup status line.
2. The popup opens with the text. Optionally pick a tone chip
   (*Glow up / Professional / Casual / Shorter / Longer / Fix grammar*) and/or
   type free-form instructions.
3. Press `Enter` → the rewrite streams into the popup (centered on screen).
4. Compare the result your way. The default is **Side by side** (original left
   half, rewrite right half); other options are **Diff** (word-level inline
   diff, removals red / additions green) and **Result only**.
5. **Enter** replaces the original selection (Ctrl+V into the source app, then
   your clipboard is restored). **Copy** just copies. Or type a follow-up
   instruction and press `Enter` to refine the result again.
6. `Esc` dismisses without cancelling: a running stream keeps going (you'll
   find it in History when it finishes). Only closing the popup or pressing
   **Cancel** stops a run. From Settings or History, `Esc`/`Back` returns to
   the input view. The window itself only closes via the **×** in its
   top-right corner.

## History

Every finished run (source + result, with tone and instruction) is kept in
`%APPDATA%\textglow\history.json` (newest first). Open it via the **History**
button in the popup or the tray menu: expand an entry to view, **Load into
editor**, **Copy result**, **Delete**, or **Clear all**. The number of runs
kept is configurable in **Settings → History** (default 25, `0` disables
history entirely). Each entry has a **Delete** button on its row; **Back**
(Esc) returns to the popup without interrupting anything that's still
streaming.

## Appearance

**Settings → Font size** (default 15 pt, range 11–20) scales the popup's text
and applies immediately. The window is resizable: drag the grip in the
bottom-right corner (or the window edges). Side-by-side panes share the same
height and scroll in sync.

You can also open the popup from the tray icon (left click); it loads the
current clipboard text.

## Providers

One OpenAI-compatible client covers everything. Presets (tray → **Settings**):

| Preset | Base URL | Key needed |
|---|---|---|
| OpenRouter | `https://openrouter.ai/api/v1` | yes |
| DeepSeek | `https://api.deepseek.com/v1` | yes |
| Z.ai (GLM) | `https://api.z.ai/api/paas/v4` | yes |
| OpenAI | `https://api.openai.com/v1` | yes |
| Groq | `https://api.groq.com/openai/v1` | yes |
| Anthropic (OpenAI-compat) | `https://api.anthropic.com/v1` | yes |
| Google Gemini (OpenAI-compat) | `https://generativelanguage.googleapis.com/v1beta/openai` | yes |
| Ollama (local) | `http://localhost:11434/v1` | no |
| LM Studio (local) | `http://localhost:1234/v1` | no |
| Custom | any OpenAI-compatible endpoint | maybe |

**Fetch models** pulls the live model list from the provider's `GET /models`
endpoint (works on OpenRouter, DeepSeek, OpenAI, Groq, Ollama, …). Manual model
ids are always accepted.

API keys are stored in **Windows Credential Manager**, never in the config file.

## Configuration

`%APPDATA%\textglow\config.toml` (created on first run):

```toml
provider = "openrouter"
base_url = "https://openrouter.ai/api/v1"
model = "openai/gpt-4o-mini"
thinking = "medium"           # off | low | medium | high (provider-mapped)
temperature = 0.7
history_size = 25             # past runs kept (source + result); 0 = off
system_prompt = ""            # empty = built-in glow-up prompt
hotkey_modifiers = "SUPER"    # SUPER | CONTROL | ALT | SHIFT, joined with |
hotkey_key = "F8"             # F1-F12, A-Z, 0-9, Space, Tab, Enter
```

Restart the app after editing the hotkey.

## Build & run

Requires the Rust toolchain (MSVC target on Windows).

```
cargo run            # dev run
cargo build --release   # → target/release/textglow.exe
cargo test           # unit tests
```

Dev helpers (all bypass the single-instance guard so they can run next to a
live instance):

- `textglow --smoke`: init everything, exit after 3 frames
- `textglow --demo`: popup with sample text
- `textglow --demo-sbs` / `--demo-diff`: finished-run result views
- `textglow --demo-settings` / `--demo-history`: those screens with sample data

**Start with Windows**: enable it in Settings or the tray menu (writes the
HKCU Run key).

## Manual test checklist

After building, verify across target apps:

- [ ] Browser textarea, VS Code, Notepad, Word: select text → `Win+F8` → text
      appears in popup → Enter → result replaces the selection.
- [ ] No selection → popup falls back to clipboard text.
- [ ] Empty clipboard → popup opens empty, typing/pasting works.
- [ ] `Esc` → popup closes; your original clipboard content is intact.
- [ ] Copy (not Replace) leaves the source untouched, result in clipboard.
- [ ] Refine: after a result, type a follow-up + Enter → improved result.
- [ ] Bad API key → clear red error message in the popup.
- [ ] Terminals: Ctrl+C is often "interrupt", not "copy", so the clipboard
      fallback covers it (known limitation).

## Known limitations (v1)

- Only text (+ images) clipboard formats are restored; e.g. copied *files* are
  not restored after capture/paste-back.
- The hotkey is set in the config file (no remap UI yet).
- Windows-only behaviors: taskbar hiding, single-instance mutex, Credential
  Manager. The crate stack is cross-platform; macOS/Linux builds are a
  follow-up (macOS needs accessibility permission for key simulation).

## Ideas for iteration 2

models.dev-powered searchable model picker (pricing/logos), hotkey remap UI,
custom prompt library, multiple result variants, richer clipboard format
backup, auto-update.
