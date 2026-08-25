# What happens when you press Ctrl+Alt+F

A trace of one grammar fix, from the keypress to the replaced text, with the file that
does each part. This is the Linux/GNOME path; where Windows and macOS differ is noted
at the end.

## The short version

```
you: Ctrl+A            (select text — gramit never sees this)
you: Ctrl+Alt+F
      │
      ▼
GNOME keybinding ──runs──> gramit fix --selection      (a new short-lived process)
      │
      ▼  unix socket, one line of JSON
   gramitd  (the long-lived daemon)
      │
      ├─ 1. save your clipboard
      ├─ 2. clear it
      ├─ 3. release modifiers, inject Ctrl+C   ──> your app copies
      ├─ 4. poll the clipboard  ──(retry 3 if empty)
      │
      ├─ 5. POST /v1/fix ──> backend ──> Azure OpenAI ──> corrected text
      │
      ├─ 6. put the correction on the clipboard
      ├─ 7. inject Ctrl+V                      ──> your app pastes
      ├─ 8. restore your clipboard
      └─ 9. desktop notification
```

The single most important thing to understand: **gramit cannot see your selection.**
It only knows whether a copy landed on the clipboard. Every "Nothing selected" message
really means "the copy I asked for never arrived".

---

## Step by step

### 0. `Ctrl+A` — nothing to do with gramit

Your app handles this entirely. gramit is not involved and has no way to observe it.
Wayland gives applications no access to another app's selection.

### 1. `Ctrl+Alt+F` reaches GNOME, not gramit

The shortcut is a **GNOME custom keybinding** in dconf, pointing at an absolute path:

```
/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/gramit/
  binding = '<Control><Alt>f'
  command = '/path/to/gramit fix --selection'
```

Installed by `gramit doctor --fix` (`crates/gramit-cli/src/doctor.rs`).

Why a keybinding rather than the app grabbing the key itself: the XDG
`GlobalShortcuts` portal rejects any app without a sandbox app id
(`NotAllowed: An app id is required`), which rules out an ordinary installed binary.
The daemon still *attempts* the portal on startup and falls back to this.

**GNOME fires on key _press_** — this matters in step 3.

### 2. A one-shot CLI process talks to the daemon

`gramit fix --selection` starts, sends one line of JSON over a unix socket, waits for
one line back, prints, exits.

- `crates/gramit-cli/src/fix.rs` → `fix_selection()` sends `Request::FixSelection`
- `crates/gramit-cli/src/client.rs` → connects to `$XDG_RUNTIME_DIR/gramit-<user>.sock`
- `crates/gramitd/src/server.rs` → `handle_connection` reads the line
- `crates/gramitd/src/handler.rs:20` → routes `FixSelection` to `selection::fix_selection`

The CLI is deliberately dumb. It holds no state and does no work — which is why it can
be spawned fresh on every keypress without cost.

### 3. The daemon runs the fix loop

`crates/gramitd/src/selection.rs` → `run()`:

- takes `fix_gate`, a mutex that **refuses a second fix while one is running** — two
  concurrent fixes would fight over one clipboard and paste into whichever window
  happened to be focused
- calls the loop below
- notifies you of the result, whatever it is

`crates/gramitd/src/fixloop.rs` → `run()` → `capture_and_replace()`:

**3a. Save the clipboard** (`clipboard::snapshot`). Restored in step 8 no matter how
the fix ends.

**3b. Wait `modifier_release_ms` (250 ms).** You are almost certainly still holding
`Ctrl+Alt`, because GNOME fired on key press. Anything injected now arrives as
`Ctrl+Alt+C`, which copies nothing.

**3c. Clear the clipboard.** This turns "did the copy work?" into the much simpler "is
there anything here?". Comparing against the previous contents instead would mistake a
selection identical to your clipboard for a failed copy.

**3d. Release every modifier, then inject `Ctrl+C`.**
`crates/gramit-input/src/linux/inject.rs` → `ctrl_chord()` first sends a *release* for
all eight modifier keysyms (both Shift/Ctrl/Alt/Super), then presses Ctrl, taps `c`,
releases Ctrl — releasing Ctrl even if the tap failed, since a stuck modifier would
wreck every subsequent keystroke you type.

The keystrokes go through the **RemoteDesktop portal** over D-Bus. That is the
Wayland-sanctioned way to synthesise input: no root, no `ydotool`, no `/dev/uinput`.
You approve it once; a restore token in `~/.local/share/gramit/remote-desktop.token`
keeps it silent afterwards. If the session has died, `chord()` detects it and
re-establishes rather than failing until the next restart.

**3e. Poll the clipboard, and retry the copy.** `capture()` watches for
`copy_retry_interval_ms` (200 ms), and if nothing appeared, **injects `Ctrl+C` again** —
repeating until `copy_settle_ms` (1500 ms) is spent, so roughly 7 attempts.

This retry is what makes the hotkey reliable. Your fingers come off the keys somewhere
in the first few hundred milliseconds; one of the later attempts lands. A copy that
works first time costs nothing extra, because the loop returns the moment text appears.

**If nothing is captured**, the daemon logs one diagnostic line:

```
no text captured attempts=7 elapsed_ms=1503 primary_has_text=true
```

`primary_has_text` is the tell. PRIMARY is the X11 selection your app publishes just by
you highlighting text:

