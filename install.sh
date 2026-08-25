#!/bin/sh
# gramit installer for macOS and Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/JoeCelaster/gramit/main/install.sh | sh
#
# Environment:
#   GRAMIT_VERSION         tag to install, e.g. v0.1.0 (default: the latest release)
#   GRAMIT_INSTALL_DIR     where the binaries go (default: $HOME/.local/bin)
#   GRAMIT_NO_MODIFY_PATH  set to anything to print the PATH line instead of writing it
#
# Flags (pipe them through with: curl ... | sh -s -- --uninstall):
#   --uninstall            remove the binaries, leaving config and logs alone
set -eu

REPO="JoeCelaster/gramit"
INSTALL_DIR="${GRAMIT_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${GRAMIT_VERSION:-latest}"

# `gramit` locates `gramitd` as a sibling before falling back to PATH
# (crates/gramit-cli/src/lifecycle.rs), so the two always travel together.

TARGET=""
OS=""
ARCHIVE=""
TMPDIR_GRAMIT=""

say()  { printf '%s\n' "$*"; }
info() { printf '  %s\n' "$*"; }
err()  { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

cleanup() {
    [ -n "$TMPDIR_GRAMIT" ] && rm -rf "$TMPDIR_GRAMIT"
    return 0
}
trap cleanup EXIT INT TERM

detect_target() {
    OS=$(uname -s)
    uname_m=$(uname -m)

    case "$OS" in
        Darwin)
            # One universal archive covers Apple Silicon and Intel.
            TARGET="universal-apple-darwin"
            ARCHIVE="gramit-${TARGET}.tar.gz"
            ;;
        Linux)
            case "$uname_m" in
                x86_64 | amd64)
                    TARGET="x86_64-unknown-linux-gnu"
                    ARCHIVE="gramit-${TARGET}.tar.gz"
                    ;;
                *)
                    err "no prebuilt Linux binary for $uname_m yet.
Build from source instead:
  git clone https://github.com/$REPO && cd gramit
  cargo build --release
  cp target/release/gramit target/release/gramitd \"$INSTALL_DIR\""
                    ;;
            esac
            ;;
        *)
            err "$OS is not supported. gramit builds for macOS, Linux and Windows;
on Windows use the PowerShell installer:
  irm https://raw.githubusercontent.com/$REPO/main/install.ps1 | iex"
            ;;
    esac
}

asset_url() {
    if [ "$VERSION" = "latest" ]; then
        printf 'https://github.com/%s/releases/latest/download/%s' "$REPO" "$1"
    else
        printf 'https://github.com/%s/releases/download/%s/%s' "$REPO" "$VERSION" "$1"
    fi
}

# Fetches $1 into $2. curl is what got this script here in the first place, but a
# wget-only box is cheap to support.
fetch() {
    if have curl; then
        curl -fsSL --proto '=https' --tlsv1.2 "$1" -o "$2" \
            || err "download failed: $1"
    elif have wget; then
        wget -q "$1" -O "$2" || err "download failed: $1"
    else
        err "neither curl nor wget is installed"
    fi
}

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        err "no sha256sum or shasum available to verify the download"
    fi
}

verify() {
    expected=$(grep " $ARCHIVE\$" "$TMPDIR_GRAMIT/SHA256SUMS" | cut -d' ' -f1 || true)
    [ -n "$expected" ] || err "$ARCHIVE is not listed in SHA256SUMS"

    actual=$(sha256_of "$TMPDIR_GRAMIT/$ARCHIVE")
    if [ "$expected" != "$actual" ]; then
        err "checksum mismatch for $ARCHIVE
  expected $expected
  got      $actual
Refusing to install. Re-run, and report this if it happens again."
    fi
    info "checksum ok"
}

# A running daemon holds the old code; replacing the file under it does nothing until
# it restarts. Stop it first so `gramit start` afterwards picks up the new build.
stop_running_daemon() {
    if [ -x "$INSTALL_DIR/gramit" ]; then
        "$INSTALL_DIR/gramit" stop >/dev/null 2>&1 || true
    fi
}

install_binaries() {
    mkdir -p "$INSTALL_DIR" || err "could not create $INSTALL_DIR"
    stop_running_daemon

    for bin in gramit gramitd; do
        src="$TMPDIR_GRAMIT/gramit-$TARGET/$bin"
        [ -f "$src" ] || err "$bin is missing from the archive"
        chmod +x "$src"
        # Unlink first: moving onto a file that is currently executing fails with
        # ETXTBSY when the temp dir is on a different filesystem than $INSTALL_DIR.
        rm -f "$INSTALL_DIR/$bin"
        mv "$src" "$INSTALL_DIR/$bin" || err "could not write $INSTALL_DIR/$bin"
        info "installed $INSTALL_DIR/$bin"
    done

    if [ "$OS" = "Darwin" ]; then
        # A curl download carries no quarantine attribute, but a re-run over a copy
        # that came from a browser might. Harmless when there is nothing to remove.
        for bin in gramit gramitd; do
            xattr -d com.apple.quarantine "$INSTALL_DIR/$bin" >/dev/null 2>&1 || true
        done
    fi
}

