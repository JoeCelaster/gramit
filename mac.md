# The gramit shortcut on macOS

Where the keyboard shortcut is stored, how it is spelled, and how to check what a
user's machine is actually doing with it.

**The one thing to know up front:** the shortcut is *not* a macOS system shortcut. It
does not appear in System Settings → Keyboard → Keyboard Shortcuts, and looking for it
there will always come up empty. It lives in gramit's own config file, and the running
daemon claims it from the OS at startup via Carbon's `RegisterEventHotKey`. So there
are two separate things to verify — what is *configured*, and what the daemon actually
*bound* — and they can disagree.

## Quick reference

| Question | Command |
|---|---|
| Where is the config file? | `gramit config path` |
| What shortcut is configured? | `gramit config get hotkey` |
| What did the daemon actually bind? | `gramit status` |
| Why isn't it working? | `gramit doctor` |
| Is the key press even arriving? | press it, then `gramit logs` |
| Change it | `gramit config set hotkey "Ctrl+Option+G"` then `gramit restart` |

## Where the setting lives

```
~/Library/Application Support/gramit/config.toml
```

Ask the machine rather than assuming — `gramit config path` prints the real answer, and
it is the only reliable one:

```console
$ gramit config path
/Users/<you>/Library/Application Support/gramit/config.toml
```

The path comes from `paths::config_path()` (`crates/gramit-core/src/paths.rs`), which
checks the `GRAMIT_CONFIG` environment variable first and otherwise asks the
`directories` crate for the platform config dir. On macOS that is
`~/Library/Application Support/gramit`, which is also where `gramitd.log` lands — config
and logs share one directory here, unlike on Linux.

**The file often does not exist.** `Config::load_from` treats a missing file as "use the
defaults", so a fresh install runs entirely on built-in values and writes nothing until
something calls `save`. `gramit config path` still prints where the file *would* go, and
`gramit config get` still answers:

```console
$ gramit config get hotkey     # with no config.toml on disk at all
Ctrl+Alt+F
```

So "there's no config file" is normal, not a fault.

### What the file looks like

A plain TOML table — the shortcut is one ordinary string field, `hotkey`:

```toml
hotkey = "Ctrl+Alt+F"
backend_url = ""
mode = "grammar"
notifications = true
max_chars = 8000
request_timeout_ms = 15000
capture = "copy"
modifier_release_ms = 250
copy_settle_ms = 1500
copy_retry_interval_ms = 200
paste_delay_ms = 120
restore_delay_ms = 200
```

Two properties of the writer worth knowing before you hand-edit it:

- **`gramit config set` rewrites the whole file** from the parsed struct
  (`toml::to_string_pretty`), so hand-written comments and field ordering are lost on the
  next `set`. Hand-editing is supported; just expect that.
- **Unknown fields are a hard error, not a warning.** The struct is
  `#[serde(deny_unknown_fields)]` on purpose, so a typo like `hotkeys = "..."` fails at
  load with a real message instead of being silently ignored — and the daemon will refuse
  to start rather than run with a setting you thought you had changed.

## How the shortcut is spelled

The stored value is **canonical and platform-neutral**: `Ctrl+Alt+F` on every OS,
including macOS. What a Mac user *reads* is rendered separately.

Mac keyboards have no key labelled Alt — the key is ⌥ Option — so
`hotkey_spec::display()` renders the stored `Ctrl+Alt+F` as **`Ctrl+Option+F`** in
`gramit status`, `gramit doctor`, and anywhere else a person sees it. `gramit config get`
deliberately does *not* do this: it reports the raw stored value, because that is what
you would type back in.

```console
$ gramit config get hotkey
Ctrl+Alt+F              # canonical, what is on disk

$ gramit status
  hotkey  Ctrl+Option+F # rendered for a Mac reader — same chord
```

Both name the same physical keys: **Control + Option + F**.

### Spellings the parser accepts

You can write it either way. `hotkey_spec::parse` (`crates/gramit-input/src/hotkey_spec.rs`)
is case-insensitive, tolerates spaces around `+`, and accepts these aliases:

| Modifier | Accepted spellings |
|---|---|
| Control | `Ctrl`, `Control` |
| Option / Alt | `Alt`, `Option` |
| Shift | `Shift` |
| Command | `Cmd`, `Command`, `Super`, `Win`, `Meta` |

So `gramit config set hotkey "Ctrl+Option+G"` is valid and stores the string
`Ctrl+Option+G` verbatim — the parser maps `Option` to the same modifier as `Alt`. Keys
are normalised internally (letters lowercased, `f5` → `F5`, `esc` → `Escape`), but the
config keeps whatever you typed.

A hotkey must have at least one modifier and exactly one non-modifier key. A bare key is
refused, because it would fire while you type.

## Changing it

```console
$ gramit config set hotkey "Ctrl+Option+G"
✓ hotkey = Ctrl+Option+G
    saved to /Users/<you>/Library/Application Support/gramit/config.toml
    restart the daemon to apply: gramit restart

$ gramit restart
```

