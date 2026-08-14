//! macOS main-thread run loop.
//!
//! Carbon delivers hotkey events to a handler installed on
//! `GetApplicationEventTarget()`, and that handler only runs while the **main
//! thread's** run loop is being pumped. A daemon that just parks `main` on a Tokio
//! runtime would register its hotkey successfully and then never hear a single press.
//!
//! So on macOS `gramitd` inverts the usual shape: Tokio runs on worker threads and
//! `main` calls [`pump_until`], which services the run loop until the daemon stops.
//! On other platforms `pump_until` simply idles, so `main` keeps one shape everywhere.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// How long each pump services the run loop before re-checking the stop flag.
/// Short enough that shutdown feels instant, long enough to stay idle-cheap.
const SLICE_SECONDS: f64 = 0.25;

#[cfg(target_os = "macos")]
mod ffi {
    use std::ffi::c_void;

    pub type CFTimeInterval = f64;
    pub type CFStringRef = *const c_void;
    pub type CFRunLoopRunResult = i32;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub static kCFRunLoopDefaultMode: CFStringRef;

        pub fn CFRunLoopRunInMode(
            mode: CFStringRef,
            seconds: CFTimeInterval,
            return_after_source_handled: u8,
        ) -> CFRunLoopRunResult;
    }
}

/// Services the main run loop until `stop` is set. Must be called on the main thread.
#[cfg(target_os = "macos")]
pub fn pump_until(stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        // Returning after each slice (rather than CFRunLoopRun + CFRunLoopStop from
        // another thread) keeps shutdown a plain atomic read, with no cross-thread
        // run-loop surgery to get wrong.
        unsafe {
            ffi::CFRunLoopRunInMode(ffi::kCFRunLoopDefaultMode, SLICE_SECONDS, 0);
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