# Which rc file a login shell will actually read.
rc_file() {
    case "$(basename "${SHELL:-/bin/sh}")" in
        zsh)  printf '%s/.zshrc' "$HOME" ;;
        fish) printf '%s/.config/fish/config.fish' "$HOME" ;;
        bash)
            if [ "$OS" = "Darwin" ] && [ -f "$HOME/.bash_profile" ]; then
                printf '%s/.bash_profile' "$HOME"
            else
                printf '%s/.bashrc' "$HOME"
            fi
            ;;
        *) printf '%s/.profile' "$HOME" ;;
    esac
}

path_line() {
    if [ "$(basename "${SHELL:-/bin/sh}")" = "fish" ]; then
        printf 'fish_add_path %s' "$INSTALL_DIR"
    else
        printf 'export PATH="%s:$PATH"' "$INSTALL_DIR"
    fi
}

ensure_path() {
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) return 0 ;;
    esac

    line=$(path_line)

    if [ -n "${GRAMIT_NO_MODIFY_PATH:-}" ]; then
        say ""
        say "$INSTALL_DIR is not on your PATH. Add it yourself with:"
        info "$line"
        return 0
    fi

    rc=$(rc_file)
    mkdir -p "$(dirname "$rc")"
    # The marker keeps a re-run from stacking duplicate exports.
    if [ -f "$rc" ] && grep -q '# added by gramit installer' "$rc"; then
        info "PATH entry already present in $rc"
    else
        {
            printf '\n# added by gramit installer\n'
            printf '%s\n' "$line"
        } >> "$rc"
        info "added $INSTALL_DIR to PATH in $rc"
    fi
}

next_steps() {
    say ""
    say "gramit is installed. Next:"
    say ""
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *) info "open a new terminal (or source your shell rc) so PATH picks up $INSTALL_DIR" ;;
    esac
    info "gramit setup             # tell gramit which backend to send text to"
    info "gramit start"

    if [ "$OS" = "Darwin" ]; then
        info "grant Accessibility: System Settings > Privacy & Security > Accessibility"
        info "gramit doctor"
        say ""
        say "Note: this build is not Developer ID signed, so macOS ties the Accessibility"
        say "grant to this exact binary. You will have to grant it again after an upgrade."
    else
        info "gramit doctor --fix        # binds Ctrl+Alt+F and reports anything broken"
    fi

    say ""
    say "Then select text anywhere and press Ctrl+Alt+F."
    say ""
    say "gramit ships with no backend address: you choose where your text is sent,"
    say "and it is saved only in your own config. See the README if you need to run one."
}

uninstall() {
    say "Removing gramit..."
    if [ -x "$INSTALL_DIR/gramit" ]; then
        "$INSTALL_DIR/gramit" stop >/dev/null 2>&1 || true
    fi

    removed=0
    for bin in gramit gramitd; do
        if [ -e "$INSTALL_DIR/$bin" ]; then
            rm -f "$INSTALL_DIR/$bin"
            info "removed $INSTALL_DIR/$bin"
            removed=1
        fi
    done
    [ "$removed" = 1 ] || info "nothing found in $INSTALL_DIR"

    say ""
    say "Left in place on purpose:"
    info "config  ~/.config/gramit/config.toml (or the macOS equivalent)"
    info "logs    the directory 'gramit logs' was reading"
    if [ "$(uname -s)" = "Linux" ]; then
        say ""
        say "If you bound the GNOME hotkey, clear it with:"
        info "gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings \"[]\""
    fi
}

main() {
    if [ "${1:-}" = "--uninstall" ]; then
        uninstall
        return 0
    fi

    detect_target

    say "Installing gramit ($VERSION) for $TARGET"

    TMPDIR_GRAMIT=$(mktemp -d 2>/dev/null || mktemp -d -t gramit)

    fetch "$(asset_url "$ARCHIVE")" "$TMPDIR_GRAMIT/$ARCHIVE"
    fetch "$(asset_url SHA256SUMS)" "$TMPDIR_GRAMIT/SHA256SUMS"
    verify

    tar -xzf "$TMPDIR_GRAMIT/$ARCHIVE" -C "$TMPDIR_GRAMIT" \
        || err "could not unpack $ARCHIVE"

    install_binaries
    ensure_path

    version=$("$INSTALL_DIR/gramit" --version 2>/dev/null || true)
    [ -n "$version" ] || err "$INSTALL_DIR/gramit did not run after install"
    info "$version"

    next_steps
}

main "${1:-}"
