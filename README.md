# gramit

Fix your grammar from where you stay.

Select text in any app — Chrome, Slack, a text editor — press `Ctrl+Alt+F`, and the
selection is replaced in place with a corrected version. Grammar, spelling and
punctuation only: your wording, voice and formatting are left alone.

```
gramit (CLI) ──local socket──> gramitd (daemon) ──HTTP──> backend ──> Azure OpenAI
                                  │
                                  ├─ global hotkey
                                  ├─ clipboard      (save → capture → restore)
                                  ├─ key injection  (Ctrl+C / Ctrl+V)
                                  └─ desktop notifications
```

The daemon does the work; the CLI just talks to it. The backend is the only component
that holds an API key.

## Quick start

```bash
# 1. build
cargo build --release          # produces target/release/gramit and gramitd

# 2. configure the backend
cd backend
npm install
cp .env.example .env           # fill in your Azure OpenAI details
npm run build && npm start     # listens on 127.0.0.1:8787

# 3. start gramit and check the setup
gramit start
gramit doctor --fix            # binds the hotkey and reports anything broken
```

`gramit doctor` is the command to reach for whenever something isn't working — every
failed check prints what to do about it.

## Usage

```bash
gramit fix "he go to the store"   # correct text, print the result
echo "he go" | gramit fix -       # correct stdin
gramit fix --clipboard            # correct the clipboard in place
gramit fix --selection            # capture the selection, correct it, paste it back
```

`gramit fix --selection` is what the hotkey runs.

```bash
gramit start [--foreground]   gramit stop   gramit restart   gramit status
gramit config get [key]       gramit config set <key> <value>   gramit config path
gramit logs [-f] [-n N]       gramit doctor [--fix]
```

## Settings

`~/.config/gramit/config.toml` (platform-appropriate elsewhere). Change with
`gramit config set <key> <value>`, then `gramit restart`.

| Setting | Default | Meaning |
|---|---|---|
| `hotkey` | `Ctrl+Alt+F` | The shortcut that fixes the selection |
| `backend_url` | `http://127.0.0.1:8787` | Where the backend listens |
| `mode` | `grammar` | Correction style |
| `notifications` | `true` | Show a toast for each fix |
| `max_chars` | `8000` | Refuse selections longer than this |
| `request_timeout_ms` | `10000` | Give up on the backend after this long |
| `modifier_release_ms` | `120` | Wait for you to let go of the hotkey before typing |
| `copy_settle_ms` | `400` | How long to wait for the copy to land |
| `paste_delay_ms` | `120` | Pause before pasting the correction |
| `restore_delay_ms` | `200` | Pause before restoring your clipboard |

If the paste is unreliable on a slower machine, raise `paste_delay_ms` and
`restore_delay_ms` first.

## Platform notes

**Linux (GNOME / Wayland).** Two separate mechanisms, and only one is a portal:

- *Typing* uses the **RemoteDesktop portal**. You approve it once; a saved restore
  token keeps it silent afterwards. No root, no `ydotool`, no `/dev/uinput`.
- *The hotkey* uses a **GNOME custom keybinding**, installed by `gramit doctor --fix`.
  The GlobalShortcuts portal refuses apps without a sandbox app id, so it cannot bind
  a shortcut for an ordinary installed binary.

If typing stops working, the portal session is usually stuck:

```bash
systemctl --user restart xdg-desktop-portal-gnome xdg-desktop-portal && gramit restart
```

**macOS.** Needs Accessibility permission (System Settings → Privacy & Security →
Accessibility) before it can type. Not yet run on real hardware.

**Windows.** Needs no permissions. Not yet run on real hardware.

## Development

See `TESTING.md` for the full local-testing guide.

```bash
cargo test --workspace        # Rust: core, input, daemon, CLI
cd backend && npm test        # backend

# work on the daemon without Azure credentials:
node backend/dev-stub.mjs     # canned corrections on 127.0.0.1:8787
```

The dev stub applies a handful of fixed substitutions instead of calling a model,
which keeps the daemon's capture → correct → paste loop predictable enough to assert
on.

```
crates/gramit-core/    config, IPC protocol, backend client
crates/gramit-input/   clipboard, hotkeys, keystroke injection (per-platform)
crates/gramitd/        the daemon
crates/gramit-cli/     the `gramit` command
backend/               Node + Express + Azure OpenAI
```

`FLOW.md` traces exactly what happens between pressing the hotkey and the text
changing — the best place to start when something misbehaves. `TESTING.md` covers
running the backend and testing on each OS. `phase.md` has the build plan and the
reasoning behind the platform decisions.
