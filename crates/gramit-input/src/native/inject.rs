//! Keystroke injection on Windows and macOS via `enigo`.
//!
//! Windows needs no permission. macOS requires Accessibility (TCC) — the first
//! injection triggers the system prompt, and until it is granted `CGEvent` posts are
//! silently swallowed, which is why `probe()` exists.

use async_trait::async_trait;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use tokio::sync::{mpsc, oneshot};
use tracing::debug;

use crate::injector::Injector;
use crate::InputError;

/// Copy and paste use Cmd on macOS and Ctrl everywhere else.
#[cfg(target_os = "macos")]
const MODIFIER: Key = Key::Meta;
#[cfg(not(target_os = "macos"))]
const MODIFIER: Key = Key::Control;

enum Command {
    Chord(char, oneshot::Sender<Result<(), InputError>>),
}

/// `enigo::Enigo` is blocking and not `Sync`, so it lives on its own thread — the same
/// shape as the clipboard.
pub struct NativeInjector {
    commands: mpsc::Sender<Command>,
}

impl NativeInjector {
    pub fn new() -> Result<Self, InputError> {
        let (commands, mut rx) = mpsc::channel::<Command>(8);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        std::thread::Builder::new()
            .name("gramit-injector".to_string())
            .spawn(move || {
                let mut enigo = match Enigo::new(&Settings::default()) {
                    Ok(enigo) => {
                        let _ = ready_tx.send(Ok(()));
                        enigo
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err.to_string()));
                        return;
                    }
                };

                while let Some(Command::Chord(key, reply)) = rx.blocking_recv() {
                    let _ = reply.send(send_chord(&mut enigo, key));
                }
                debug!("injector thread stopping");
            })
            .map_err(|err| {
                InputError::Injection(format!("could not start the injector thread: {err}"))
            })?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { commands }),
            Ok(Err(err)) => Err(InputError::Injection(format!(
                "could not open the keyboard: {err}{}",
                accessibility_hint()
            ))),
            Err(_) => Err(InputError::Injection("the injector thread died on startup".into())),
        }
    }

    async fn chord(&self, key: char) -> Result<(), InputError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.commands
            .send(Command::Chord(key, reply_tx))
            .await
            .map_err(|_| InputError::Injection("the injector thread is gone".into()))?;
        reply_rx
            .await
            .map_err(|_| InputError::Injection("the injector thread dropped the request".into()))?
    }
}

/// Modifiers the user could still be leaning on when a fix starts — both sides of the
/// keyboard, since `Key::Alt` and `Key::Meta` name only the left-hand keycodes.
///
/// Windows is deliberately left alone. `RegisterHotKey` has the same fire-on-press
/// hazard, but that platform has never been run on real hardware, and the fix loop's
/// retry window already covers it; changing its injected event sequence belongs in a
/// change someone can actually test.
#[cfg(target_os = "macos")]
const HELD_MODIFIERS: &[Key] = &[
    Key::Shift,
    Key::RShift,
    Key::Control,
    Key::RControl,
    Key::Alt,
    Key::ROption,
    Key::Meta,
    Key::RCommand,
];

#[cfg(not(target_os = "macos"))]
const HELD_MODIFIERS: &[Key] = &[];

/// Lets go of whatever the user is still holding, before we type anything.
///
/// The hotkey that triggered this fix is very likely still down: Carbon fires a hot key
/// on key *press*, and a `Ctrl+Option+F` chord is comfortably held for several hundred
/// milliseconds. Without this the `Cmd+C` we inject arrives as `Ctrl+Option+Cmd+C`,
/// which copies nothing and looks exactly like an empty selection. Linux does the same
/// thing for the same reason — see `release_modifiers` in `linux/inject.rs`.
///
/// This does not replace the retry window in `fixloop::capture`: that also covers a
/// user still leaning on the key, a slow clipboard, and an app that ignores the first
/// synthetic event. This just means the first attempt usually lands.
///
/// Releasing a key that was never held is a no-op, so this is best-effort: a failure
/// here is not worth abandoning the fix over, and the chord that follows is what
/// actually has to land.
fn release_held_modifiers(enigo: &mut Enigo) {
    for key in HELD_MODIFIERS {
        if let Err(err) = enigo.key(*key, Direction::Release) {
            debug!(%err, ?key, "could not release a modifier before typing");
        }
    }
}

fn send_chord(enigo: &mut Enigo, key: char) -> Result<(), InputError> {
    let press = |enigo: &mut Enigo, k: Key, d: Direction| {
        enigo.key(k, d).map_err(|err| InputError::Injection(err.to_string()))
    };

    release_held_modifiers(enigo);

    press(enigo, MODIFIER, Direction::Press)?;
    let tapped = press(enigo, Key::Unicode(key), Direction::Click);
    // Release the modifier even if the key failed: a stuck Ctrl or Cmd would wreck
    // every keystroke the user types afterwards.
    let released = press(enigo, MODIFIER, Direction::Release);

    tapped?;
    released?;
    debug!(key = %key, "sent modifier chord");
    Ok(())
}

#[cfg(target_os = "macos")]
fn accessibility_hint() -> &'static str {
    " — grant Accessibility permission in System Settings → Privacy & Security → Accessibility"
}

#[cfg(not(target_os = "macos"))]
fn accessibility_hint() -> &'static str {
    ""
}

#[async_trait]
impl Injector for NativeInjector {
    async fn copy(&self) -> Result<(), InputError> {
        self.chord('c').await
    }

    async fn paste(&self) -> Result<(), InputError> {
        self.chord('v').await
    }

    fn describe(&self) -> String {
        if cfg!(target_os = "macos") {
            "CGEvent via enigo (macOS)".to_string()
        } else {
            "SendInput via enigo (Windows)".to_string()
        }
    }
}
