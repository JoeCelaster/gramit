//! GNOME custom keybinding — the fallback when the GlobalShortcuts portal refuses us.
//!
//! On GNOME 50 / xdg-desktop-portal 1.21 the GlobalShortcuts portal rejects any app
//! without a sandbox-provided app id (`NotAllowed: An app id is required`), which
//! rules out a plain installed binary. A GNOME custom keybinding has no such
//! requirement: it binds a key to a command, and that command drives the identical
//! daemon code path over IPC.

use std::process::Command;

use crate::InputError;

/// dconf path for gramit's binding. Any leaf name works; a stable one lets us find
/// and update our own entry without disturbing the user's other shortcuts.
pub const KEYBINDING_PATH: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/gramit/";

const MEDIA_KEYS_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_KEYBINDING_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const BINDING_NAME: &str = "gramit: fix selection";

/// Converts a config hotkey such as `Ctrl+Alt+F` into a GTK accelerator, `<Control><Alt>f`.
pub fn to_gtk_accelerator(hotkey: &str) -> Result<String, InputError> {
    Ok(crate::hotkey_spec::parse(hotkey)?.gtk_accelerator())
}

fn gsettings(args: &[&str]) -> Result<String, InputError> {
    let output = Command::new("gsettings")
        .args(args)
        .output()
        .map_err(|err| InputError::Hotkey(format!("could not run gsettings: {err}")))?;

    if !output.status.success() {
        return Err(InputError::Hotkey(format!(
            "gsettings {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Reads the list of custom keybinding paths GNOME currently knows about.
fn existing_paths() -> Result<Vec<String>, InputError> {
    let raw = gsettings(&[
        "get",
        MEDIA_KEYS_SCHEMA,
        "custom-keybindings",
    ])?;
    Ok(parse_path_list(&raw))
}

/// Parses the GVariant string-array syntax gsettings prints, e.g. `['/a/', '/b/']`.
pub fn parse_path_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed == "@as []" || trimmed == "[]" {
        return Vec::new();
    }
    trimmed
        .trim_start_matches("@as ")
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches('\'').trim_matches('"').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn format_path_list(paths: &[String]) -> String {
    let items: Vec<String> = paths.iter().map(|path| format!("'{path}'")).collect();
    format!("[{}]", items.join(", "))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledBinding {
    pub command: String,
    pub binding: String,
}

/// Returns gramit's installed keybinding, if there is one.
pub fn status() -> Result<Option<InstalledBinding>, InputError> {
    if !existing_paths()?.iter().any(|path| path == KEYBINDING_PATH) {
        return Ok(None);
    }

    let target = format!("{CUSTOM_KEYBINDING_SCHEMA}:{KEYBINDING_PATH}");
    let command = gsettings(&["get", &target, "command"])?;
    let binding = gsettings(&["get", &target, "binding"])?;

    Ok(Some(InstalledBinding {
        command: unquote(&command),
        binding: unquote(&binding),
    }))
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('\'').trim_matches('"').to_string()
}

/// Binds `hotkey` to `command`, leaving the user's other custom shortcuts alone.
pub fn install(hotkey: &str, command: &str) -> Result<(), InputError> {
    let accelerator = to_gtk_accelerator(hotkey)?;
    let target = format!("{CUSTOM_KEYBINDING_SCHEMA}:{KEYBINDING_PATH}");

    gsettings(&["set", &target, "name", BINDING_NAME])?;
    gsettings(&["set", &target, "command", command])?;
    gsettings(&["set", &target, "binding", &accelerator])?;

    // Register the path last: GNOME reads the entry as soon as the path is listed, so
    // filling in the fields first avoids a moment where the shortcut exists but is blank.
    let mut paths = existing_paths()?;
    if !paths.iter().any(|path| path == KEYBINDING_PATH) {
        paths.push(KEYBINDING_PATH.to_string());
        gsettings(&["set", MEDIA_KEYS_SCHEMA, "custom-keybindings", &format_path_list(&paths)])?;
    }

    Ok(())
}

/// Removes gramit's keybinding, leaving every other custom shortcut untouched.
pub fn remove() -> Result<(), InputError> {
    let paths: Vec<String> = existing_paths()?
        .into_iter()
        .filter(|path| path != KEYBINDING_PATH)
        .collect();

    gsettings(&["set", MEDIA_KEYS_SCHEMA, "custom-keybindings", &format_path_list(&paths)])?;
    Ok(())
}

/// The command a user can paste into a terminal to bind the shortcut by hand.
pub fn manual_instructions(hotkey: &str, command: &str) -> String {
    let accelerator = to_gtk_accelerator(hotkey).unwrap_or_else(|_| hotkey.to_string());
    let target = format!("{CUSTOM_KEYBINDING_SCHEMA}:{KEYBINDING_PATH}");
    format!(
        "gsettings set {target} name '{BINDING_NAME}'\n\
         gsettings set {target} command '{command}'\n\
         gsettings set {target} binding '{accelerator}'\n\
         gsettings set {MEDIA_KEYS_SCHEMA} custom-keybindings \"['{KEYBINDING_PATH}']\""
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_the_default_hotkey() {
        assert_eq!(to_gtk_accelerator("Ctrl+Alt+F").unwrap(), "<Control><Alt>f");
    }

    #[test]
    fn accepts_alternative_modifier_names() {
        assert_eq!(to_gtk_accelerator("Control+Shift+K").unwrap(), "<Control><Shift>k");
        assert_eq!(to_gtk_accelerator("Super+Space").unwrap(), "<Super>space");
    }

    #[test]
    fn keeps_function_keys_uppercase() {
        assert_eq!(to_gtk_accelerator("Ctrl+F5").unwrap(), "<Control>F5");
    }

    #[test]
    fn rejects_a_hotkey_without_a_modifier() {
        assert!(to_gtk_accelerator("F").is_err());
    }

    #[test]
    fn parses_an_empty_path_list() {
        assert!(parse_path_list("@as []").is_empty());
        assert!(parse_path_list("[]").is_empty());
    }

    #[test]
    fn parses_a_populated_path_list() {
        let raw = "['/org/gnome/a/custom0/', '/org/gnome/a/custom1/']";
        assert_eq!(
            parse_path_list(raw),
            vec!["/org/gnome/a/custom0/", "/org/gnome/a/custom1/"]
        );
    }

    #[test]
    fn formats_a_path_list_gsettings_accepts() {
        let paths = vec!["/a/".to_string(), "/b/".to_string()];
        assert_eq!(format_path_list(&paths), "['/a/', '/b/']");
        assert_eq!(format_path_list(&[]), "[]");
    }

    #[test]
    fn a_formatted_list_round_trips_through_the_parser() {
        let paths = vec![KEYBINDING_PATH.to_string(), "/other/".to_string()];
        assert_eq!(parse_path_list(&format_path_list(&paths)), paths);
    }

    #[test]
    fn manual_instructions_mention_the_accelerator_and_command() {
        let text = manual_instructions("Ctrl+Alt+F", "/usr/local/bin/gramit fix --selection");
        assert!(text.contains("<Control><Alt>f"), "{text}");
        assert!(text.contains("gramit fix --selection"), "{text}");
    }
}
