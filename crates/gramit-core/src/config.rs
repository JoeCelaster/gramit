use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::paths;

/// Turns what a person typed into a base URL the HTTP client can use.
///
/// People type `example.vercel.app`, not `https://example.vercel.app/`, so a bare
/// host gets `https://` and a trailing slash is dropped. An empty input stays empty,
/// which is how "no backend configured" is represented.
pub fn normalize_backend_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

/// A backend address from `GRAMIT_BACKEND_URL`, or `None`.
///
/// Read at run time only — no address is compiled into these binaries. This exists so
/// a developer can aim one process at a local backend without touching the config
/// every other process reads.
pub fn backend_url_from_env() -> Option<String> {
    match std::env::var("GRAMIT_BACKEND_URL") {
        Ok(url) if !url.trim().is_empty() => Some(normalize_backend_url(&url)),
        _ => None,
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

/// What gramit does with a selection.
///
/// One process-wide setting rather than a per-request choice: the hotkey carries no
/// argument, so whatever is in the config is what pressing it means.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    
    /// Grammar, spelling, punctuation only — preserves the author's voice.
    ///
    /// The default, because it is the safe one to press a hotkey on by accident: it
    /// only ever repairs what is already there. Code and write mode both replace the
    /// selection with something new, so they are opted into rather than landed on.
    #[default]
    Grammar,
    /// Read the selection as a brief — "write a mail to Ravi saying I am on leave on
    /// 28 Aug", "essay on urbanisation, 300 words" — and replace it with the finished
    /// piece: an email, an essay, a paragraph, an assignment brief, whatever was asked
    /// for. Nothing of the brief survives; it is carried out, not corrected.
    Write,
    /// Read the selection together with any request written in its comments, and give
    /// back code: the same block carried out, or a whole program when the selection is
    /// only a request like "Write Java code for two sum".
    Code,
}

impl Mode {
    /// Every mode, in the order they are offered at the prompt.
    ///
    /// Grammar first because it is the default and the safe one; write next, because
    /// it is what most people reach for after it; code last, since it is the one you
    /// go looking for on purpose.
    pub const ALL: [Mode; 3] = [Mode::Grammar, Mode::Write, Mode::Code];

    /// One line of help, for the mode prompt and `gramit mode` with no argument.
    pub fn summary(&self) -> &'static str {
        match self {
            Mode::Grammar => "fix grammar, spelling and punctuation — wording is left alone",
            Mode::Write => "write what you ask for — the selection is the brief, not the text",
            Mode::Code => "write and fix code — comments in the selection are the request",
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `f.pad` rather than `write!`, so `{mode:<8}` lines the mode menu up into
        // columns. A plain `write!` silently ignores width and alignment.
        f.pad(match self {
            Mode::Grammar => "grammar",
            Mode::Write => "write",
            Mode::Code => "code",
        })
    }
}

impl std::str::FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "grammar" | "g" | "text" => Ok(Mode::Grammar),
            "write" | "w" | "compose" => Ok(Mode::Write),
            "code" | "c" => Ok(Mode::Code),
            other => Err(format!("unknown mode {other:?} (expected: code, grammar or write)")),
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
    /// Where selections are sent to be fixed. Empty until `gramit setup` fills it in.
    ///
    /// There is deliberately no default and nothing compiled into the binary: this
    /// repository is public, so a built-in address would point every install at
    /// whoever built it and spend that person's model credits. Read it through
    /// [`Config::backend_url`], which also honours `GRAMIT_BACKEND_URL`.
    pub backend_url: String,
    pub mode: Mode,
    pub notifications: bool,
    /// Selections longer than this are refused before a request is made.
    ///
    /// Sized for a whole source file, not a sentence: the usual gesture is select-all
    /// in the file you are editing. It stays under the backend's own 25,000-character
    /// ceiling so an over-long selection is refused here, instantly, rather than after
    /// a round trip.
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
            backend_url: String::new(),
            mode: Mode::Grammar,
            notifications: true,
            max_chars: 16_000,
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
        // Empty means "not set up yet". A fresh install is in exactly that state and
        // has to stay loadable, or `gramit setup` — the thing that fixes it — could
        // not run. A value that is present still has to be a real absolute URL.
        let url = self.backend_url.trim();
        if !url.is_empty() && !(url.starts_with("http://") || url.starts_with("https://")) {
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

    /// The backend to talk to, or `None` when the user has not configured one.
    ///
    /// The environment wins over the saved file so a single process can be pointed
    /// elsewhere without changing what every other process uses.
    pub fn backend_url(&self) -> Option<String> {
        if let Some(from_env) = backend_url_from_env() {
            return Some(from_env);
        }
        let configured = normalize_backend_url(&self.backend_url);
        (!configured.is_empty()).then_some(configured)
    }

    /// Whether there is a backend to talk to at all.
    pub fn has_backend(&self) -> bool {
        self.backend_url().is_some()
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
    fn a_fresh_config_has_no_backend() {
        // The point of the whole design: nothing ships with an address, so a fresh
        // install cannot send anyone's text anywhere until its owner says where.
        let config = Config::default();
        assert_eq!(config.backend_url, "");
        // Honours GRAMIT_BACKEND_URL, so only assert the no-backend case when the
        // developer running the tests has not set one.
        if std::env::var_os("GRAMIT_BACKEND_URL").is_none() {
            assert_eq!(config.backend_url(), None);
            assert!(!config.has_backend());
        }
    }

    #[test]
    fn an_unset_backend_still_validates() {
        // `gramit setup` has to be able to load the config it is about to fix.
        Config { backend_url: String::new(), ..Config::default() }
            .validate()
            .expect("an empty backend_url is 'not set up yet', not an error");
    }

    #[test]
    fn normalizes_what_a_user_would_actually_type() {
        assert_eq!(normalize_backend_url("example.vercel.app"), "https://example.vercel.app");
        assert_eq!(normalize_backend_url("  example.vercel.app/  "), "https://example.vercel.app");
        assert_eq!(normalize_backend_url("http://127.0.0.1:8787/"), "http://127.0.0.1:8787");
        assert_eq!(normalize_backend_url("https://a.b"), "https://a.b");
        assert_eq!(normalize_backend_url("   "), "");
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
        if std::env::var_os("GRAMIT_BACKEND_URL").is_none() {
            assert_eq!(config.backend_url().as_deref(), Some("http://127.0.0.1:8787"));
        }
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
        assert_eq!("code".parse::<Mode>().unwrap(), Mode::Code);
        assert_eq!(" CODE ".parse::<Mode>().unwrap(), Mode::Code);
        assert_eq!("grammar".parse::<Mode>().unwrap(), Mode::Grammar);
        assert_eq!(" GRAMMAR ".parse::<Mode>().unwrap(), Mode::Grammar);
        assert_eq!("write".parse::<Mode>().unwrap(), Mode::Write);
        assert_eq!(" WRITE ".parse::<Mode>().unwrap(), Mode::Write);
        assert!("sarcastic".parse::<Mode>().is_err());
    }

    #[test]
    fn mode_display_honours_width() {
        // The mode prompt prints a padded column; `write!` would drop the padding.
        assert_eq!(format!("[{:<8}]", Mode::Code), "[code    ]");
        assert_eq!(format!("{}", Mode::Grammar), "grammar");
        assert_eq!(format!("[{:<8}]", Mode::Write), "[write   ]");
    }

    #[test]
    fn mode_accepts_a_single_letter() {
        // The mode prompt shows every mode's name, so its first letter has to work.
        assert_eq!("c".parse::<Mode>().unwrap(), Mode::Code);
        assert_eq!("g".parse::<Mode>().unwrap(), Mode::Grammar);
        assert_eq!("w".parse::<Mode>().unwrap(), Mode::Write);
    }

    #[test]
    fn every_mode_round_trips_through_its_own_name() {
        // Display feeds the config file and the wire format, FromStr reads both back.
        for mode in Mode::ALL {
            assert_eq!(mode.to_string().parse::<Mode>().unwrap(), mode);
            let config: Config = toml::from_str(&format!("mode = \"{mode}\"")).unwrap();
            assert_eq!(config.mode, mode);
        }
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
