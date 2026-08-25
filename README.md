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

## Install

**Windows** (PowerShell, no admin needed)

```powershell
irm https://raw.githubusercontent.com/JoeCelaster/gramit/main/install.ps1 | iex
```

**macOS and Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/JoeCelaster/gramit/main/install.sh | sh
```

Both scripts verify the download against `SHA256SUMS`, drop `gramit` and `gramitd`
into a per-user directory (`~/.local/bin`, or `%LOCALAPPDATA%\Programs\gramit` on
Windows), and put it on your PATH. Nothing is installed system-wide and no password is
asked for. macOS gets one universal binary that runs on Apple Silicon and Intel.

Then, in a new terminal:

```bash
gramit start
gramit doctor --fix            # binds the hotkey and reports anything broken
```

On macOS, grant Accessibility in System Settings → Privacy & Security before the first
fix. These builds are not Developer ID signed, so macOS ties that grant to the exact
binary and will ask again after an upgrade.

To pin a version or change where it lands, set `GRAMIT_VERSION` / `GRAMIT_INSTALL_DIR`
(`$env:GRAMIT_VERSION` / `$env:GRAMIT_INSTALL_DIR` on Windows) before running the
script. `GRAMIT_NO_MODIFY_PATH` leaves your shell config alone.

Prebuilt archives for every release are also on the
[releases page](https://github.com/JoeCelaster/gramit/releases) if you would rather
unpack them yourself. Keep `gramit` and `gramitd` in the same directory — the CLI looks
for the daemon beside itself.

### Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/JoeCelaster/gramit/main/install.sh | sh -s -- --uninstall
```
```powershell
&([scriptblock]::Create((irm https://raw.githubusercontent.com/JoeCelaster/gramit/main/install.ps1))) -Uninstall
```

Both leave your config and logs in place.

### Build from source instead

```bash
cargo build --release          # produces target/release/gramit and gramitd
```

Corrections go to the hosted backend named by `backend.url` in the workspace
`deploy.toml`, so there is nothing else to install and no API key on your machine.
That file is the single source for the address: `gramit-core` reads it at build time,
and no URL is hardcoded in the Rust sources. To run the backend yourself instead:

```bash
cd backend
npm install
cp .env.example .env           # fill in your Azure OpenAI details
npm run build && npm start     # listens on 127.0.0.1:8787
gramit config set backend_url http://127.0.0.1:8787
gramit restart
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
| `backend_url` | `backend.url` from `deploy.toml` | Backend that corrects the text |
| `mode` | `grammar` | Correction style |
| `notifications` | `true` | Show a toast for each fix |
| `max_chars` | `8000` | Refuse selections longer than this |
| `request_timeout_ms` | `15000` | Give up on the backend after this long |
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

### Releasing

Tag and push; `.github/workflows/release.yml` does the rest.

```bash
git tag v0.1.0 && git push origin v0.1.0
```

It refuses a tag that disagrees with `[workspace.package] version` in `Cargo.toml`,
then builds Windows x64, a macOS universal binary, and Linux x64, and publishes them
as a GitHub Release with a `SHA256SUMS` file. The installers read that release, so
nothing else has to be updated when a version ships. A tag with a suffix
(`v0.2.0-rc1`) is published as a pre-release.

The backend address baked into those binaries comes from `deploy.toml` as it stands at
the tag, so check it before tagging.

`FLOW.md` traces exactly what happens between pressing the hotkey and the text
changing — the best place to start when something misbehaves. `TESTING.md` covers
running the backend and testing on each OS. `phase.md` has the build plan and the
reasoning behind the platform decisions.