- `true` → you clearly had a selection, so the injected `Ctrl+C` is not reaching the app
- `false` → nothing was selected, or the app publishes no selection we can see

### 4. The correction round trip

`crates/gramit-core/src/client.rs` → `POST <backend_url>/v1/fix`
with `{"text": "...", "mode": "grammar"}`.

`backend_url` comes from the user's `config.toml`, put there by `gramit setup`. There
is no default and nothing is compiled in: `Config::backend_url()` returns `None` until
the user names an address, and every path that needs one then fails with `NO_BACKEND`
rather than reaching a stranger's machine. `GRAMIT_BACKEND_URL` overrides the saved
value at run time for a single process.

In the backend (`backend/src/`):

1. `app.ts` → `routes/fix.ts` validates with zod (25k cap)
2. `service.ts` → checks an LRU cache keyed by `sha256(mode + text)`; a repeat fix
   returns instantly and costs nothing
3. `llm/azure.ts` → calls Azure OpenAI. Because a custom deployment's accepted
   parameters can't be known ahead of time, it walks a ladder of request shapes
   (`json+temperature` → `json` → `plain text`) on a 400 and remembers the first that
   works. `gpt-5.6-luna` rejects `temperature: 0`, so it settles on the second shape.
4. `prompt.ts` → the system prompt fixes grammar/spelling/punctuation only, preserves
   formatting, and treats your text as **data, never instructions** — a selection
   reading "ignore previous instructions" gets corrected, not obeyed
5. `prompt.ts` → `sanitizeCorrection()` unwraps JSON, code fences, "Here's the
   corrected text:" preambles and added quotes, then restores your original leading and
   trailing whitespace
6. `diff.ts` → word-level change count, which becomes "Fixed 3 issues"

Response: `{corrected, changed, changes, model, latency_ms, cached}`.

### 5. Paste, or don't

**If the text came back unchanged**, the daemon stops here and reports *Looks good
already*. Pasting identical text would still cost you an undo step and move the caret.

Otherwise:

1. put the correction on the clipboard
2. wait `paste_delay_ms` (120 ms) so the compositor publishes it
3. inject `Ctrl+V` — same chord machinery as the copy
4. wait `restore_delay_ms` (200 ms), because the paste is asynchronous: your app
   requests the clipboard *after* receiving the keystroke, so restoring immediately
   would hand it the old text

### 6. Put your clipboard back

`fixloop::run()` restores the step-3a snapshot on **every** path — success, failure,
nothing selected, backend down. Whatever you had copied before is still there.

Known limitation: non-text clipboard content (an image, files) reads as empty and is
therefore cleared rather than restored.

### 7. Tell you what happened

`crates/gramitd/src/notify.rs` turns the outcome into a desktop notification:

| Outcome | Notification | Urgency |
|---|---|---|
| Replaced | *Fixed 3 issues* | Low |
| Unchanged | *Looks good already* | Low |
| Nothing captured | *Nothing selected* | Normal |
| Failure | *gramit backend is not running* (etc.) + detail | Critical |

Every outcome notifies, including the boring ones. A hotkey press has no terminal, so
silence would be indistinguishable from a crashed daemon.

Error codes survive the whole chain — a backend `NO_API_KEY` arrives at the notification
as `NO_API_KEY`, not a generic HTTP 503, which is what lets the toast say something
actionable.

---

## Timing budget

| Phase | Setting | Default |
|---|---|---|
| Wait for you to release the hotkey | `modifier_release_ms` | 250 ms |
| Watch the clipboard before retrying the copy | `copy_retry_interval_ms` | 200 ms |
| Total budget for landing the copy | `copy_settle_ms` | 1500 ms |
| Clipboard published before pasting | `paste_delay_ms` | 120 ms |
| Paste consumed before restoring | `restore_delay_ms` | 200 ms |

A successful fix pays only `modifier_release_ms` + one poll + the model round trip.
The rest is spent only when something is going wrong.

Tune with `gramit config set <key> <value> && gramit restart`.

---

## Why the daemon exists

The CLI could in principle do all of this itself. It cannot, for three reasons:

1. **The clipboard needs an owner.** On X11 the process that sets the clipboard must
   stay alive to serve it. A CLI that exited immediately would take your corrected text
   with it. This is also why `gramit fix --clipboard` is an IPC request rather than
   local work.
2. **The portal session is expensive.** Establishing RemoteDesktop access costs a D-Bus
   round trip and, the first time, a consent dialog. Doing that per keypress would be
   unusable.
3. **Single-flight.** Something must refuse a second fix while one is in progress.

## Where the other platforms differ

Only steps 1 and 3d change:

| | Hotkey | Injection |
|---|---|---|
| **Linux** | GNOME keybinding → `gramit fix --selection` | RemoteDesktop portal (D-Bus) |
| **Windows** | daemon registers it (`global-hotkey`), main thread pumps the message loop | `SendInput` via `enigo` |
| **macOS** | daemon registers it (Carbon), main thread pumps the run loop | `CGEvent` via `enigo`, needs Accessibility permission |

On Windows and macOS the hotkey fires **inside** the daemon, so there is no CLI process
in the path — `crates/gramitd/src/hotkey_loop.rs` calls `selection::run` directly.
Everything from step 3a onward is identical, and macOS sends `Cmd` instead of `Ctrl`.

Windows and macOS are compile-verified but have not been run on real hardware; see
`TESTING.md`.
