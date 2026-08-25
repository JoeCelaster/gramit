# gramit on macOS — architecture and implementation plan

Status: **plan only, nothing here is built yet.** Today macOS *compiles*
(`gramit-input` for `aarch64-apple-darwin`) and has never been run. This document says
what "supports macOS" has to mean, what the user's path from install to first fix looks
like, and which file in this repo owns each piece of it.

Read `FLOW.md` first if you have not: everything from "save the clipboard" to "restore
the clipboard" is already platform-independent and does **not** change here. macOS
touches four things only: **how the daemon is launched**, **how the hotkey is caught**,
**how keystrokes are injected**, and **how permission to do that is obtained**.

---

## 1. Scope

**In scope (v1 for macOS)**

- Apple Silicon and Intel, macOS 13 Ventura and later.
- `gramit start/stop/restart/status/fix/config/logs/doctor` all behave as on Linux.
- The hotkey fixes the selection in any app that supports Cmd+C / Cmd+V.
- Accessibility permission is requested once, survives daemon restarts and gramit
  upgrades, and `gramit doctor` says exactly what to click when it is missing.
- The daemon starts at login and stays up.
- A supported install path (Homebrew) and a supported uninstall path.

**Out of scope for v1**

- A menu-bar UI / preferences window (the app bundle in Phase M5 is a container, not a
  GUI; a real menu bar item is a follow-up).
- Sandboxing / Mac App Store distribution — the App Store forbids the Accessibility
  API use that gramit is built on. Direct distribution only.
- Reading the selection without a copy (`capture = "primary"` has no macOS equivalent;
  the AX API can read focused-element text but only in AX-cooperative apps, so it is a
  later enhancement, not a v1 substitute).
- Touch Bar, Stage Manager, and Universal Control quirks.

---

## 2. What already exists, and what is missing

| Piece | State today | File |
|---|---|---|
| Main-thread CFRunLoop pump | **Written**, unverified | `crates/gramit-input/src/run_loop.rs:41` |
| Hotkey registration (Carbon via `global-hotkey`) | **Written**, unverified | `crates/gramit-input/src/native/hotkey.rs:35` |
| Injection (CGEvent via `enigo`), Cmd instead of Ctrl | **Written**, unverified | `crates/gramit-input/src/native/inject.rs:16` |
| Clipboard (NSPasteboard via `arboard`) | **Written**, unverified | `crates/gramit-input/src/clipboard.rs:64` |
| Daemon shape (`main` pumps, Tokio on workers) | **Written**, unverified | `crates/gramitd/src/main.rs:74` |
| Accessibility *permission handling* | **Missing** — we surface a hint string and nothing else | `native/inject.rs:98` |
| Launch-at-login / service management | **Missing** — `gramit start` bare-spawns `gramitd` | `crates/gramit-cli/src/lifecycle.rs:53` |
| Deterministic socket path under launchd | **Risk** — derived from `$TMPDIR`/`$USER` | `crates/gramit-core/src/paths.rs:31` |
| Notifications on macOS | **Broken as written** — `notify-rust` needs a bundle id | `crates/gramitd/src/notify.rs:79` |
| macOS `doctor` checks and `--fix` | **Stub** — one generic failure string | `crates/gramit-cli/src/doctor.rs:195` |
| Packaging / signing / notarization | **Missing** (`packaging/homebrew/` is an empty directory) | `packaging/` |
| CI on macOS | **Missing** (`.github/workflows/` is empty) | `.github/workflows/` |

So the work is not "port the input layer" — that part exists. The work is **the daemon's
relationship with the operating system**: launchd, TCC, code signing, and the diagnostics
that make those three legible to a user.

---

## 3. Target architecture

### 3.1 The daemon

```
                    ┌─────────────────────── login session (Aqua, gui/$UID) ──┐
                    │                                                          │
  launchd ──loads──>│  dev.gramit.gramitd  (LaunchAgent, RunAtLoad, KeepAlive) │
                    │        │                                                 │
                    │        └── /Applications/Gramit.app/Contents/MacOS/gramitd│
                    │                 │                                        │
                    │                 ├─ main thread: CFRunLoopRunInMode  ◀── Carbon hotkey
                    │                 ├─ Tokio workers: IPC server, fix loop   │
                    │                 ├─ clipboard thread (NSPasteboard)       │
                    │                 └─ injector thread (CGEventPost)         │
                    └──────────────────────────────────────────────────────────┘
                                      ▲
   gramit (CLI, /usr/local/bin/gramit)─┘  unix socket, newline JSON  (unchanged)
```

