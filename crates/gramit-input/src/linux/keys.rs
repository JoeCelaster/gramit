//! X11 keysyms for injection, and translation of a config hotkey into the trigger
//! syntax the GlobalShortcuts portal expects.

use crate::InputError;

pub const KEYSYM_CONTROL_L: i32 = 0xffe3;
pub const KEYSYM_C: i32 = 0x0063;
pub const KEYSYM_V: i32 = 0x0076;

/// Every modifier, both sides. Released before an injected chord so a hotkey the user
/// is still physically holding cannot combine with it — `Ctrl+Alt` still down when we
/// send `Ctrl+C` turns it into `Ctrl+Alt+C`, which copies nothing.
pub const MODIFIER_KEYSYMS: [i32; 8] = [
    0xffe1, // Shift_L
    0xffe2, // Shift_R
    0xffe3, // Control_L
    0xffe4, // Control_R
    0xffe9, // Alt_L
    0xffea, // Alt_R
    0xffeb, // Super_L
    0xffec, // Super_R
];

/// Converts a config hotkey such as `Ctrl+Alt+F` into a portal trigger such as
/// `CTRL+ALT+f`.
///
/// The XDG shortcuts syntax wants uppercase modifier names and an XKB keysym name for
/// the key itself — lowercase for letters, `F1`-style for function keys.
pub fn to_portal_trigger(hotkey: &str) -> Result<String, InputError> {
    Ok(crate::hotkey_spec::parse(hotkey)?.portal_trigger())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_the_default_hotkey() {
        assert_eq!(to_portal_trigger("Ctrl+Alt+F").unwrap(), "CTRL+ALT+f");
    }

    #[test]
    fn is_case_insensitive_and_tolerates_spaces() {
        assert_eq!(to_portal_trigger(" ctrl + ALT + g ").unwrap(), "CTRL+ALT+g");
    }

    #[test]
    fn accepts_alternative_modifier_names() {
        assert_eq!(to_portal_trigger("Control+Shift+K").unwrap(), "CTRL+SHIFT+k");
        assert_eq!(to_portal_trigger("Super+Space").unwrap(), "SUPER+space");
        assert_eq!(to_portal_trigger("Cmd+J").unwrap(), "SUPER+j");
    }

    #[test]
    fn keeps_function_keys_uppercase() {
        assert_eq!(to_portal_trigger("Ctrl+F5").unwrap(), "CTRL+F5");
        assert_eq!(to_portal_trigger("Ctrl+f12").unwrap(), "CTRL+F12");
    }

    #[test]
    fn maps_named_keys_to_keysym_names() {
        assert_eq!(to_portal_trigger("Ctrl+Enter").unwrap(), "CTRL+Return");
        assert_eq!(to_portal_trigger("Alt+Esc").unwrap(), "ALT+Escape");
    }

    #[test]
    fn deduplicates_repeated_modifiers() {
        assert_eq!(to_portal_trigger("Ctrl+Ctrl+F").unwrap(), "CTRL+f");
    }

    #[test]
    fn rejects_a_hotkey_with_no_modifier() {
        assert!(to_portal_trigger("F").is_err());
    }

    #[test]
    fn rejects_a_hotkey_with_no_key() {
        assert!(to_portal_trigger("Ctrl+Alt").is_err());
    }

    #[test]
    fn rejects_two_non_modifier_keys() {
        assert!(to_portal_trigger("Ctrl+F+G").is_err());
    }

    #[test]
    fn rejects_empty_components() {
        assert!(to_portal_trigger("Ctrl++F").is_err());
        assert!(to_portal_trigger("").is_err());
    }
}
