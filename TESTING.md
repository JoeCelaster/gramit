# Testing gramit locally

How to run the backend and exercise gramit on Linux, macOS and Windows.

**Status of each platform**, so you know what you're walking into:

| Platform | State |
|---|---|
| **Linux** (GNOME / Wayland) | Verified end to end on GNOME 50 / Ubuntu |
| **macOS** | Compiles; **never run on real hardware** — expect to find bugs |
| **Windows** | Compiles; **never run on real hardware** — expect to find bugs |

If you are the first person to run this on macOS or Windows, skip to
[First run on macOS](#6-first-run-on-macos) or [First run on Windows](#7-first-run-on-windows) —
those sections list exactly what to check and what failure looks like.

---

## 1. Run the backend

The backend is the only component holding an API key. It listens on `127.0.0.1:8787`.

### Option A — the real backend (Azure OpenAI)

```bash
cd backend
npm install
cp .env.example .env
```

Fill in `.env`:

```
AZURE_OPENAI_ENDPOINT=https://<your-resource>.openai.azure.com
AZURE_OPENAI_API_KEY=<your key>
AZURE_OPENAI_DEPLOYMENT=gpt-5.6-luna
AZURE_OPENAI_API_VERSION=<your api version>
```

Then:

```bash
npm run build && npm start      # or: npm run dev   (watch mode, no build step)
```

Check it:

```bash
curl -s localhost:8787/health
# {"ok":true,"version":"0.1.0","hasKey":true,"model":"gpt-5.6-luna","missing":[]}

curl -sX POST localhost:8787/v1/fix \
  -H 'content-type: application/json' \
  -d '{"text":"he go to the store yesterday"}'
# {"corrected":"He goes to the store yesterday.","changed":true,"changes":2,...}
```

`hasKey` is the field that matters. If it is `false`, `missing` names the environment
variables that aren't set, and every fix will fail with `NO_API_KEY`.

### Option B — the dev stub (no credentials needed)

For working on the daemon or the CLI, the stub is usually the better choice: it is
instant, free, and deterministic, so you can assert on exact output.

```bash
node backend/dev-stub.mjs
# gramit dev stub listening on http://127.0.0.1:8787
```

It applies a handful of canned substitutions (`he go` → `he goes`, `teh` → `the`,
`buyed` → `bought`, capitalise, add a full stop) instead of calling a model. Same URLs,
same response shape, so gramit cannot tell the difference. Every mode is served: code
mode gets a stub call in place of the request comment, and write mode gets a canned
email, essay or paragraph in the shape the brief asked for.

Stop it with `Ctrl+C`, or `pkill -f dev-stub.mjs`.

---

## 2. Build gramit

```bash
cargo build --release
```

This produces two binaries in `target/release/`:

- `gramit` — the CLI you type
- `gramitd` — the daemon that does the work

`gramit start` looks for `gramitd` **next to itself** first, then on `PATH`. Keeping
them together is all that's required.

To use `gramit` from anywhere without installing:

```bash
export PATH="$PWD/target/release:$PATH"          # Linux / macOS
$env:PATH = "$PWD\target\release;$env:PATH"      # Windows PowerShell
```

Use `target/debug/` instead if you built with plain `cargo build`.

---

## 3. Start it and check the setup

```bash
gramit start
gramit doctor
```

`doctor` is the command to reach for whenever anything is wrong — every failed check
prints what to do about it. A healthy system looks like this:

```
✓ config
    /home/you/.config/gramit/config.toml
✓ daemon
    running, version 0.1.0 (pid 75058)
✓ backend
    http://127.0.0.1:8787
✓ typing
    RemoteDesktop portal (Wayland)
✓ hotkey
    Ctrl+Alt+F via a GNOME keybinding

everything looks good
```

On Linux, `gramit doctor --fix` installs the hotkey for you (see the Linux section).

Paths differ per OS, so ask rather than guess:

```bash
gramit config path     # where settings live
gramit logs -n 50      # the daemon log
gramit logs -f         # follow it while you test
```

---

## 4. The test that actually matters

Everything below is the same on all three platforms. Automated tests cover the logic;
**this is the only thing that proves the whole chain works.**

1. Make sure the backend is running and `gramit doctor` is clean.
2. Open any app with a text field — a browser, a chat client, a text editor.
3. Type: `he go to the store yesterday and buyed teh milk`
4. Select it (`Ctrl+A`, or `Cmd+A` on macOS).
5. Press **`Ctrl+Alt+F`**.

Expected:

- The text is replaced with `He goes to the store yesterday and bought the milk.`
- A notification appears: *Fixed 5 issues*
- **Your clipboard still holds whatever it held before** — check with `Ctrl+V` somewhere

Then check the edge cases, which are where the bugs hide:

| Try this | Expected |
|---|---|
| Press the hotkey with **nothing selected** | Toast: *Nothing selected*. Nothing is typed. |
| Select text that is **already correct** | Toast: *Looks good already*. Nothing is pasted — no undo step. |
| Copy something first, then fix a selection | Your original clipboard is back afterwards |
| Select text **identical to your clipboard** | Still fixed (this one has caught bugs before) |
| Stop the backend, then press the hotkey | Toast: *gramit backend is not running* |
| Hold the hotkey down | One fix, not a burst |

Without a hotkey you can drive the same path directly, which is useful for isolating
whether a problem is the shortcut or the loop behind it:

```bash
gramit fix --selection     # select text first, then run this
```

Quicker checks that need no selection at all:

```bash
gramit fix "he go to the store"     # prints the corrected text
echo "he go" | gramit fix -         # reads stdin
gramit fix --clipboard              # corrects the clipboard in place
gramit status                       # what the daemon thinks is going on
```

---

## 5. Linux (GNOME / Wayland)

This is the verified path. Two separate mechanisms, and only one is a portal.

### Typing — the RemoteDesktop portal

The first `gramit start` shows a GNOME dialog asking to allow remote control.
**Approve it.** A restore token is saved to `~/.local/share/gramit/remote-desktop.token`,
so it never asks again.

`gramit doctor` reports this as the **typing** check. If it fails, the portal session
is usually stuck:

```bash
systemctl --user restart xdg-desktop-portal-gnome xdg-desktop-portal
gramit restart
```

To force the consent dialog back (e.g. to test the first-run experience):

```bash
rm ~/.local/share/gramit/remote-desktop.token
gramit restart
```

### The hotkey — a GNOME custom keybinding

The GlobalShortcuts portal refuses apps without a sandbox app id, so the daemon cannot
bind a shortcut itself. `doctor` installs a GNOME keybinding instead:

```bash
gramit doctor --fix
```

Verify it landed:

```bash
gsettings get org.gnome.settings-daemon.plugins.media-keys custom-keybindings
K="org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/gramit/"
gsettings get "$K" binding    # '<Control><Alt>f'
gsettings get "$K" command    # '/abs/path/to/gramit fix --selection'
```

It also shows up in Settings → Keyboard → View and Customize Shortcuts → Custom
Shortcuts, where you can rebind it by hand.

Because `gramit status` shows the hotkey as *not bound by the daemon*, that is expected
on Linux and not a fault.

The command must be an **absolute path** — the desktop does not inherit your shell's
`PATH`. If you rebuild into a different directory, re-run `gramit doctor --fix`.

To remove it:

```bash
gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings "[]"
```

### Non-GNOME desktops

Out of scope for v1. The typing half only needs the RemoteDesktop portal, so it may
work on other Wayland compositors; the hotkey half is GNOME-specific and you would
bind `gramit fix --selection` using your own desktop's shortcut settings.

---

## 6. First run on macOS

**Nobody has run this yet.** It compiles for `aarch64-apple-darwin`; that is all we know.

```bash
cargo build --release
export PATH="$PWD/target/release:$PATH"
node backend/dev-stub.mjs &     # or run the real backend
gramit start
gramit doctor
```

### Accessibility permission is mandatory

macOS will not let any process synthesise keystrokes without it. On first run you
should get a system prompt; if not, add it by hand:

**System Settings → Privacy & Security → Accessibility** → add `gramitd`.

Then `gramit restart`.

The permission attaches to *that exact binary*, so **rebuilding may revoke it** — if
typing suddenly stops after a rebuild, re-check that panel first. For that reason,
prefer testing a stable `target/release/gramitd` over rebuilding repeatedly.

`gramit doctor` reports this as the **typing** check, with the same wording.

### What to check, in order

1. `gramit status` — does **typing** say `CGEvent via enigo (macOS)`?
2. `gramit status` — does **hotkey** say `Ctrl+Alt+F` as registered (not "not bound by the daemon")? Unlike Linux, the daemon registers the hotkey itself here.
3. `gramit fix "he go"` — proves the CLI → daemon → backend chain, no permissions involved.
4. **Did Carbon actually deliver it?** Select text, press the hotkey, then `gramit logs`.
   `hotkey_loop` logs `hotkey pressed` for every delivered press. That one line splits the
   two failures that look identical from the outside: no line means the event never
   arrived (the main thread is not dispatching); a line followed by `nothing was
   selected` means it arrived and the fix itself failed. Registration succeeding tells
   you nothing here — it reserves the chord with the WindowServer, no more.
5. Then the [real test](#4-the-test-that-actually-matters).

### Failure modes specific to macOS

| Symptom | Likely cause |
|---|---|
| Hotkey registers but **never fires** | The main thread is not *dispatching*. Carbon only queues the press; `run_loop::pump_until` has to dequeue it (`ReceiveNextEvent`) and hand it to the dispatcher target. Registration succeeding proves nothing — it reserves the chord with the WindowServer and says nothing about delivery. A CFRunLoop-only pump looks healthy in `doctor` and delivers not one press; that was the bug through 0.1.1. |
| Text is captured but nothing pastes | Accessibility not granted, or granted to a stale binary |
| Copy/paste does nothing at all | gramit sends **Cmd**+C/V on macOS; if it is sending Ctrl, that's a bug in the modifier selection. Also check the modifiers you were holding: the hotkey fires on press, so an unreleased `Ctrl+Option` turns the injected `Cmd+C` into `Ctrl+Option+Cmd+C`, which copies nothing. `send_chord` releases them first for exactly this reason. |
| Daemon exits immediately | Check `gramit logs` |

---

## 7. First run on Windows

**Nobody has run this yet.** It compiles for `x86_64-pc-windows-msvc`; that is all we know.

No special permissions are needed.

```powershell
cargo build --release
$env:PATH = "$PWD\target\release;$env:PATH"
node backend\dev-stub.mjs      # in another terminal
gramit start
gramit doctor
```

The daemon registers `Ctrl+Alt+F` itself, so `gramit status` should show the hotkey as
registered — no keybinding setup step as on Linux.

### What to check, in order

1. `gramit status` — does **typing** say `SendInput via enigo (Windows)`?
2. `gramit status` — is the **hotkey** registered?
3. `gramit fix "he go"` — the chain, without any input simulation.
4. Then the [real test](#4-the-test-that-actually-matters).

### Failure modes specific to Windows

| Symptom | Likely cause |
|---|---|
| Hotkey registers but **never fires** | The message pump isn't dispatching. `global-hotkey` creates a hidden window but pumps nothing itself; `run_loop::pump_until` runs `PeekMessageW`/`DispatchMessageW` for this. |
| `could not bind Ctrl+Alt+F` | Another application already owns that shortcut. Try `gramit config set hotkey "Ctrl+Alt+G"` then `gramit restart`. |
| Works everywhere except one app | That app is probably running elevated. A non-elevated process cannot send input to an elevated window — this is a Windows security boundary, not a gramit bug. |

The daemon talks over the named pipe `\\.\pipe\gramit` rather than a socket file; that
difference is handled internally and needs nothing from you.

---

## 8. Automated tests

```bash
cargo test --workspace      # 122 tests: core, input, daemon, CLI
cd backend && npm test      # 47 tests
```

The Rust suite includes end-to-end tests that spawn the real `gramitd` binary and talk
to it over a real socket, so a broken IPC protocol fails the build.

Tests that touch the real system clipboard are excluded by default — they need a
desktop session and mutate your clipboard:

```bash
cargo test -p gramit-input --test clipboard_live -- --ignored --nocapture
```

To check the portal path on Linux without starting the daemon:

```bash
cargo run -p gramit-input --example portal_check
```

### Cross-compile checks

You can catch platform bugs without owning the hardware, which is how the current
Windows and macOS code was validated:

```bash
rustup target add x86_64-pc-windows-msvc aarch64-apple-darwin

cargo check --workspace --target x86_64-pc-windows-msvc
cargo check -p gramit-input --target aarch64-apple-darwin
```

The full workspace cannot be checked for macOS from a non-Mac: `notify-rust` pulls in
`mac-notification-sys`, which compiles Objective-C and needs the Apple SDK. Checking
`gramit-input` covers the clipboard, hotkey and injection code, which is where the
platform risk actually is.

---

## 9. Troubleshooting

Start with `gramit doctor`. It is designed to answer this question, and every failure
it reports comes with the command that fixes it.

| Symptom | Try |
|---|---|
| `could not reach the gramit daemon` | `gramit start` |
| `NO_API_KEY` | Backend has no credentials — check `.env`, or use `node backend/dev-stub.mjs` |
| `BACKEND_UNREACHABLE` | Start the backend: `cd backend && npm start` |
| Hotkey does nothing | `gramit fix --selection` — if *that* works, the problem is the shortcut, not gramit |
| Nothing pastes, no notification | `gramit logs -f`, then press the hotkey and watch |
| Paste is flaky / lands half the time | Raise `paste_delay_ms` and `restore_delay_ms` |
| Wrong text pasted | Raise `copy_settle_ms` — the copy hadn't landed before the read |
| Clipboard not restored | `gramit logs` will show the restore failure |

Timing settings, if the defaults don't suit your machine:

```bash
gramit config set paste_delay_ms 250
gramit config set restore_delay_ms 400
gramit restart
```

### Full reset

```bash
gramit stop
rm -rf ~/.config/gramit ~/.local/share/gramit ~/.local/state/gramit   # Linux paths
gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings "[]"
```

On macOS and Windows use `gramit config path` and `gramit logs` to find the equivalent
directories before deleting anything.
