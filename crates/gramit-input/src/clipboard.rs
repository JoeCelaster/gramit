use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use tracing::debug;

use crate::InputError;

#[async_trait]
pub trait Clipboard: Send + Sync {
    /// `Ok(None)` means "nothing readable as text" — an empty clipboard, or one
    /// holding an image or files.
    async fn get_text(&self) -> Result<Option<String>, InputError>;
    async fn set_text(&self, text: String) -> Result<(), InputError>;
    async fn clear(&self) -> Result<(), InputError>;

    /// The X11 PRIMARY selection — whatever the user currently has highlighted,
    /// published by most apps without any copy taking place.
    ///
    /// Used two ways: as a diagnostic (PRIMARY has text but the clipboard stayed empty
    /// ⇒ our injected Ctrl+C never reached the app), and as an opt-in capture source.
    /// Returns `Ok(None)` where the concept doesn't exist.
    async fn get_primary_text(&self) -> Result<Option<String>, InputError> {
        Ok(None)
    }
}

/// What the clipboard held before a fix, so it can be put back afterwards.
///
/// Known v1 limitation: non-text content (images, files) reads as `Empty`, so it is
/// cleared rather than restored. Restoring it would mean compiling arboard's
/// `image-data` support and round-tripping raw bitmaps for a rare case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardSnapshot {
    Text(String),
    Empty,
}

impl ClipboardSnapshot {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ClipboardSnapshot::Text(text) => Some(text),
            ClipboardSnapshot::Empty => None,
        }
    }
}

pub async fn snapshot(clipboard: &dyn Clipboard) -> Result<ClipboardSnapshot, InputError> {
    Ok(match clipboard.get_text().await? {
        Some(text) => ClipboardSnapshot::Text(text),
        None => ClipboardSnapshot::Empty,
    })
}

pub async fn restore(
    clipboard: &dyn Clipboard,
    snapshot: &ClipboardSnapshot,
) -> Result<(), InputError> {
    match snapshot {
        ClipboardSnapshot::Text(text) => clipboard.set_text(text.clone()).await,
        ClipboardSnapshot::Empty => clipboard.clear().await,
    }
}

/// Opens the platform clipboard.
pub fn open() -> Result<Box<dyn Clipboard>, InputError> {
    Ok(Box::new(ArboardClipboard::new()?))
}

enum Command {
    GetText(oneshot::Sender<Result<Option<String>, InputError>>),
    GetPrimary(oneshot::Sender<Result<Option<String>, InputError>>),
    SetText(String, oneshot::Sender<Result<(), InputError>>),
    Clear(oneshot::Sender<Result<(), InputError>>),
}

/// The clipboard, owned by a dedicated thread.
///
/// Two reasons it can't just be called inline: `arboard::Clipboard` is blocking and
/// not `Sync`, and on X11 the process that set the clipboard must stay alive to serve
/// it — dropping the instance would drop the user's text with it. One long-lived
/// thread solves both, and makes the handle cheap to clone and share.
pub struct ArboardClipboard {
    commands: mpsc::Sender<Command>,
}

