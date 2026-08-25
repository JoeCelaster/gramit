use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::paths;

/// The backend every crate talks to, and the only place its address appears in the
/// Rust sources. The value comes from `backend.url` in the workspace `deploy.toml`,
/// read at build time by `build.rs` — deployments change it there, not in code.
pub const DEFAULT_BACKEND_URL: &str = env!("GRAMIT_BACKEND_URL");

/// The backend address a fresh config starts with.
///
/// `GRAMIT_BACKEND_URL` is honoured at run time as well as at build time, so a
/// developer can aim the daemon at a local backend for one run without rebuilding.
/// An explicit `backend_url` in `config.toml` still wins over both.
pub fn default_backend_url() -> String {
    match std::env::var("GRAMIT_BACKEND_URL") {
        Ok(url) if !url.trim().is_empty() => url.trim().to_string(),
        _ => DEFAULT_BACKEND_URL.to_string(),
    }
}

/// Where a selection is read from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capture {
    /// Inject Ctrl+C and read the clipboard. Works in every app, but depends on the
    /// keystroke actually reaching the focused window.
    #[default]
    Copy,
    /// Read the X11 PRIMARY selection directly — no keystroke at all.
    ///
    /// Not the default on purpose: an app that does not publish PRIMARY leaves a
    /// *stale* selection there, and we would paste text from some other window over
    /// what the user has highlighted. Silently corrupting their text is worse than
    /// reporting "nothing selected".
    Primary,
}

impl std::fmt::Display for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Capture::Copy => write!(f, "copy"),
            Capture::Primary => write!(f, "primary"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Grammar, spelling, punctuation only — preserves the author's voice.
    #[default]
    Grammar,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Grammar => write!(f, "grammar"),
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "grammar" => Ok(Mode::Grammar),
            other => Err(format!("unknown mode {other:?} (expected: grammar)")),
        }
    }
}

/// User-editable settings, read from `config.toml`.
///
/// `deny_unknown_fields` is deliberate: this file is hand-edited, and a silently
/// ignored typo like `notification = false` is worse than a startup error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub hotkey: String,
    pub backend_url: String,
    pub mode: Mode,
    pub notifications: bool,
    /// Selections longer than this are refused before a request is made.
    pub max_chars: usize,
    /// Budget for one correction. The default backend is remote, so this covers a
    /// round trip plus the model's own latency, not just the model's.
    pub request_timeout_ms: u64,

    // Timings for the capture → correct → paste loop. Defaults are tuned for GNOME
    // on this hardware; slower machines or remote sessions may need larger values.
    /// Where the selection is read from: `copy` (inject Ctrl+C) or `primary`.
    pub capture: Capture,
    /// Grace period for the user to let go of the hotkey before we inject Ctrl+C,
    /// so their held modifiers don't combine with ours.
    pub modifier_release_ms: u64,
    /// Total budget for getting the copy to land. The copy is retried within this
    /// window, because the user may still be holding the hotkey on the first attempt.
    pub copy_settle_ms: u64,
    /// How long to watch the clipboard before injecting another Ctrl+C.
    pub copy_retry_interval_ms: u64,
    /// Pause after putting the correction on the clipboard, before pasting.
    pub paste_delay_ms: u64,
    /// Pause after pasting, before restoring the user's clipboard — restore too
    /// early and the target app pastes the old contents.
    pub restore_delay_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Alt+F".to_string(),
            backend_url: default_backend_url(),
            mode: Mode::Grammar,
            notifications: true,
            max_chars: 8_000,
            request_timeout_ms: 15_000,
            capture: Capture::Copy,
            // Generous on purpose: a Ctrl+Alt+F chord is commonly held 300-500ms, and
            // anything we inject while it is down arrives as Ctrl+Alt+C.
            modifier_release_ms: 250,
            copy_settle_ms: 1_500,
            copy_retry_interval_ms: 200,
            paste_delay_ms: 120,
            restore_delay_ms: 200,
        }
    }
}

