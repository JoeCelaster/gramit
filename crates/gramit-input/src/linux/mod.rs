//! Linux (Wayland/GNOME) input via XDG desktop portals.
//!
//! Wayland deliberately denies apps global hotkeys and synthetic input. The portals
//! are the sanctioned way back in: `GlobalShortcuts` binds the key, `RemoteDesktop`
//! injects the copy/paste chords. Both prompt the user once; `RemoteDesktop` hands
//! back a restore token we persist so the prompt doesn't come back.

pub mod gnome;
pub mod inject;
pub mod keys;
pub mod shortcuts;
mod token;