impl ArboardClipboard {
    pub fn new() -> Result<Self, InputError> {
        let (commands, mut rx) = mpsc::channel::<Command>(16);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        std::thread::Builder::new()
            .name("gramit-clipboard".to_string())
            .spawn(move || {
                let mut clipboard = match arboard::Clipboard::new() {
                    Ok(clipboard) => {
                        let _ = ready_tx.send(Ok(()));
                        clipboard
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err.to_string()));
                        return;
                    }
                };

                while let Some(command) = rx.blocking_recv() {
                    match command {
                        Command::GetText(reply) => {
                            let _ = reply.send(read_text(&mut clipboard));
                        }
                        Command::GetPrimary(reply) => {
                            let _ = reply.send(read_primary(&mut clipboard));
                        }
                        Command::SetText(text, reply) => {
                            let result = clipboard
                                .set_text(text)
                                .map_err(|err| InputError::Clipboard(err.to_string()));
                            let _ = reply.send(result);
                        }
                        Command::Clear(reply) => {
                            let result = clipboard
                                .clear()
                                .map_err(|err| InputError::Clipboard(err.to_string()));
                            let _ = reply.send(result);
                        }
                    }
                }
                debug!("clipboard thread stopping");
            })
            .map_err(|err| InputError::Clipboard(format!("could not start the clipboard thread: {err}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { commands }),
            Ok(Err(err)) => Err(InputError::Clipboard(format!("could not open the clipboard: {err}"))),
            Err(_) => Err(InputError::Clipboard("the clipboard thread died on startup".into())),
        }
    }

    async fn send<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<T, InputError>>) -> Command,
    ) -> Result<T, InputError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.commands
            .send(make(reply_tx))
            .await
            .map_err(|_| InputError::Clipboard("the clipboard thread is gone".into()))?;
        reply_rx
            .await
            .map_err(|_| InputError::Clipboard("the clipboard thread dropped the request".into()))?
    }
}

/// arboard reports "empty" and "holds an image" the same way, so both become `None`.
fn read_text(clipboard: &mut arboard::Clipboard) -> Result<Option<String>, InputError> {
    match clipboard.get_text() {
        Ok(text) => Ok(Some(text)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(err) => Err(InputError::Clipboard(err.to_string())),
    }
}

#[cfg(target_os = "linux")]
fn read_primary(clipboard: &mut arboard::Clipboard) -> Result<Option<String>, InputError> {
    use arboard::{GetExtLinux, LinuxClipboardKind};

    match clipboard.get().clipboard(LinuxClipboardKind::Primary).text() {
        Ok(text) => Ok(Some(text)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        // Not every compositor exposes PRIMARY. That is not an error worth failing a
        // fix over — it only costs us the diagnostic.
        Err(err) => {
            debug!(%err, "could not read the PRIMARY selection");
            Ok(None)
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn read_primary(_clipboard: &mut arboard::Clipboard) -> Result<Option<String>, InputError> {
    Ok(None)
}

#[async_trait]
impl Clipboard for ArboardClipboard {
    async fn get_text(&self) -> Result<Option<String>, InputError> {
        self.send(Command::GetText).await
    }

    async fn set_text(&self, text: String) -> Result<(), InputError> {
        self.send(|reply| Command::SetText(text, reply)).await
    }

    async fn clear(&self) -> Result<(), InputError> {
        self.send(Command::Clear).await
    }

    async fn get_primary_text(&self) -> Result<Option<String>, InputError> {
        self.send(Command::GetPrimary).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeClipboard;

    #[tokio::test]
    async fn snapshot_captures_text() {
        let clipboard = FakeClipboard::with_text("hello");
        assert_eq!(
            snapshot(&clipboard).await.unwrap(),
            ClipboardSnapshot::Text("hello".into())
        );
    }

    #[tokio::test]
    async fn snapshot_of_an_empty_clipboard_is_empty() {
        let clipboard = FakeClipboard::empty();
        assert_eq!(snapshot(&clipboard).await.unwrap(), ClipboardSnapshot::Empty);
    }

    #[tokio::test]
    async fn restore_puts_the_original_text_back() {
        let clipboard = FakeClipboard::with_text("original");
        let saved = snapshot(&clipboard).await.unwrap();

        clipboard.set_text("correction".into()).await.unwrap();
        restore(&clipboard, &saved).await.unwrap();

        assert_eq!(clipboard.get_text().await.unwrap().as_deref(), Some("original"));
    }

    #[tokio::test]
    async fn restoring_an_empty_snapshot_clears_the_clipboard() {
        let clipboard = FakeClipboard::empty();
        let saved = snapshot(&clipboard).await.unwrap();

        clipboard.set_text("correction".into()).await.unwrap();
        restore(&clipboard, &saved).await.unwrap();

        assert_eq!(clipboard.get_text().await.unwrap(), None);
    }
}