impl Config {
    /// Loads the config, falling back to defaults when the file doesn't exist yet.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&paths::config_path()?)
    }

    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                config.validate()?;
                return Ok(config);
            }
            Err(source) => return Err(ConfigError::Read { path: path.to_path_buf(), source }),
        };

        let config: Self = toml::from_str(&text)
            .map_err(|err| ConfigError::Parse { path: path.to_path_buf(), message: err.to_string() })?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self) -> Result<PathBuf, ConfigError> {
        let path = paths::config_path()?;
        self.save_to(&path)?;
        Ok(path)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| ConfigError::Write { path: parent.to_path_buf(), source })?;
        }
        let text = toml::to_string_pretty(self).map_err(|err| ConfigError::Serialize(err.to_string()))?;
        std::fs::write(path, text)
            .map_err(|source| ConfigError::Write { path: path.to_path_buf(), source })
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.hotkey.trim().is_empty() {
            return Err(ConfigError::Invalid("hotkey must not be empty".into()));
        }
        let url = self.backend_url.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ConfigError::Invalid(format!(
                "backend_url must start with http:// or https://, got {url:?}"
            )));
        }
        if self.max_chars == 0 {
            return Err(ConfigError::Invalid("max_chars must be greater than 0".into()));
        }
        if self.request_timeout_ms == 0 {
            return Err(ConfigError::Invalid("request_timeout_ms must be greater than 0".into()));
        }
        if self.copy_retry_interval_ms == 0 {
            return Err(ConfigError::Invalid("copy_retry_interval_ms must be greater than 0".into()));
        }
        Ok(())
    }

    pub fn backend_url_trimmed(&self) -> &str {
        self.backend_url.trim().trim_end_matches('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().expect("defaults must validate");
    }

    #[test]
    fn the_default_backend_comes_from_deploy_toml() {
        // Guards the rule this crate's build.rs exists to enforce: the address is
        // configuration, so it arrives from deploy.toml rather than a literal here.
        assert!(
            DEFAULT_BACKEND_URL.starts_with("http://") || DEFAULT_BACKEND_URL.starts_with("https://"),
            "GRAMIT_BACKEND_URL must be an absolute URL, got {DEFAULT_BACKEND_URL:?}"
        );
        assert_eq!(Config::default().backend_url, default_backend_url());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let path = std::env::temp_dir().join("gramit-does-not-exist-9e3f.toml");
        assert_eq!(Config::load_from(&path).unwrap(), Config::default());
    }

    #[test]
    fn partial_config_fills_in_defaults() {
        let config: Config = toml::from_str("max_chars = 500").unwrap();
        assert_eq!(config.max_chars, 500);
        assert_eq!(config.hotkey, Config::default().hotkey);
    }

    #[test]
    fn unknown_field_is_an_error() {
        let err = toml::from_str::<Config>("notification = false").unwrap_err();
        assert!(err.to_string().contains("notification"), "{err}");
    }

    #[test]
    fn round_trips_through_toml() {
        let config = Config {
            max_chars: 1234,
            hotkey: "Ctrl+Alt+G".into(),
            ..Config::default()
        };

        let text = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn rejects_a_url_without_a_scheme() {
        let config = Config { backend_url: "127.0.0.1:8787".into(), ..Config::default() };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_limits() {
        let config = Config { max_chars: 0, ..Config::default() };
        assert!(config.validate().is_err());

        let config = Config { request_timeout_ms: 0, ..Config::default() };
        assert!(config.validate().is_err());
    }

    #[test]
    fn trims_trailing_slash_from_backend_url() {
        let config =
            Config { backend_url: "http://127.0.0.1:8787/".into(), ..Config::default() };
        assert_eq!(config.backend_url_trimmed(), "http://127.0.0.1:8787");
    }

    #[test]
    fn capture_defaults_to_copy() {
        // Reading PRIMARY by default would risk pasting a stale selection.
        assert_eq!(Config::default().capture, Capture::Copy);
    }

    #[test]
    fn capture_round_trips_through_toml() {
        let config = Config { capture: Capture::Primary, ..Config::default() };
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(text.contains("primary"), "{text}");
        assert_eq!(toml::from_str::<Config>(&text).unwrap().capture, Capture::Primary);
    }

    #[test]
    fn the_copy_budget_allows_several_attempts() {
        // The whole point of the retry: one attempt lands while keys are still held.
        let config = Config::default();
        assert!(
            config.copy_settle_ms / config.copy_retry_interval_ms >= 4,
            "copy_settle_ms must allow at least a few retries"
        );
    }

    #[test]
    fn rejects_a_zero_retry_interval() {
        let config = Config { copy_retry_interval_ms: 0, ..Config::default() };
        assert!(config.validate().is_err());
    }

    #[test]
    fn mode_parses_from_str() {
        assert_eq!("grammar".parse::<Mode>().unwrap(), Mode::Grammar);
        assert_eq!(" GRAMMAR ".parse::<Mode>().unwrap(), Mode::Grammar);
        assert!("sarcastic".parse::<Mode>().is_err());
    }

    #[test]
    fn saves_and_reloads() {
        let dir = std::env::temp_dir().join(format!("gramit-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        let config = Config { notifications: false, ..Config::default() };

        config.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), config);

        std::fs::remove_dir_all(&dir).ok();
    }
}
