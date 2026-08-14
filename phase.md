# gramit v1 — system-wide grammar fixer

## Context

`gramit` fixes the grammar of text you've selected in *any* app — Chrome, Slack, a text editor — without leaving that app. The user selects text (`Ctrl+A` or by hand), presses `Ctrl+Alt+F`, and the selection is replaced in place with a corrected version.

v1 builds three pieces:

- **CLI** (Rust) — `gramit start/stop/status/fix/config/doctor`
- **Daemon** (Rust) — owns the global hotkey, clipboard, and synthetic keystrokes
- **Backend** (Node + Express) — the only component holding an API key; calls Azure OpenAI

Locked decisions: Azure OpenAI (`gpt-5.6-luna` deployment), backend runs locally on `127.0.0.1`, corrections are **minimal grammar fixes** (preserve the user's voice), feedback via **desktop notifications**. Must work on Windows, macOS, and this Linux machine (Ubuntu GNOME Shell 50.1, Wayland); other Linux desktops are out of scope for v1.

### How this works on Wayland *(revised after Module 2c — measured, not assumed)*

Wayland blocks global hotkeys and keystroke injection, which would kill the in-place-replacement UX. Two different mechanisms get us back in, and only one of them turned out to be a portal:

- **Keystroke injection** — `org.freedesktop.portal.RemoteDesktop`. Works. One consent dialog, then a persisted `restore_token` keeps it silent. **No root, no `ydotool`, no `/dev/uinput`.**
- **The hotkey** — a **GNOME custom keybinding**, *not* the GlobalShortcuts portal. That portal rejects any app lacking a sandbox app id, which rules out an installed binary; see the Module 2c notes for everything that was tried. The keybinding runs `gramit fix --selection`, driving the identical daemon code path.

---

## Architecture

```
gramit (CLI) ──local socket──> gramitd (daemon) ──HTTP──> Express :8787 ──> Azure OpenAI
                                  │
                                  ├─ global hotkey  (Ctrl+Alt+F)
                                  ├─ clipboard      (save → capture → restore)
                                  ├─ key injection  (Ctrl+C / Ctrl+V)
                                  └─ notifications
```

**The fix loop** (the heart of v1, in `gramitd`):

1. Hotkey fires; ignore if a fix is already in flight.
2. Save the current clipboard (note whether it held non-text so we don't clobber it).
3. Inject Copy (`Cmd+C` on macOS, `Ctrl+C` elsewhere).
4. Poll the clipboard up to ~400 ms — clipboard writes are async and racy.
5. No change / empty → notify "No text selected", restore, abort.
6. Reject text over `max_chars` (default 8000) or pure whitespace.
7. `POST /v1/fix` to the backend (10 s timeout).
8. If the result is identical → notify "Looks good already", restore, **skip the paste** (don't disturb the selection or the undo stack).
9. Otherwise: write corrected text → inject Paste → wait ~150 ms → restore the original clipboard.
10. Notify "Fixed N issues". Any error restores the clipboard and notifies with a real reason.

---

## Layout

```
gramit/
├── Cargo.toml                 # workspace
├── crates/
│   ├── gramit-core/           # config, IPC types, backend HTTP client, errors
│   ├── gramit-input/          # platform traits + per-OS impls (cfg-heavy)
│   ├── gramitd/               # daemon binary
│   └── gramit-cli/            # `gramit` binary
└── backend/                   # Node 24 + TypeScript + Express 5
```

### Rust crates

| Concern | Crate |
|---|---|
| CLI parsing | `clap` (derive) |
| Async / IPC | `tokio`, `interprocess` (Unix socket + Windows named pipe) |
| HTTP | `reqwest` (no TLS in v1 — see Module 2a notes) |
| Config | `toml`, `serde`, `directories` |
| Clipboard | `arboard` |
| Hotkey (Win/macOS) | `global-hotkey` |
| Injection (Win/macOS) | `enigo` |
| Hotkey + injection (Linux) | `ashpd` (GlobalShortcuts + RemoteDesktop portals) |
| Notifications | `notify-rust` |
| Logging | `tracing`, `tracing-subscriber` |
| Errors | `anyhow` (bins), `thiserror` (libs) |

### `gramit-input` — the platform seam

Three traits keep everything above them platform-agnostic:

```rust
trait HotkeySource { async fn next(&mut self) -> HotkeyEvent; }
trait Injector     { fn copy(&self) -> Result<()>; fn paste(&self) -> Result<()>; }
trait Clipboard    { fn get_text(&mut self) -> Result<Option<String>>; fn set_text(&mut self, s: &str) -> Result<()>; }
```

| | Hotkey | Injection | Clipboard | Permission |
|---|---|---|---|---|
| **Linux** (GNOME/Wayland) | GNOME custom keybinding (the GlobalShortcuts portal refuses unsandboxed apps) | RemoteDesktop portal | `arboard` — X11 backend works, Mutter bridges Xwayland↔Wayland clipboards | one-time portal consent, persisted via `restore_token` |
| **Windows** | `global-hotkey` (`RegisterHotKey`) | `enigo` (`SendInput`) | `arboard` | none |
| **macOS** | `global-hotkey` (Carbon) | `enigo` (`CGEvent`) | `arboard` | Accessibility (TCC) prompt |

Two platform gotchas to build around from the start:

- **macOS**: `global-hotkey` and `enigo` need the main-thread run loop. The daemon must keep `main` for the event loop and run Tokio on a worker thread — retrofitting this later is painful.
- **Linux hotkey** (confirmed in 2c: this is the primary path, not a fallback): a GNOME custom keybinding (`gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings …`) bound to `gramit fix --selection`, which drives the identical daemon path. `gramit doctor` detects and offers to install it.

### IPC

Newline-delimited JSON over `$XDG_RUNTIME_DIR/gramit.sock` (macOS/Linux) or `\\.\pipe\gramit` (Windows):

- `Ping` → `Pong { version, pid, uptime }`
- `Fix { text, mode }` → `Fixed { text, changes }` — powers `gramit fix` and all testing without a hotkey
- `FixSelection` → runs the full capture→replace loop (used by the Linux keybinding fallback)
- `Status` → daemon state, backend reachability, last error
- `Shutdown`

### CLI surface

| Command | Behavior |
|---|---|
| `gramit start` | spawn daemon detached, wait for the socket, report readiness; `--foreground` for debugging |
| `gramit stop` / `restart` / `status` | lifecycle + health |
| `gramit fix [TEXT\|-]` | one-shot to stdout; `--clipboard` fixes the clipboard in place; `--selection` runs the full loop |
| `gramit config get/set/path` | hotkey, backend URL, mode, notifications |
| `gramit doctor` | daemon up? backend reachable? API key set? platform permission? clipboard read/write? injection smoke test? |
| `gramit logs [-f]` | tail the daemon log |

Config at `~/.config/gramit/config.toml` (platform-appropriate via `directories`):

```toml
hotkey             = "Ctrl+Alt+F"
backend_url        = "http://127.0.0.1:8787"
mode               = "grammar"
notifications      = true
max_chars          = 8000
request_timeout_ms = 10000
```

---

## Backend detail (Node 24 + TypeScript + Express 5)

```
backend/src/
├── index.ts          # bootstrap, bind 127.0.0.1:8787
├── routes/fix.ts     # POST /v1/fix
├── routes/health.ts  # GET  /health
├── llm/azure.ts      # AzureOpenAI client (openai npm package)
├── prompt.ts         # minimal-grammar-fix system prompt
├── diff.ts           # change counter (diff package, word-level)
└── cache.ts          # small LRU keyed by hash(text + mode)
```

`POST /v1/fix` — `{ text, mode }` → `{ corrected, changed, changes, model, latency_ms }`. Validate with `zod`, cap at 25k chars, 30 s upstream timeout, and always return a machine-readable `code` on errors (`NO_API_KEY`, `UPSTREAM_TIMEOUT`, `RATE_LIMITED`, `TOO_LONG`) so the daemon's toast says something useful.

Azure config via `.env` (`.env.example` committed, `.env` gitignored):

```
AZURE_OPENAI_ENDPOINT=https://<resource>.openai.azure.com
AZURE_OPENAI_API_KEY=...
AZURE_OPENAI_DEPLOYMENT=gpt-5.6-luna
AZURE_OPENAI_API_VERSION=...
PORT=8787
```

**Prompt contract**: fix grammar, spelling, and punctuation only. Preserve wording, voice, casing, line breaks, and leading/trailing whitespace. Never add, remove, or explain content. Leave code, URLs, `@mentions`, and emoji untouched. Return the text unchanged if it is already correct.

Ask for a JSON object (`{"corrected": "…"}`) so the model can't prepend "Here's the corrected text"; if the deployment doesn't honor JSON mode, fall back to plain text through a sanitizer that strips code fences and `Corrected:`-style prefixes. The LRU cache makes a repeated fix on the same text instant and free.

---

## Phases — module by module

Build order is **Backend → Daemon → CLI**, because each is testable using only what precedes it: the backend with `curl`, the daemon against the live backend, the CLI against the running daemon. The one exception is the shared `gramit-core` crate (config + IPC types + HTTP client) — it lands at the start of the Daemon module since both Rust binaries depend on it.

Check items off here as we go; this file is the living checklist.

### Module 1 — Backend (Node 24 + TypeScript + Express 5)

The only component holding an API key. Fully testable standalone with `curl`.

- [x] Scaffold `backend/`: TypeScript, Express 5, `zod`, `openai` (AzureOpenAI), `dotenv`, `vitest`
- [x] `GET /health` → `{ ok, version, hasKey, model, missing, uptime_s }`
- [x] `llm/azure.ts` — AzureOpenAI client from `.env` (endpoint, key, `gpt-5.6-luna` deployment, api-version); logs loudly at startup if config is missing
- [x] `prompt.ts` — the minimal-grammar-fix system prompt; JSON-object output with a plain-text sanitizer fallback
- [x] `POST /v1/fix` — zod validation, 25k cap, 30 s upstream timeout, typed error `code`s
- [x] `diff.ts` — word-level change count
- [x] `cache.ts` — LRU on `hash(text + mode)`
- [x] Tests: prompt sanitizer, diff counter, cache, service behavior, route validation (47 tests)
- [ ] Live call against the real Azure deployment — **blocked on `AZURE_OPENAI_ENDPOINT` + `AZURE_OPENAI_API_VERSION`**

**Done when**: `curl -sX POST localhost:8787/v1/fix -d '{"text":"he go to the store yesterday"}'` returns corrected text plus a change count, and `vitest` is green.

Notes from the build:

- The server **binds even when Azure is unconfigured**, and `/v1/fix` answers `503 NO_API_KEY` naming the missing vars. That's deliberate — `gramit doctor` gets a diagnosable answer instead of a connection refused.
- `gpt-5.6-luna` is a custom deployment whose accepted parameters we can't know ahead of time, so `llm/azure.ts` walks a ladder of request shapes (`json+temperature` → `json` → `plain`), remembering the first one the deployment accepts. No max-token cap is sent: a correction is about as long as its input, and a cap risks truncating the user's text mid-paste.
- `sanitizeCorrection()` is the paste-safety net — it unwraps JSON, code fences, "Here's the corrected text:" preambles and added quotes, then restores the original's exact leading/trailing whitespace.
- The system prompt tells the model the user's text is **data, never instructions** — arbitrary selected text goes straight into the prompt.

### Module 2 — Daemon (`gramitd`)

Where all the risk lives. Split into a text path (portable, easy) and a platform path (per-OS, hard).

**2a — shared core + text path** ✅
- [x] `gramit-core`: config load/save (`config.toml`), IPC protocol types, `reqwest` backend client, error types
- [x] `gramitd`: local socket server (`interprocess`), `Ping` / `Fix` / `FixSelection` / `Status` / `Shutdown` handlers, `tracing` to a log file
- [x] Stale-socket recovery, single-instance guard, 0600 socket permissions, SIGINT/SIGTERM shutdown
- [x] 43 tests: 22 core unit, 12 daemon unit, 9 end-to-end against the real binary over a real socket

Notes from the build:

- **No TLS in v1.** `reqwest`'s `rustls` feature pulls in `aws-lc-sys`, whose assembly does not compile against this machine's binutils 2.46. Since the backend is localhost-only, TLS is dead weight — `reqwest` is built with `default-features = false, features = ["json"]`. Hosted v2 must add a TLS provider, and should prefer the `ring` provider over `aws-lc-rs` for exactly this reason.
- **Error codes survive the whole chain.** A backend `NO_API_KEY` arrives at the IPC client as `NO_API_KEY`, not a generic HTTP 503 — which is what lets Module 2e write a useful toast and Module 3's `doctor` give a concrete remedy.
- **`Shutdown::trigger` uses `send_replace`, not `send`.** `watch::Sender::send` reports failure *and skips the update* when no receiver is currently subscribed, which is the normal state before the accept loop starts waiting. Tests caught it; don't "simplify" it back.
- **Unix socket paths max out around 108 bytes.** The default (`$XDG_RUNTIME_DIR/gramit-<user>.sock`) is nowhere near it, but `GRAMIT_SOCKET` overrides in deep temp directories can exceed it.
- `FixSelection` is wired into the protocol now and answers `NOT_IMPLEMENTED`, so the shape is stable before 2c fills it in.

**2b — clipboard** ✅
- [x] `gramit-input`: `Clipboard` trait over `arboard`, plus `Injector` and hotkey traits and fakes for all of them
- [x] Snapshot/restore semantics, verified against the real system clipboard (`cargo test -p gramit-input --test clipboard_live -- --ignored`)

Notes:

- `arboard`'s X11 backend works on this Wayland session — Mutter bridges the Xwayland and Wayland clipboards. UTF-8 and multi-line text round-trip intact.
- The clipboard lives on a **dedicated thread**, not behind `spawn_blocking`. `arboard::Clipboard` is blocking and not `Sync`, and on X11 the process that set the clipboard must stay alive to serve it — dropping the instance would drop the user's text with it.
- **Known limitation:** non-text clipboard content (images, files) reads as empty, so it is cleared rather than restored. Fixing it means compiling arboard's `image-data` support and round-tripping raw bitmaps for a rare case.

**2c — hotkey + injection, Linux first** — injection ✅, hotkey via fallback
- [x] `Injector` via the RemoteDesktop portal (`ashpd`), with the `restore_token` persisted to `~/.local/share/gramit/remote-desktop.token` (0600)
- [x] The fix loop end to end: clear → Copy → poll → backend → Paste → restore, with 13 unit tests over fakes
- [x] Single-flight gate: a second press while a fix runs is refused, not queued
- [x] `FixSelection` over IPC runs the real loop — verified live against the portal
- [x] GNOME custom-keybinding fallback implemented (`install` / `status` / `remove` / `manual_instructions`)
- [ ] Bind the fallback keybinding for real — **blocked on the `gramit` CLI (Module 3)**, since the shortcut runs `gramit fix --selection`
- [ ] Confirm a real paste lands in Chrome/Slack — needs a human with text selected

**The GlobalShortcuts portal will not work for us.** On GNOME 50 / xdg-desktop-portal 1.21 it rejects any app without a sandbox-provided app id:

    org.freedesktop.portal.Error.NotAllowed: An app id is required

This fails at `CreateSession`, before any consent dialog. Tried and ruled out: a plain binary; `systemd-run --scope` as `app-<appid>-<n>.scope` and `app-gnome-<appid>-<n>.scope`; a user service as `app-<appid>@<n>.service`; installing a matching `.desktop` file; restarting the portal to rescan it. All fail identically — the app id must come from a real Flatpak/Snap sandbox. Packaging gramit as a Flatpak would unlock it; until then the GNOME custom keybinding is the supported path on Linux, and the portal attempt stays in the code (it costs milliseconds and would start working under a sandbox).

**The RemoteDesktop portal does work**, and is what makes in-place replacement possible without root or `ydotool`. Two things learned the hard way:

- A stuck portal session makes every request fail with `Portal request was cancelled`. `systemctl --user restart xdg-desktop-portal-gnome xdg-desktop-portal` clears it — worth having `gramit doctor` suggest.
- With `PersistMode::ExplicitlyRevoked` and the saved token, later starts get a session **with no dialog at all**. Verified across several restarts.

**Design note — clear before copy.** The loop empties the clipboard before injecting Ctrl+C, then waits for anything to appear. Comparing against the previous contents instead would mistake a selection identical to the current clipboard for a failed copy. There is a test for exactly this.

**2d — Windows + macOS** — written and compile-verified, **not run**
- [x] Injector via `enigo` on its own thread; `Cmd` on macOS, `Ctrl` on Windows
- [x] Hotkey via `global-hotkey`, created on the main thread (both platforms require it)
- [x] `main` restructured: Tokio on worker threads, main thread pumps the platform event loop
- [x] Compile-verified — full workspace for `x86_64-pc-windows-msvc`, `gramit-input` for `aarch64-apple-darwin`
- [ ] **Runtime verification on real Windows/macOS hardware** — impossible from this Linux box; treat 2d as unproven until someone runs it

Cross-compiling paid for itself immediately — it caught two bugs that would otherwise have shipped:

- `char` does not implement `tracing::Value`, so `debug!(key, …)` failed to compile.
- `GlobalHotKeyManager` holds a raw `*mut c_void` (the message window) and is **not `Send`**, so it cannot move into a task. Only the event receiver crosses threads now; the manager stays put.

Both platforms constrain *which thread* owns the manager, and neither is optional:

- **Windows** — `GlobalHotKeyManager::new()` creates a hidden message window and receives `WM_HOTKEY` in its `WndProc`, but **pumps nothing itself**. The creating thread must dispatch messages, so `pump_until` runs `PeekMessageW`/`DispatchMessageW`.
- **macOS** — Carbon dispatches hotkeys on the **main** thread's run loop. `pump_until` calls `CFRunLoopRunInMode` in 0.25 s slices, so shutdown is a plain atomic read rather than cross-thread `CFRunLoopStop` surgery.

That is why `main` is no longer `#[tokio::main]`: the daemon runs on the runtime and the main thread becomes the pump. On Linux `pump_until` just idles, keeping one shape everywhere.

Also worth knowing: `notify-rust` cannot be cross-checked for macOS from Linux — its `mac-notification-sys` dependency compiles Objective-C and needs the Apple SDK. It builds fine on a real Mac. And macOS injection needs Accessibility (TCC); without it `enigo` fails to open the keyboard, and the error points at System Settings → Privacy & Security → Accessibility.

**2e — notifications** ✅
- [x] `notify-rust` toasts: "Fixed N issues", "Looks good already", "Nothing selected", plus a real reason on every failure
- [x] Raised from `selection::run`, so **both** the hotkey path and the IPC/keybinding path notify — on Linux the keybinding is the primary path, and notifying only from the hotkey loop would have left it silent
- [x] `notifications = false` swaps in a silent notifier; a recording notifier covers the wiring in tests
- [x] Verified live on GNOME — a real toast appeared (`notification shown summary=Nothing selected`)

Error codes become human summaries (`NO_API_KEY` → "gramit backend has no API key", `BACKEND_UNREACHABLE` → "gramit backend is not running"), with the raw detail in the body. Successes are Low urgency and failures Critical, so a fix that worked doesn't demand attention while a broken backend does.

**Every outcome notifies, including the boring ones.** A hotkey press has no terminal: if "already correct" were silent it would be indistinguishable from a daemon that had crashed.

**Done when**: selecting text in Chrome and Slack and pressing `Ctrl+Alt+F` replaces it in place with a toast, and the pre-existing clipboard contents survive.

### Module 3 — CLI (`gramit`) ✅

The user-facing surface; thin, since the daemon does the work.

- [x] `clap` command tree: `start` (detached spawn + readiness wait, `--foreground`), `stop`, `restart`, `status`
- [x] `fix [TEXT|-]` with `--clipboard` and `--selection`
- [x] `config get/set/path`, `logs [-f] [-n]`
- [x] `doctor` — config, daemon, backend, typing, hotkey — each failure printing a concrete remedy
- [x] `doctor --fix` installs the GNOME keybinding (verified: it appears in `gsettings` and runs the absolute path to `gramit fix --selection`)
- [x] `backend/dev-stub.mjs` — canned corrections, so the daemon can be worked on without Azure credentials
- [x] README covering setup, settings, and the per-platform permission story
- [ ] **Press `Ctrl+Alt+F` in a real app and watch the text change** — needs a human with text selected

Verified end to end against the dev stub:

```
$ gramit fix "he go to the store yesterday and buyed teh milk"
He goes to the store yesterday and bought the milk.
5 correction(s)

$ gramit fix --clipboard          # clipboard held: i cant recieve teh message
✓ clipboard fixed (5 correction(s))
$ xclip -selection clipboard -o
I can't receive the message.
```

Notes:

- **`--clipboard` is an IPC request, not local work.** On X11 the process that sets the clipboard must stay alive to serve it, so a CLI that exited immediately would take the corrected text with it. The daemon owns the clipboard; the round trip above proves it survives the CLI exiting.
- **`config get/set` round-trips through TOML** rather than matching on field names, so the `Config` struct stays the single source of truth — a new setting is settable the day it is added, and `deny_unknown_fields` rejects typos with a real message.
- **`fix --selection` notifies when there is no terminal.** It is what the desktop keybinding runs, so a failure the daemon cannot report (because it isn't running) would otherwise be completely silent.
- **Only the corrected text goes to stdout**; counts and status go to stderr, so `gramit fix` composes in a pipeline.

**Done when**: `gramit start` brings up a working daemon from a cold boot and `gramit doctor` passes clean on this machine.

---

## Verification

```bash
# backend
curl -s localhost:8787/health
curl -sX POST localhost:8787/v1/fix -H 'content-type: application/json' \
  -d '{"text":"he go to the store yesterday"}'

# daemon + CLI
gramit start --foreground        # terminal 1
echo "he go to the store" | gramit fix -
gramit fix --clipboard
gramit doctor
```

**Manual end-to-end** (the actual acceptance test): open Chrome and Slack, type a sentence with errors, `Ctrl+A`, `Ctrl+Alt+F` — text is replaced in place and a toast reports the change count. Then confirm the clipboard still holds whatever it held before the fix.

**Automated**: `cargo test` for config parsing, IPC round-trip, and the fix-loop state machine against fake `Clipboard`/`Injector` implementations; `vitest` for the prompt sanitizer, diff counter, cache, and route validation with a mocked Azure client.

## Not in v1

System tray, per-app rules, `gramit undo`, streaming responses, tone/formality modes, backend auth or hosting, telemetry, auto-update, OS installer packages, non-GNOME Linux desktops.
