//! macOS main-thread event pump.
//!
//! Carbon delivers hotkey events to a handler installed on
//! `GetApplicationEventTarget()`, and reaching that handler takes **two** things on the
//! main thread, not one: something must *dequeue* the press from the process event
//! queue, and something must *dispatch* it to the target. A daemon that just parks
//! `main` on a Tokio runtime would register its hotkey successfully and then never hear
//! a single press.
//!
//! Pumping `CFRunLoopRunInMode` is not enough, which is subtle enough to be worth
//! stating: `global-hotkey` installs a Carbon handler and registers the key, and it
//! adds a run-loop source for *media keys only*. An ordinary chord like `Ctrl+Alt+F`
//! has no run-loop source at all, so a CFRunLoop pump services timers and observers
//! while every press sits in the queue unread.
//!
//! So on macOS `gramitd` inverts the usual shape: Tokio runs on worker threads and
//! `main` calls [`pump_until`], which drains and dispatches events until the daemon
//! stops. On other platforms `pump_until` simply idles, so `main` keeps one shape
//! everywhere.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// How long each pump services the run loop before re-checking the stop flag.
/// Short enough that shutdown feels instant, long enough to stay idle-cheap.
const SLICE_SECONDS: f64 = 0.25;

#[cfg(target_os = "macos")]
mod ffi {
    use std::ffi::c_void;

    pub type OSStatus = i32;
    pub type EventRef = *mut c_void;
    pub type EventTargetRef = *mut c_void;
    /// Carbon's `ItemCount` is `unsigned long`, so 64-bit on every Mac we build for.
    pub type ItemCount = usize;
    /// `EventTimeout` is a `double` in seconds; `kEventDurationSecond` is `1.0`.
    pub type EventTimeout = f64;
    /// Carbon's `Boolean` is an `unsigned char`.
    pub type Boolean = u8;

    pub const NO_ERR: OSStatus = 0;
    /// `eventLoopTimedOutErr` — the wait expired with nothing queued.
    pub const EVENT_LOOP_TIMED_OUT_ERR: OSStatus = -9875;
    /// `eventLoopQuitErr` — someone called `QuitApplicationEventLoop`. Nothing here
    /// does, so it means the queue is gone and this loop has no reason to continue.
    pub const EVENT_LOOP_QUIT_ERR: OSStatus = -9876;

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        pub fn ReceiveNextEvent(
            num_types: ItemCount,
            list: *const c_void,
            timeout: EventTimeout,
            pull_event: Boolean,
            out_event: *mut EventRef,
        ) -> OSStatus;

        pub fn SendEventToEventTarget(event: EventRef, target: EventTargetRef) -> OSStatus;

        pub fn GetEventDispatcherTarget() -> EventTargetRef;

        pub fn ReleaseEvent(event: EventRef);
    }
}

/// Drains and dispatches the main event queue until `stop` is set. Must be called on
/// the main thread.
///
/// `ReceiveNextEvent` runs the run loop while it waits, so run-loop sources, timers and
/// observers keep being serviced exactly as a `CFRunLoopRunInMode` pump would — this
/// dispatches Carbon events *as well*, rather than instead. It also needs no
/// `NSApplication`, which keeps a background daemon out of the Dock and away from the
/// user's focus.
#[cfg(target_os = "macos")]
pub fn pump_until(stop: Arc<AtomicBool>) {
    use tracing::warn;

    let slice = std::time::Duration::from_secs_f64(SLICE_SECONDS);
    let target = unsafe { ffi::GetEventDispatcherTarget() };

    while !stop.load(Ordering::Relaxed) {
        let mut event: ffi::EventRef = std::ptr::null_mut();

        // Waiting a slice at a time (rather than forever, woken from another thread)
        // keeps shutdown a plain atomic read, with no cross-thread event-loop surgery
        // to get wrong. A count of 0 with a null list means "any event".
        let status = unsafe {
            ffi::ReceiveNextEvent(0, std::ptr::null(), SLICE_SECONDS, 1, &mut event)
        };

        match status {
            // Nothing queued this slice. The idle path, not a failure — and the one
            // this loop spends almost all of its life on.
            ffi::EVENT_LOOP_TIMED_OUT_ERR => continue,

            ffi::NO_ERR if !event.is_null() => unsafe {
                // The dispatcher routes the event on to the application target, where
                // `global-hotkey`'s handler lives. Anything nobody claims comes back
                // `eventNotHandledErr`, which is the normal outcome for most of what a
                // daemon with no windows is handed, and is nothing to report.
                ffi::SendEventToEventTarget(event, target);
                // We asked for the event to be pulled, so this reference is ours to drop.
                ffi::ReleaseEvent(event);
            },

            ffi::EVENT_LOOP_QUIT_ERR => {
                warn!("the macOS event queue quit; the hotkey is no longer live");
                break;
            }

            // Should not happen. Sleep out the slice before trying again: without this,
            // a status that keeps repeating turns the daemon into a busy loop that eats
            // a core for as long as it runs.
            _ => {
                warn!(status, "unexpected status from ReceiveNextEvent");
                std::thread::sleep(slice);
            }
        }
    }
}

/// Windows delivers `WM_HOTKEY` to the hidden window `global-hotkey` creates, whose
/// `WndProc` only runs while this thread dispatches messages.
#[cfg(target_os = "windows")]
pub fn pump_until(stop: Arc<AtomicBool>) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    while !stop.load(Ordering::Relaxed) {
        let mut message: MSG = unsafe { std::mem::zeroed() };
        // Drain everything queued, then idle briefly so this stays cheap.
        while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        std::thread::sleep(std::time::Duration::from_secs_f64(SLICE_SECONDS));
    }
}

/// Everywhere else there is no main-thread requirement, so this just idles until the
/// daemon stops, keeping `main`'s shape identical across platforms.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn pump_until(stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_secs_f64(SLICE_SECONDS));
    }
}

/// Whether this platform needs [`pump_until`] on the main thread for hotkeys to fire.
pub const fn required() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}
