//! One parser for the configured hotkey, and the three spellings each platform wants.
//!
//! `Ctrl+Alt+F` has to become `CTRL+ALT+f` for the XDG portal, `<Control><Alt>f` for a
//! GNOME keybinding, and `Modifiers::CONTROL | ALT` + `Code::KeyF` for `global-hotkey`
//! on Windows and macOS. Parsing once here keeps those three from drifting apart, and
//! keeps the logic testable on any OS — the platform crates aren't involved.

use crate::InputError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeySpec {
    modifiers: Vec<Modifier>,
    /// The key in XKB spelling: `f`, `F5`, `space`, `Return`.
    key: String,
}

pub fn parse(hotkey: &str) -> Result<HotkeySpec, InputError> {
    let mut modifiers: Vec<Modifier> = Vec::new();
    let mut key: Option<String> = None;

    for raw in hotkey.split('+') {
        let part = raw.trim();
        if part.is_empty() {
            return Err(InputError::Hotkey(format!("{hotkey:?} has an empty component")));
        }

        let modifier = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Modifier::Ctrl),
            "alt" | "option" => Some(Modifier::Alt),
            "shift" => Some(Modifier::Shift),
            "super" | "win" | "cmd" | "command" | "meta" => Some(Modifier::Super),
            _ => None,
        };

        match modifier {
            Some(modifier) => {
                if !modifiers.contains(&modifier) {
                    modifiers.push(modifier);
                }
            }
            None => {
                if key.is_some() {
                    return Err(InputError::Hotkey(format!(
                        "{hotkey:?} names more than one non-modifier key"
                    )));
                }
                key = Some(normalize_key(part));
            }
        }
    }

    let key = key
        .ok_or_else(|| InputError::Hotkey(format!("{hotkey:?} has no key, only modifiers")))?;

    if modifiers.is_empty() {
        return Err(InputError::Hotkey(format!(
            "{hotkey:?} has no modifier; a bare key would fire while typing"
        )));
    }

    Ok(HotkeySpec { modifiers, key })
}

/// XKB spelling: lowercase letters, `F1`-style function keys, capitalised named keys.
fn normalize_key(key: &str) -> String {
    let lower = key.to_ascii_lowercase();

    if lower.len() >= 2 && lower.starts_with('f') {
        if let Ok(number) = lower[1..].parse::<u8>() {
            if (1..=24).contains(&number) {
                return format!("F{number}");
            }
        }
    }

    match lower.as_str() {
        "enter" | "return" => "Return".to_string(),
        "tab" => "Tab".to_string(),
        "escape" | "esc" => "Escape".to_string(),
        _ => lower,
    }
}

