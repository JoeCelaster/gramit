//! Terminal output helpers. Colour only when stdout is a terminal, so piped output
//! and the GNOME keybinding's captured output stay clean.

use std::io::IsTerminal;
use std::sync::OnceLock;

fn colour_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        // Honour the NO_COLOR convention, then fall back to a terminal check.
        std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
    })
}

fn paint(code: &str, text: &str) -> String {
    if colour_enabled() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn green(text: &str) -> String {
    paint("32", text)
}

pub fn red(text: &str) -> String {
    paint("31", text)
}

pub fn yellow(text: &str) -> String {
    paint("33", text)
}

pub fn dim(text: &str) -> String {
    paint("2", text)
}

pub fn bold(text: &str) -> String {
    paint("1", text)
}

pub fn ok(label: &str) -> String {
    format!("{} {label}", green("✓"))
}

pub fn fail(label: &str) -> String {
    format!("{} {label}", red("✗"))
}

pub fn warn(label: &str) -> String {
    format!("{} {label}", yellow("!"))
}

/// Indented follow-up lines under a check. Every line is indented, so a multi-line
/// remedy stays visually attached to its check instead of falling back to column 0.
pub fn detail(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {}", dim(line)))
        .collect::<Vec<_>>()
        .join("\n")
}
