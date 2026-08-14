use async_trait::async_trait;

use crate::InputError;

/// Sends the copy and paste chords to whichever window has focus.
///
/// The modifier differs by platform (Cmd on macOS, Ctrl elsewhere); implementations
/// hide that, so callers only ever say "copy" or "paste".
#[async_trait]
pub trait Injector: Send + Sync {
    async fn copy(&self) -> Result<(), InputError>;
    async fn paste(&self) -> Result<(), InputError>;
    /// Human-readable mechanism, shown by `gramit status` and `gramit doctor`.
    fn describe(&self) -> String;
}

/// Opens the platform's injector, prompting for permission if the OS requires it.
#[cfg(target_os = "linux")]
pub async fn open() -> Result<Box<dyn Injector>, InputError> {
    let injector = crate::linux::inject::PortalInjector::connect().await?;
    Ok(Box::new(injector))
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub async fn open() -> Result<Box<dyn Injector>, InputError> {
    Ok(Box::new(crate::native::inject::NativeInjector::new()?))
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub async fn open() -> Result<Box<dyn Injector>, InputError> {
    Err(InputError::Unsupported(
        "keystroke injection is not supported on this platform".into(),
    ))
}