**The restart is required.** The config is read once, at daemon startup; the shortcut is
claimed from the OS at that moment. Editing the file changes nothing about the running
daemon.

### The validation gap to watch for

`config set` checks the *type* and a few range rules — it does **not** check that the
hotkey can actually be parsed or bound. This is accepted without complaint:

```console
$ gramit config set hotkey "banana"
✓ hotkey = banana
    saved to ...
```

The problem only appears when the daemon next starts, which is why a machine can look
correctly configured and still have no working shortcut. That is exactly the case the
next section is for.

## Verifying on a user's machine

Three layers, in order. Each one answers a different question, and the first that fails
is your answer.

### Layer 1 — what is configured

```console
$ gramit config path
$ gramit config get hotkey
```

Or read the file directly. This tells you what *should* happen. It says nothing about
what is happening.

### Layer 2 — what the daemon actually bound

```console
$ gramit status
  daemon  0.1.2 (pid 37903)
  uptime  4m 12s
  hotkey  Ctrl+Option+F
  typing  CGEvent via enigo (macOS)
  backend  https://your-backend.vercel.app
    gpt-5.6-terra
  fixes   3
```

The `hotkey` line has two forms, and the difference is the whole point of this check:

| What you see | What it means |
|---|---|
| `Ctrl+Option+F` | The daemon holds the OS registration. |
| `Ctrl+Option+F (not bound by the daemon)` | Configured, but **not registered**. The chord does nothing. |

If it says "not bound", `gramit doctor` gives the reason and a remedy:

```console
$ gramit doctor
✗ typing
    no keystroke injection.
    grant Accessibility in System Settings → Privacy & Security → Accessibility,
    then run: gramit restart
```

The most common macOS cause by far: Accessibility has not been granted, so the daemon
has no way to capture a selection and does not bind a shortcut it could not service.
Granting the permission is not enough on its own — **the daemon must be restarted
afterwards**, because it checks once at startup.

Because the builds are only ad-hoc signed, macOS ties that grant to the *exact binary*.
Upgrading or rebuilding gramit invalidates it, and the machine silently returns to the
"not bound" state until it is granted again.

For a hotkey that cannot be parsed, the log carries the real reason:

```
WARN gramitd: could not register the global hotkey ...
     code="HOTKEY_ERROR" err=hotkey: "banana" has no modifier; a bare key would fire while typing
```

### Layer 3 — is the key press arriving?

Layers 1 and 2 can both look perfect while nothing happens, because a successful
registration only means the chord was *reserved* with the WindowServer. It says nothing
about whether presses are delivered.

Select some text, press the shortcut, then:

```console
$ gramit logs | tail -20
```

`hotkey_loop` logs one line for every delivered press:

```
INFO gramitd::hotkey_loop: hotkey pressed id=fix-selection
```

This single line splits the two failures that look identical from the outside:

| In the log | Meaning |
|---|---|
| `hotkey pressed`, then `fixed the selection` | Working end to end. |
| `hotkey pressed`, then `nothing was selected` | The press arrived; capture or injection failed. |
| No `hotkey pressed` at all | The press never reached gramit. |

That last row was a real bug through 0.1.1: the daemon pumped the CFRunLoop, which does
not drain the Carbon event queue, so every press sat unread. See
`crates/gramit-input/src/run_loop.rs`.

Also check the startup lines. A healthy daemon logs, in order:

```
INFO gramitd: gramitd starting version="..."
INFO gramitd: selection machinery ready injector=CGEvent via enigo (macOS)
INFO gramit_input::native::hotkey: global hotkey registered hotkey="Ctrl+Alt+F"
INFO gramitd::hotkey_loop: hotkey loop started hotkey=Ctrl+Alt+F
```

If `skipping hotkey registration: there is no way to capture a selection` appears
instead, that is the Accessibility case from Layer 2.

## When configured and actual disagree

| Symptom | Cause |
|---|---|
| Config changed, behaviour did not | No `gramit restart`. The config is read once at startup. |
| `status` shows a different shortcut than the file | The daemon is older than the edit — restart it. |
| Correct everywhere, still nothing happens | Check Layer 3. If `hotkey pressed` never appears, the press is not reaching gramit. |
| Worked yesterday, not today | Accessibility was revoked by an upgrade — the grant is tied to the exact binary. Re-grant, then restart. |
| The file the CLI reads isn't the one the daemon read | `GRAMIT_CONFIG` was set in one shell and not the other. Compare `gramit config path` against the daemon's environment. |
| Another app owns the chord | Registration fails with "another application may already own it". Pick a different combination. |

A note on the last two rows: `GRAMIT_CONFIG`, `GRAMIT_SOCKET`, and `GRAMIT_LOG` all
override their default paths. If the daemon was started from a shell with one of these
set and you are inspecting from a shell without it, every command above will describe a
different install than the one that is running.

## Related

- `README.md` — install, permissions, the settings table
- `TESTING.md` — the first-run checklist for macOS and its failure modes
- `crates/gramit-core/src/config.rs` — the config struct, defaults, and validation
- `crates/gramit-input/src/hotkey_spec.rs` — parsing, the accepted aliases, and `display()`