Four decisions, each with a reason:

**A LaunchAgent, never a LaunchDaemon.** Carbon's `RegisterEventHotKey` and
`CGEventPost` both need a connection to the WindowServer, which only exists inside a
GUI login session. A LaunchDaemon runs as root outside any session: it would register a
hotkey that can never fire. The plist therefore goes in `~/Library/LaunchAgents/` and is
bootstrapped into `gui/$UID`.

**launchd owns the process, not `gramit start`.** This is not a style preference — it is
how the permission survives. macOS attributes an Accessibility request to the
**responsible process**, which for a child of Terminal is *Terminal*, not `gramitd`. A
daemon bare-spawned from a shell (today's `lifecycle.rs:53`) makes the user grant
Accessibility to *Terminal.app* — which works, silently, until they close the terminal
or start it from a different one, and which never shows `gramit` in the permissions
list at all. Launched by launchd, `gramitd` is its own responsible process, so the grant
is attached to gramit and is stable. `gramit start` on macOS therefore becomes
`launchctl bootstrap` + `launchctl kickstart`, and `gramit stop` becomes an IPC
`Shutdown` (with `KeepAlive = { SuccessfulExit = false }` so a clean exit is not
resurrected) plus `launchctl bootout` for a full removal.

**`main` stays the run loop.** Already true and already correct — `main.rs:74` builds the
runtime by hand, registers the hotkey on the main thread, and hands the thread to
`run_loop::pump_until`. Nothing to change; this is the one macOS-shaped decision the
codebase already got right.

**The daemon binary lives inside an app bundle.** `Gramit.app` is a container, not a UI:
`LSUIElement = true` (no Dock icon, no menu), `CFBundleIdentifier = dev.gramit.Gramit`,
containing `gramitd`, `gramit`, and the LaunchAgent template. The bundle buys three
things that are painful to get any other way: a stable code-signing identity so the TCC
grant survives upgrades, a bundle id so notifications can be delivered as *gramit*
rather than as *Terminal*, and one thing for a user to drag to the Trash. The CLI is a
symlink into `/usr/local/bin` (or `/opt/homebrew/bin`).

**Paths.** `directories::ProjectDirs` already resolves these; the two marked ✎ need code
changes.

| What | Path on macOS | Owner |
|---|---|---|
| Config | `~/Library/Application Support/gramit/config.toml` | `paths::config_path` (works as-is) |
| Log | ✎ `~/Library/Logs/gramit/gramitd.log` (today: `Application Support/…`) | `paths::log_path` |
| State | `~/Library/Application Support/gramit/` | `paths::state_dir` (works as-is) |
| IPC socket | ✎ `~/Library/Application Support/gramit/gramitd.sock` (today: `$TMPDIR/gramit-$USER.sock`) | `paths::endpoint` |
| LaunchAgent | `~/Library/LaunchAgents/dev.gramit.gramitd.plist` | new `packaging/macos/` + CLI |

The socket move matters: under launchd the daemon's `$TMPDIR` and `$USER` are not
guaranteed to match the shell's, and if they diverge the CLI and the daemon bind
different paths and every command reports "not running". A fixed path under the user's
own Library removes the guess. It stays well under the 104-byte `sun_path` limit and
inherits `~/Library`'s ownership; the daemon still `chmod 0600`s it
(`endpoint.rs:69`).

### 3.2 Keys handling

Two different things get called "keys". Both are covered.

#### 3.2.1 Keystrokes — catching the hotkey

`RegisterEventHotKey` (Carbon), reached through `global-hotkey`, already implemented at
`native/hotkey.rs:35`. Properties that shape the design:

- **No permission needed.** Carbon hotkeys are not event taps, so they need neither
  Accessibility nor Input Monitoring. The hotkey works *before* the user grants
  anything — which is exactly why the first press must produce a clear "grant
  Accessibility" notification rather than silence.
- **Main thread only.** Carbon dispatches to the main run loop; `run_loop.rs` pumps it
  in 0.25 s slices so shutdown stays a plain atomic read.
- **Fires on key *down*.** Same as GNOME, so `modifier_release_ms` is just as necessary
  here (§3.2.2).
- **Exclusive.** A second registration of the same combination fails; a system shortcut
  wins. `register()` already turns that into "another application may already own it" —
  on macOS `doctor` should additionally suggest a fallback combination.
- **Default hotkey.** Stays `Ctrl+Alt+F` on every platform. An earlier draft of this
  plan proposed `Ctrl+Cmd+F` on the grounds that it has "no system owner" — that is
  wrong: `⌃⌘F` is macOS's standard **Enter/Exit Full Screen**, present in the View menu
  of nearly every app, so it is a far worse choice than what it replaced. `Ctrl+Alt+F`
  registers cleanly on macOS (Alt = Option) and users already know it; only its
  *spelling* needed fixing, which `hotkey_spec::display()` now handles by rendering it
  as `Ctrl+Option+F` wherever a Mac user reads it.

#### 3.2.2 Keystrokes — injecting copy and paste

`CGEventPost` through `enigo`, at `native/inject.rs`. Three changes:

1. **Release the user's modifiers first.** The Linux injector explicitly releases all
   eight modifier keysyms before pressing anything (`linux/inject.rs:118`). The native
   injector does not — it presses Cmd and taps the key. Since the hotkey fires on key
   down, the user is still holding Ctrl+Cmd, so the injected event reaches the app as
   **Ctrl+Cmd+C**, which copies nothing. Add a `release_modifiers()` step over
   Shift/Control/Alt/Meta (both sides) and, because CGEvent flags are per-event, set the
   injected event's flags explicitly to *only* Command.
2. **Use virtual key codes, not Unicode.** `Key::Unicode('c')` asks enigo to find a
   keycode for the character in the *current layout*; on Dvorak, AZERTY or a Colemak
   remap that is not the physical C key, so Cmd+C lands on the wrong key. Cmd+C and
   Cmd+V are position-based shortcuts on every layout, so use raw codes:
   `kVK_ANSI_C = 8`, `kVK_ANSI_V = 9`, via `enigo::Key::Other`.
3. **Detect secure input.** When a password field owns the keyboard, macOS enables
   `EnableSecureEventInput` and silently drops every posted event — the exact symptom of
   a missing permission, with a completely different fix. Wrap
   `IsSecureEventInputEnabled()` and report it as its own outcome
   (`SECURE_INPUT_ACTIVE` → "A password field is blocking typing").

Everything downstream — clipboard snapshot, clear, poll-and-retry, paste, restore — is
unchanged. `Cmd` vs `Ctrl` is already handled by the `MODIFIER` constant.

#### 3.2.3 Secret keys and stored credentials

Worth stating plainly because it constrains the packaging: **gramit stores no API key on
the user's machine.** The Azure key lives only in the backend
(`backend/.env`, or the hosted deployment named by `deploy.toml`). The macOS install
therefore has no secret to protect and no Keychain dependency in v1.

What *is* on disk after install: `config.toml` (no secrets), the log (no selection text —
only lengths and error codes), and the socket (0600). Linux additionally stores the
portal `restore_token` (`linux/token.rs`); **macOS has no equivalent** — the TCC grant is
held by the system, keyed to our code signature, which is precisely why signing identity
stability is a correctness concern and not a distribution nicety.

If a future release adds per-user auth to the hosted backend, the token goes in the
login Keychain via the `keyring` crate with
`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, in a new `gramit-core/src/secrets.rs`
behind a trait — never in `config.toml`. Flagged here so nobody puts it in the config
struct out of convenience.

### 3.3 Permissions

| Capability | Permission needed | Prompted by | If missing |
|---|---|---|---|
| Register the hotkey | none | — | — |
| Post Cmd+C / Cmd+V | **Accessibility** (TCC) | first `CGEventPost`, or our explicit prompt | events silently dropped |
| Read/write the clipboard | none | — | — |
| Read another app's text directly | Accessibility | — | not used in v1 |
| Show a notification | user-visible authorization, per bundle id | first notification | toast never appears |
| Listen to all keystrokes | Input Monitoring | — | **never requested** — gramit is not a keylogger and asks for nothing that would let it be one |

**The Accessibility state machine.** Only the process that needs the right can ask about
it, so this lives in the daemon, not the CLI:

```
daemon start
   │
   ├─ AXIsProcessTrusted() == true ──> injector opens, selection_ready = true
   │
   └─ false
        ├─ first ever run: AXIsProcessTrustedWithOptions({prompt: true})
        │     └─ macOS shows "gramit would like to control this computer"
        ├─ log ACCESSIBILITY_DENIED, keep serving IPC
        │     (so `gramit fix "text"` and `gramit doctor` still work — degraded, not dead)
        └─ poll AXIsProcessTrusted() every 2 s for the first 5 minutes
              └─ becomes true → open the injector, no restart needed
```

That poll is worth the twenty lines: macOS grants Accessibility to a *running* process
without restarting it, and "grant, then run `gramit restart`" is a step users forget and
then report as a bug.

**Notifications.** `notify-rust` on macOS goes through `mac-notification-sys`, which
requires a registered bundle identifier and defaults to impersonating
`com.apple.Terminal`. Inside `Gramit.app` we call `notify_rust::set_application(
"dev.gramit.Gramit")` once at startup; if that fails (binary running outside the bundle,
e.g. a `cargo build` dev loop) fall back to `osascript -e 'display notification …'` and
log that toasts will be attributed to Script Editor. A fix the user cannot see is a fix
they will assume failed, so this is not optional polish.

**Code signing is part of the permission design.** TCC keys the grant to the binary's
identity. Ad-hoc-signed or unsigned binaries are keyed by path + cdhash, so *every
rebuild and every `brew upgrade` revokes Accessibility* — this is the single biggest
source of "it stopped working" reports on tools like this. Developer ID signing with a
stable bundle id and hardened runtime, then notarization, makes the grant survive
upgrades and removes the Gatekeeper warning. Until a signing certificate exists, the
beta path is honest about it: `doctor` detects an unsigned build and warns that an
upgrade will require re-granting.

---

## 4. User flow, install to first fix

### 4.1 Install (recommended path, Homebrew)

```bash
brew install --cask tryhitchikersway/tap/gramit
```

The cask drops `Gramit.app` in `/Applications` and symlinks `gramit` into the Homebrew
`bin`. Nothing is running yet, nothing has been granted yet. (Terminal-only users who
prefer plain binaries: `brew install tryhitchikersway/tap/gramit` — same daemon, no
bundle, notifications degraded, see §3.3.)

### 4.2 First run

```bash
$ gramit setup
```

One command, because the alternative is a README the user has to follow in order.
`gramit setup` runs, in order, printing each step:

```
  ✓ config          ~/Library/Application Support/gramit/config.toml (created)
  ✓ launch agent    installed and loaded (dev.gramit.gramitd)
  ✓ daemon          running, version 0.1.0 (pid 4183)
  ✓ backend         your URL
  ⚠ accessibility   gramit needs permission to type into other apps.

    macOS just asked, or will ask now. In System Settings → Privacy &
    Security → Accessibility, switch on **gramit**.

    Opening that panel for you…                 (press Return when granted)
```

The system dialog is macOS's own — *"gramit.app would like to control this computer
using accessibility features"* — with **Open System Settings** / **Deny**. `setup` opens
`x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility` as well,
because the dialog is easy to dismiss by reflex.

The moment the toggle flips, the daemon's poll notices and opens the injector — no
restart:

```
  ✓ accessibility   granted
  ✓ hotkey          Ctrl+Cmd+F

  gramit is ready. Select text anywhere and press Ctrl+Cmd+F.
```

`gramit setup` is idempotent: run it after any upgrade, or when something feels wrong.
Under the hood it is `gramit doctor --fix` with first-run wording.

### 4.3 Using it

Identical to Linux, one key different:

1. Select text in any app — Mail, Slack, Notes, a browser textarea.
2. Press **Ctrl+Cmd+F**.
3. ~0.5–1.5 s later the selection is replaced, and a notification says *Fixed 3 issues*
   / *Looks good already* / *Nothing selected*.

Undo (Cmd+Z) restores the original: the correction arrives as a single paste. The
clipboard the user had before the fix is put back on every path, including failures.

```bash
gramit fix "he go to the store"    # no permissions involved, good for a first test
gramit fix --clipboard             # correct the clipboard in place
gramit status                      # daemon, hotkey, typing, backend
gramit config set hotkey Cmd+Shift+G && gramit restart
gramit logs -f
gramit doctor                      # says what to click, per failure
```

### 4.4 Login, upgrade, uninstall

- **Login:** the LaunchAgent is `RunAtLoad`, so gramit is up before the user's first app.
- **Upgrade:** `brew upgrade --cask gramit`; the LaunchAgent path is stable, launchd
  restarts the daemon, and a Developer-ID-signed build keeps the Accessibility grant.
  (Unsigned beta builds will ask again — `gramit setup` walks it.)
- **Uninstall:** `brew uninstall --cask gramit` runs a `zap` stanza that boots out and
  removes the LaunchAgent, deletes the socket and log, and leaves `config.toml` unless
  `--zap` is passed. A stale entry may remain in the Accessibility list; the uninstall
  message says so and how to remove it.

### 4.5 When it does not work

`gramit doctor` is the single entry point, and every failure carries its remedy:

| Symptom | `doctor` says |
|---|---|
| Nothing happens on the hotkey | `hotkey  Ctrl+Cmd+F is not registered — another app owns it. Try: gramit config set hotkey Ctrl+Cmd+G` |
| "Nothing selected" every time | `typing  Accessibility is not granted → System Settings → …` (with the panel opened) |
| Nothing pastes in one app only | `typing  secure input is active (a password field has focus) — click into the text field first` |
| Works from Terminal, not at login | `service  the launch agent is not loaded → gramit setup` |
| Stopped after an upgrade | `typing  this build is not Developer ID signed, so macOS revoked Accessibility on upgrade — re-grant it` |
| Toasts never appear | `notifications  not authorized for dev.gramit.Gramit → System Settings → Notifications` |

---

## 5. Implementation plan

Seven phases. M0–M3 make it work for us; M4–M5 make it work for a stranger; M6–M7 keep
it working. Each phase ends with something verifiable on a real Mac.

### M0 — Get it building and running on real hardware (½ day)

The current state is "compiles for one target from Linux". First establish ground truth.

- `.github/workflows/macos.yml`: `macos-14` runner, `cargo test --workspace`,
  `cargo build --release` for `aarch64-apple-darwin` **and** `x86_64-apple-darwin`,
  `lipo -create` into a universal binary. This also gives the first full-workspace macOS
  check ever, including `notify-rust`, which cannot be cross-checked from Linux.
- Run through `TESTING.md §6` by hand and write down what actually breaks.

*Exit:* the workspace tests pass on a macOS runner; a universal `gramitd` runs and
answers `gramit status`.

### M1 — Keys: make injection actually land (1 day)

`crates/gramit-input/`

- `native/inject.rs` — add `release_modifiers()` (mirror `linux/inject.rs:118`); set
  CGEvent flags explicitly; switch to virtual key codes 8/9 via `Key::Other`; construct
  `enigo::Settings` explicitly rather than `default()` so the permission-prompt
  behaviour is ours and not the crate's.
- new `macos/mod.rs`, `macos/permissions.rs` — thin FFI over
  `AXIsProcessTrusted`, `AXIsProcessTrustedWithOptions(kAXTrustedCheckOptionPrompt)`,
  and `IsSecureEventInputEnabled` (ApplicationServices + Carbon, no new crates). Public
  API: `is_trusted() -> bool`, `request_trust()`, `secure_input_active() -> bool`.
- `lib.rs` — `#[cfg(target_os = "macos")] pub use macos::permissions as macos_permissions;`
  alongside the existing `linux_gnome` re-export, and a new
  `InputError::PermissionDenied` variant with code `ACCESSIBILITY_DENIED`.
- `config.rs` (core) — per-platform default hotkey (`Ctrl+Cmd+F` on macOS).

*Exit:* with Accessibility granted by hand, the hotkey replaces a selection in
TextEdit, Chrome and Slack, on a US and a non-US keyboard layout.

### M2 — Permissions: the state machine and honest status (1 day)

`crates/gramitd/`

- `main.rs` — before `open_selection()`, on macOS: check trust, prompt on first run,
  and if untrusted still start (IPC + `gramit fix` keep working). Spawn the 2 s / 5 min
  trust poll that opens the injector when the toggle flips, promoting
  `selection_ready` without a restart.
- `state.rs` — hold the injector behind the same `OnceCell`/`RwLock` the poll writes, so
  status reflects reality.
- `notify.rs` — `set_application("dev.gramit.Gramit")` at startup; `osascript` fallback;
  add `SECURE_INPUT_ACTIVE` and `ACCESSIBILITY_DENIED` to `summary_for_code`.
- `fixloop.rs` — before injecting, if secure input is active, return
  `Failed{SECURE_INPUT_ACTIVE}` instead of a bare "nothing captured": same observable
  symptom, different remedy, and the diagnostic is the whole point.

`crates/gramit-core/src/ipc.rs`

- `StatusReport` gains `#[serde(default)]` fields: `accessibility: Option<bool>`,
  `secure_input: Option<bool>`, `service_managed: Option<bool>`, `signed: Option<bool>`.
  `#[serde(default)]` is load-bearing — a new CLI must not fail against an old daemon.
- `handler.rs:134` fills them in.

*Exit:* `gramit status` distinguishes granted / not-granted / blocked-by-secure-input;
granting the toggle makes typing work with no restart.

### M3 — The daemon as a launchd service (1–1½ days)

`packaging/macos/dev.gramit.gramitd.plist.in` — `Label`, `ProgramArguments`
(absolute path to `gramitd`), `RunAtLoad = true`,
`KeepAlive = { SuccessfulExit = false }` (so `gramit stop` stays stopped),
`ProcessType = Interactive`, `StandardErrorPath` to the log,
`EnvironmentVariables` for `GRAMIT_*` overrides.

`crates/gramit-cli/`

- new `service.rs` — a `Service` trait (`install`, `uninstall`, `is_installed`,
  `is_loaded`, `start`, `stop`) with a `macos` implementation shelling out to
  `launchctl bootstrap|bootout|kickstart|print gui/$UID/…`, and a `noop` implementation
  everywhere else. Contained here so `lifecycle.rs` keeps one shape.
- `lifecycle.rs` — on macOS, `start` = install-if-needed + `kickstart` + wait-until-ready
  (replacing the bare `Command::spawn` at `:53`); `stop` = IPC `Shutdown` then confirm;
  `restart` = `kickstart -k`.
- new `setup.rs` — `gramit setup`: the §4.2 flow (config → agent → daemon → backend →
  accessibility, with the settings panel opened and a "press Return when granted" wait).
- `main.rs` — register the `Setup` subcommand.

`crates/gramit-core/src/paths.rs` — macOS socket to
`~/Library/Application Support/gramit/gramitd.sock`; log to `~/Library/Logs/gramit/`.
Both stay overridable by `GRAMIT_SOCKET` / `GRAMIT_LOG`, which the daemon tests rely on.

*Exit:* reboot the Mac, log in, select text, press the hotkey — it works, with no
terminal ever opened.

### M4 — `doctor` that can diagnose a Mac (½–1 day)

`crates/gramit-cli/src/doctor.rs` — a `check_macos_*` set mirroring the Linux
`check_gnome_keybinding` structure (`:252`):

| Check | Failure remedy | `--fix` action |
|---|---|---|
| launch agent installed + loaded | `gramit setup` | install & bootstrap |
| Accessibility granted (from `StatusReport`) | open the Accessibility panel | `open x-apple.systempreferences:…` + prompt via the daemon |
| secure input inactive | click into the text field | — |
| hotkey registered | suggest an alternative combination | — |
| binary Developer ID signed (`codesign -dv`) | warn: upgrades will revoke Accessibility | — |
| notification authorization | System Settings → Notifications | — |

`check_typing` (`:195`) keeps its macOS wording but now reads the new status fields
instead of guessing.

*Exit:* every row of the §4.5 table is reachable by breaking the corresponding thing.

### M5 — Bundle, sign, notarize, distribute (1–2 days, needs an Apple Developer account)

- `packaging/macos/Gramit.app/Contents/Info.plist.in` — `CFBundleIdentifier
  dev.gramit.Gramit`, `LSUIElement true`, `LSMinimumSystemVersion 13.0`,
  `NSAppleEventsUsageDescription` (for the `osascript` notification fallback).
- `packaging/macos/build.sh` — universal binaries → bundle → `codesign --deep
  --options runtime --timestamp` with the Developer ID → `create-dmg` →
  `notarytool submit --wait` → `stapler staple`.
- `packaging/homebrew/gramit.rb` (cask + formula) — cask installs the app and symlinks
  the CLI; `zap` stanza boots out the agent and removes the plist, socket, log, and
  (with `--zap`) the config.
- README: replace the six-line macOS note with the §4 flow.

*Exit:* a fresh Mac, `brew install --cask …`, `gramit setup`, first fix — with no
Gatekeeper warning and no build tools installed.

### M6 — Documentation and tests (½ day)

- `TESTING.md §6` — rewrite from "nobody has run this" to a real checklist: permission
  matrix, the app list to try, layout matrix, secure-input case, reboot case, upgrade
  case.
- `FLOW.md` — extend the platform table into a macOS trace: Carbon hotkey → daemon (no
  CLI process) → modifier release → Cmd+C → … → Cmd+V.
- Unit tests, all runnable on Linux: the macOS default hotkey renders to
  `Modifiers::META | CONTROL` + `KeyF`; the new `StatusReport` fields round-trip and
  decode from a payload that omits them; the launchd plist template renders with the
  right label and path; `notification_for` covers the two new codes.
- macOS-only integration test behind `#[cfg(target_os = "macos")]` + `--ignored`:
  register a hotkey, post it synthetically, assert the fix loop runs.

### M7 — Follow-ups, explicitly deferred

Menu-bar UI; `capture = "ax"` reading the focused element through the Accessibility API
(no clipboard round trip, works where Cmd+C does not); Sparkle-style self-update; a
Shortcuts.app action; Apple Silicon-native local model for offline correction.

---

## 6. File-by-file responsibility

| File | Responsibility on macOS | Phase |
|---|---|---|
| `crates/gramit-input/src/run_loop.rs` | CFRunLoop pump on the main thread | done |
| `crates/gramit-input/src/native/hotkey.rs` | Carbon hotkey registration + event forwarding | done |
| `crates/gramit-input/src/native/inject.rs` | Cmd+C/Cmd+V via CGEvent; modifier release; virtual key codes | **M1** |
| `crates/gramit-input/src/macos/permissions.rs` *(new)* | AX trust check, prompt, secure-input probe | **M1** |
| `crates/gramit-input/src/macos/mod.rs` *(new)* | macOS module root | **M1** |
| `crates/gramit-input/src/lib.rs` | re-export `macos_permissions`; `PermissionDenied` error + code | **M1** |
| `crates/gramit-input/src/clipboard.rs` | NSPasteboard via arboard; `get_primary_text` already `None` | done |
| `crates/gramit-input/src/hotkey_spec.rs` | already parses `Cmd`; no change | done |
| `crates/gramit-core/src/config.rs` | per-platform default hotkey | **M1** |
| `crates/gramit-core/src/paths.rs` | macOS socket + log locations | **M3** |
| `crates/gramit-core/src/ipc.rs` | `StatusReport` permission fields, `#[serde(default)]` | **M2** |
| `crates/gramitd/src/main.rs` | trust check, first-run prompt, trust poll, degraded start | **M2** |
| `crates/gramitd/src/state.rs` | late-opening injector; permission state for status | **M2** |
| `crates/gramitd/src/handler.rs` | report the new fields | **M2** |
| `crates/gramitd/src/notify.rs` | bundle id, `osascript` fallback, new code wording | **M2** |
| `crates/gramitd/src/fixloop.rs` | secure-input pre-check before injecting | **M2** |
| `crates/gramit-cli/src/service.rs` *(new)* | launchctl install/load/start/stop | **M3** |
| `crates/gramit-cli/src/lifecycle.rs` | start/stop/restart via the service on macOS | **M3** |
| `crates/gramit-cli/src/setup.rs` *(new)* | `gramit setup` first-run flow | **M3** |
| `crates/gramit-cli/src/main.rs` | `setup` subcommand | **M3** |
| `crates/gramit-cli/src/doctor.rs` | macOS checks and `--fix` actions | **M4** |
| `packaging/macos/dev.gramit.gramitd.plist.in` *(new)* | LaunchAgent template | **M3** |
| `packaging/macos/Gramit.app/Contents/Info.plist.in` *(new)* | bundle identity, `LSUIElement` | **M5** |
| `packaging/macos/build.sh` *(new)* | universal build, sign, notarize, DMG | **M5** |
| `packaging/homebrew/gramit.rb` *(new)* | cask/formula + `zap` uninstall | **M5** |
| `.github/workflows/macos.yml` *(new)* | test + universal build; release signing | **M0/M5** |
| `README.md`, `TESTING.md`, `FLOW.md` | user-facing macOS truth | **M6** |

Unchanged and deliberately so: `fixloop.rs`'s capture/correct/paste sequence,
`selection.rs`'s single-flight gate, `server.rs`, `endpoint.rs`, the IPC protocol
shape, and the entire `backend/`.

---

## 7. Known landmines (found by reading the code, not by running it)

1. **`native/inject.rs` never releases the user's held modifiers** — Linux does
   (`linux/inject.rs:118`), so on macOS the injected Cmd+C will arrive as
   Ctrl+Cmd+Alt+C and copy nothing. Most likely cause of "the hotkey does nothing".
   → M1.
2. **`Key::Unicode('c')` is layout-dependent** — wrong physical key on non-QWERTY.
   → M1.
3. **`notify-rust` needs a bundle id on macOS** and defaults to impersonating Terminal;
   from an unbundled binary, toasts are the user's only feedback and they will be
   missing or mis-attributed. → M2.
4. **Bare-spawning `gramitd` from a shell attributes Accessibility to Terminal**, so the
   user grants the wrong app and gramit never appears in the list. → M3.
5. **`$TMPDIR`/`$USER`-derived socket path** can differ between the shell and launchd,
   producing "gramit is not running" from a running daemon. → M3.
6. **`paths::state_dir()` returns `None` on macOS** — already handled by the
   `data_local_dir()` fallback at `paths.rs:82`, but the log lands in
   `Application Support` rather than `~/Library/Logs`. Cosmetic; fixed in M3.
7. **Adding `StatusReport` fields without `#[serde(default)]`** breaks a new CLI against
   an old daemon — the struct has no container-level default today. → M2.
8. **Ad-hoc / unsigned builds lose Accessibility on every rebuild and upgrade.** Expect
   it during development (`TESTING.md` already warns); fix properly with Developer ID
   in M5.
9. **`Capture::Primary` is meaningless on macOS** — `get_primary_text` returns `None`, so
   `capture = "primary"` yields a permanent "Nothing selected". `config.validate()`
   should reject it on macOS with a clear message. → M2.
10. **`enigo::Settings::default()`** may prompt for permissions on its own schedule; take
    control of that so our prompt is the only one and it is timed with our messaging.
    → M1.

---

## 8. Risks and open questions

| Risk | Impact | Mitigation |
|---|---|---|
| Carbon hotkeys may need more than a WindowServer connection from a non-`.app` process | hotkey never fires in the CLI-only install | verify in M0; if so, the app bundle (M5) is promoted to a hard requirement and the plain formula is dropped |
| Apple deprecating Carbon `RegisterEventHotKey` | future breakage | contained to `native/hotkey.rs`; a `CGEventTap` replacement is a drop-in behind the same trait — at the cost of needing Input Monitoring, which we would rather not ask for |
| No Apple Developer account yet | M5 blocked; Gatekeeper warnings and revoked grants | M0–M4 ship as a signed-ad-hoc beta with `doctor` warning about it |
| `arboard` clipboard behaviour on macOS under a `clear()` + poll loop | false "nothing selected" | M0 measurement; if unreliable, switch the poll to `NSPasteboard.changeCount`, which is the native signal for "something was copied" |
| Apps that block synthetic events (some VMs, secure terminals, remote desktop clients) | hotkey silently no-ops there | detect secure input, document the app list in `TESTING.md` |
| `enigo`'s macOS backend expectations about threads | injector thread misbehaves | the injector already owns a dedicated thread; if enigo needs the main thread, move injection onto the run-loop thread via a channel |

**Open questions for the owner**

1. Is there an Apple Developer account (needed for M5 signing + notarization)? If not,
   how is the beta distributed — signed-ad-hoc tarball, or unsigned with instructions?
2. Homebrew **cask** (app bundle, better permissions story) or **formula** (plain
   binaries, terminal-native) as the primary path? The plan assumes cask primary,
   formula secondary.
3. `Ctrl+Cmd+F` as the macOS default — or match Linux's `Ctrl+Alt+F` for consistency
   across platforms at the cost of being unidiomatic here?
4. Is there a Mac available for M0, or does the first phase have to run entirely on
   GitHub's `macos-14` runners (which cannot test permissions, hotkeys, or the run loop
   — they are headless)?
