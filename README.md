# gramit

Fix what you have selected, wherever you are.

Select text in any app — an editor, a chat box, a notes window — press `Ctrl+Alt+F`
(`Ctrl+Option+F` on a Mac, which has no Alt key), and the selection is replaced in
place. What it is replaced *with* depends on the mode:

| Mode | What the hotkey does |
|---|---|
| `code` *(default)* | Writes and fixes code. A request in the selection's comments is the task |
| `grammar` | Fixes grammar, spelling and punctuation. Your wording, voice and formatting are left alone |

One mode is active at a time, because the hotkey carries no argument. `gramit start`
asks which one with an arrow-key picker:

```
What should gramit do with the text you select?
  ↑/↓ to move, Enter to choose, Esc to keep the current one

  › code     write and fix code — comments in the selection are the request
    grammar  fix grammar, spelling and punctuation — wording is left alone
```

Or switch any time with `gramit mode code` / `gramit mode grammar`, which saves the
setting and restarts the daemon so it takes effect immediately.

## Code mode

There are two ways to ask, and both come back as code and nothing else.

**Ask inside code you already have.** Write the request as a comment and select the
whole block:

```js
// sort these by date, newest first
function newest(items) {
  return items;
}
```

The block comes back sorted, with the comment gone because it has been answered.
Everything else is returned untouched — the reply overwrites the selection exactly, so
what you did not ask about does not change.

**Or just ask.** Select a line of plain text:

```
Write Java code for two sum
```

and it is replaced by a complete Java file: the imports, the class, the method. No
prose, no fences, no "here's the code" — those would land in the middle of your source
file. If you do not name a language, gramit uses the one the surrounding code is
written in.

## Grammar mode

`gramit mode grammar` turns the same hotkey into a proofreader:

```
he go to the store    →    He goes to the store.
dont worry its fine   →    Don't worry, it's fine.
```

It repairs what is broken and nothing else. It will not swap a word for a synonym,
join or split your sentences, expand a contraction, add a question tag you did not
write, or touch anything inside backticks. Text that is already correct comes back
unchanged.

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
gramit setup                   # asks which backend to send text to
gramit start
gramit doctor --fix            # binds the hotkey and reports anything broken
```

`gramit start` asks the setup question itself the first time, so running `setup`
separately is optional.

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

`gramit doctor` is the command to reach for whenever something isn't working — every
failed check prints what to do about it.

## The backend

gramit does not do the fixing itself. It sends the selection, and the current mode, to
a small HTTP service — the backend — which picks the prompt for that mode, calls a
language model, and sends the result back.

**No backend address is built into gramit.** There is no default, nothing is compiled
in, and the binaries on the releases page point at nobody. You say where to send your
code and it is written to your own config file:

```bash
gramit setup                        # asks, checks the address answers, saves it
gramit setup https://your-backend    # or say it outright
```

This is deliberate. A public repository with an address baked in would aim every
install on earth at whoever built the binaries and spend their model credits, and it
would mean everyone's text quietly flowed through one machine. Neither is something a
user should have to opt out of.

### Running your own

The backend lives in `backend/` and is a small Node service. It is the only component
that holds a model API key — the key never reaches your machine.

```bash
cd backend
npm install
cp .env.example .env           # fill in your Azure OpenAI details
npm run build && npm start     # listens on 127.0.0.1:8787

gramit setup http://127.0.0.1:8787
gramit restart
```

It deploys to anything that runs a Node server; `backend/vercel.json` is set up for
Vercel. Point `gramit setup` at the deployed URL afterwards.

For one-off use against a different backend without changing your saved config, set
`GRAMIT_BACKEND_URL` in the environment — it is read at run time and wins for that
process only.

## Usage

```bash
gramit mode                      # what does the hotkey do right now?
gramit mode grammar              # switch, and restart the daemon to apply it

gramit fix "he go to the store"  # fix in the current mode, print the result
cat snippet.py | gramit fix -    # fix stdin
gramit fix --clipboard           # fix the clipboard in place
gramit fix --selection           # capture the selection, fix it, paste it back
gramit fix "..." --mode grammar  # override the mode for this one fix
```

`gramit fix --selection` is what the hotkey runs.

```bash
gramit setup [url]            gramit start [--foreground]   gramit stop
gramit restart                gramit status                 gramit mode [name]
gramit config get [key]       gramit config set <key> <value>   gramit config path
gramit logs [-f] [-n N]       gramit doctor [--fix]
```

## Settings

`~/.config/gramit/config.toml` (platform-appropriate elsewhere). Change with
`gramit config set <key> <value>`, then `gramit restart`.

| Setting | Default | Meaning |
|---|---|---|
| `hotkey` | `Ctrl+Alt+F` | The shortcut that fixes the selection. `Alt` means the Option (⌥) key on macOS; `Option` is accepted as a spelling too |
| `backend_url` | *(none — you set it)* | Backend that does the fixing |
| `mode` | `code` | `code` or `grammar`. Prefer `gramit mode <name>`, which also applies it |
| `notifications` | `true` | Show a toast for each fix |
| `max_chars` | `16000` | Refuse selections longer than this |
| `request_timeout_ms` | `15000` | Give up on the backend after this long |
| `modifier_release_ms` | `120` | Wait for you to let go of the hotkey before typing |
| `copy_settle_ms` | `400` | How long to wait for the copy to land |
| `paste_delay_ms` | `120` | Pause before pasting the result |
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

**macOS.** The hotkey is `Ctrl+Option+F` — Mac keyboards have no key labelled Alt,
and `Ctrl+Alt+F` in the config means the same chord. Accessibility permission (System
Settings → Privacy & Security → Accessibility) is required before gramit can type,
and the daemon checks for it once at startup: **grant it, then run `gramit restart`**,
or the hotkey stays unbound for the life of that daemon. `gramit doctor` says so if
you forget.

**Windows.** Needs no permissions. Not yet run on real hardware.

## Development

See `TESTING.md` for the full local-testing guide.

```bash
cargo test --workspace        # Rust: core, input, daemon, CLI
cd backend && npm test        # backend

# work on the daemon without Azure credentials:
node backend/dev-stub.mjs     # canned replies on 127.0.0.1:8787
```

The dev stub applies a handful of fixed substitutions instead of calling a model,
which keeps the daemon's capture → rewrite → paste loop predictable enough to assert
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
git tag v0.2.0 && git push origin v0.2.0
```

It refuses a tag that disagrees with `[workspace.package] version` in `Cargo.toml`,
then builds Windows x64, a macOS universal binary, and Linux x64, and publishes them
as a GitHub Release with a `SHA256SUMS` file. The installers read that release, so
nothing else has to be updated when a version ships. A tag with a suffix
(`v0.2.0-rc1`) is published as a pre-release.

Released binaries contain no backend address — each user supplies their own with
`gramit setup`, so there is nothing deployment-specific to check before tagging.

`FLOW.md` traces exactly what happens between pressing the hotkey and the text
changing — the best place to start when something misbehaves. `TESTING.md` covers
running the backend and testing on each OS. `phase.md` has the build plan and the
reasoning behind the platform decisions.