impl HotkeySpec {
    pub fn has(&self, modifier: Modifier) -> bool {
        self.modifiers.contains(&modifier)
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// XDG GlobalShortcuts trigger, e.g. `CTRL+ALT+f`.
    pub fn portal_trigger(&self) -> String {
        let mut out = String::new();
        for modifier in &self.modifiers {
            out.push_str(match modifier {
                Modifier::Ctrl => "CTRL",
                Modifier::Alt => "ALT",
                Modifier::Shift => "SHIFT",
                Modifier::Super => "SUPER",
            });
            out.push('+');
        }
        out.push_str(&self.key);
        out
    }

    /// GTK accelerator, e.g. `<Control><Alt>f`.
    pub fn gtk_accelerator(&self) -> String {
        let mut out = String::new();
        for modifier in &self.modifiers {
            out.push_str(match modifier {
                Modifier::Ctrl => "<Control>",
                Modifier::Alt => "<Alt>",
                Modifier::Shift => "<Shift>",
                Modifier::Super => "<Super>",
            });
        }
        out.push_str(&self.key);
        out
    }

    /// The `KeyboardEvent.code` name `global-hotkey` parses, e.g. `KeyF`, `F5`, `Space`.
    ///
    /// Returns None for keys with no such spelling, so the caller can report an
    /// unsupported hotkey instead of binding the wrong key.
    pub fn web_code(&self) -> Option<String> {
        let key = &self.key;

        if key.len() == 1 {
            let ch = key.chars().next()?;
            if ch.is_ascii_lowercase() {
                return Some(format!("Key{}", ch.to_ascii_uppercase()));
            }
            if ch.is_ascii_digit() {
                return Some(format!("Digit{ch}"));
            }
            return None;
        }

        if key.starts_with('F') && key[1..].parse::<u8>().is_ok() {
            return Some(key.clone());
        }

        match key.as_str() {
            "space" => Some("Space".to_string()),
            "Return" => Some("Enter".to_string()),
            "Tab" => Some("Tab".to_string()),
            "Escape" => Some("Escape".to_string()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_default_hotkey() {
        let spec = parse("Ctrl+Alt+F").unwrap();
        assert!(spec.has(Modifier::Ctrl));
        assert!(spec.has(Modifier::Alt));
        assert!(!spec.has(Modifier::Shift));
        assert_eq!(spec.key(), "f");
    }

    #[test]
    fn is_case_insensitive_and_tolerates_spaces() {
        assert_eq!(parse(" ctrl + ALT + g ").unwrap().portal_trigger(), "CTRL+ALT+g");
    }

    #[test]
    fn accepts_alternative_modifier_names() {
        assert!(parse("Cmd+J").unwrap().has(Modifier::Super));
        assert!(parse("Command+J").unwrap().has(Modifier::Super));
        assert!(parse("Win+J").unwrap().has(Modifier::Super));
        assert!(parse("Option+J").unwrap().has(Modifier::Alt));
    }

    #[test]
    fn deduplicates_repeated_modifiers() {
        assert_eq!(parse("Ctrl+Ctrl+F").unwrap().portal_trigger(), "CTRL+f");
    }

    #[test]
    fn rejects_malformed_hotkeys() {
        assert!(parse("F").is_err(), "a bare key would fire while typing");
        assert!(parse("Ctrl+Alt").is_err(), "modifiers with no key");
        assert!(parse("Ctrl+F+G").is_err(), "two non-modifier keys");
        assert!(parse("Ctrl++F").is_err(), "empty component");
        assert!(parse("").is_err());
    }

    #[test]
    fn renders_all_three_platform_spellings() {
        let spec = parse("Ctrl+Alt+F").unwrap();
        assert_eq!(spec.portal_trigger(), "CTRL+ALT+f");
        assert_eq!(spec.gtk_accelerator(), "<Control><Alt>f");
        assert_eq!(spec.web_code().unwrap(), "KeyF");
    }

    #[test]
    fn renders_function_keys_consistently() {
        let spec = parse("Ctrl+F5").unwrap();
        assert_eq!(spec.portal_trigger(), "CTRL+F5");
        assert_eq!(spec.gtk_accelerator(), "<Control>F5");
        assert_eq!(spec.web_code().unwrap(), "F5");
    }

    #[test]
    fn renders_named_keys_consistently() {
        let enter = parse("Ctrl+Enter").unwrap();
        assert_eq!(enter.portal_trigger(), "CTRL+Return");
        assert_eq!(enter.web_code().unwrap(), "Enter");

        let space = parse("Super+Space").unwrap();
        assert_eq!(space.gtk_accelerator(), "<Super>space");
        assert_eq!(space.web_code().unwrap(), "Space");
    }

    #[test]
    fn maps_digits_to_web_codes() {
        assert_eq!(parse("Ctrl+Alt+1").unwrap().web_code().unwrap(), "Digit1");
    }

    #[test]
    fn reports_keys_with_no_web_code() {
        // Parsed fine, but `global-hotkey` has no name for it — better to refuse than
        // to bind some other key.
        assert_eq!(parse("Ctrl+Alt+ü").unwrap().web_code(), None);
    }

    #[test]
    fn modifier_order_follows_the_users_spelling() {
        assert_eq!(parse("Alt+Ctrl+F").unwrap().portal_trigger(), "ALT+CTRL+f");
    }
}
