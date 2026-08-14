//! Windows and macOS input, both built on `enigo` (injection) and `global-hotkey`
//! (shortcuts). Linux uses portals instead; see `crate::linux`.

pub mod hotkey;
pub mod inject;
