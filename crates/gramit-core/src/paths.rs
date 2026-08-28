use std::path::PathBuf;

use directories::ProjectDirs;
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Name, ToFsName, ToNsName};

use crate::error::ConfigError;

/// Where the daemon listens and the CLI connects.
///
/// Unix gets a filesystem socket (so we can chmod it 0600 and clean up staleness);
/// Windows gets a named pipe, which lives in its own namespace rather than on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Path(PathBuf),
    Namespaced(String),
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Path(path) => write!(f, "{}", path.display()),
            Endpoint::Namespaced(name) => write!(f, "{name}"),
        }
    }
}

impl Endpoint {
    /// A test endpoint that means the same thing on every platform: an isolated
    /// socket under `dir` on Unix, a uniquely named pipe on Windows.
    ///
    /// Here rather than in the test file because a test that builds the name its own
    /// way stops testing what the daemon does — which is exactly how the Windows CI
    /// failure got in: the tests asked for a filesystem socket at `C:\...\gramit.sock`
    /// while the daemon was opening a named pipe.
    pub fn for_test(dir: &std::path::Path, label: &str) -> Self {
        #[cfg(windows)]
        {
            let _ = dir;
            // A pipe name may not contain a path separator, so the temp directory
            // cannot be part of it. The pid plus the label is what keeps two tests
            // running at once out of each other's way.
            Endpoint::Namespaced(format!("gramit-test-{}-{label}", std::process::id()))
        }
        #[cfg(not(windows))]
        {
            Endpoint::Path(dir.join(label))
        }
    }

    /// What to put in `GRAMIT_SOCKET` for a daemon to land on this endpoint.
    pub fn as_env_value(&self) -> std::ffi::OsString {
        match self {
            Endpoint::Path(path) => path.clone().into_os_string(),
            Endpoint::Namespaced(name) => std::ffi::OsString::from(name),
        }
    }

    /// The socket file, when there is one. Named pipes have no file, so cleaning up
    /// after one — or finding a stale one — is a Unix-only concern.
    pub fn socket_file(&self) -> Option<&std::path::Path> {
        match self {
            Endpoint::Path(path) => Some(path.as_path()),
            Endpoint::Namespaced(_) => None,
        }
    }
}

/// Turns an endpoint into the name `interprocess` binds and connects with.
///
/// The one implementation, used by the daemon, the CLI and the integration tests. The
/// two sides of a socket have to agree on the name exactly, and on Windows the two
/// forms are not interchangeable: a filesystem name has to be a `\\.\pipe\...` path,
/// while an ordinary name goes through the namespaced form instead.
pub fn to_name(endpoint: &Endpoint) -> std::io::Result<Name<'static>> {
    match endpoint {
        Endpoint::Path(path) => path.clone().into_os_string().to_fs_name::<GenericFilePath>(),
        Endpoint::Namespaced(name) => name.clone().to_ns_name::<GenericNamespaced>(),
    }
}

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "gramit")
}

/// Honours `GRAMIT_SOCKET` so tests (and power users) can run an isolated daemon.
pub fn endpoint() -> Endpoint {
    if let Some(override_value) = std::env::var_os("GRAMIT_SOCKET") {
        let value = override_value.to_string_lossy().into_owned();
        #[cfg(windows)]
        {
            return Endpoint::Namespaced(value);
        }
        #[cfg(not(windows))]
        {
            return Endpoint::Path(PathBuf::from(value));
        }
    }

    #[cfg(windows)]
    {
        Endpoint::Namespaced("gramit.sock".to_string())
    }

    #[cfg(not(windows))]
    {
        // $XDG_RUNTIME_DIR is per-user and 0700, which is exactly what we want. The
        // /tmp fallback is shared, so the socket name is qualified by username and the
        // daemon chmods it 0600 after binding.
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(std::env::temp_dir);

        let name = match std::env::var("USER").or_else(|_| std::env::var("LOGNAME")) {
            Ok(user) if !user.is_empty() => format!("gramit-{user}.sock"),
            _ => "gramit.sock".to_string(),
        };
        Endpoint::Path(dir.join(name))
    }
}

pub fn config_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = std::env::var_os("GRAMIT_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    let dirs = project_dirs().ok_or(ConfigError::NoConfigDir)?;
    Ok(dirs.config_dir().join("config.toml"))
}

/// Log file for the daemon. The CLI's `gramit logs` tails this same path.
pub fn log_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = std::env::var_os("GRAMIT_LOG") {
        return Ok(PathBuf::from(path));
    }
    let dirs = project_dirs().ok_or(ConfigError::NoConfigDir)?;
    let dir = dirs.state_dir().unwrap_or_else(|| dirs.data_local_dir()).to_path_buf();
    Ok(dir.join("gramitd.log"))
}

/// Long-lived daemon state that isn't user-editable config — e.g. the Wayland
/// portal `restore_token` that keeps consent from being asked twice (Module 2c).
pub fn state_dir() -> Result<PathBuf, ConfigError> {
    let dirs = project_dirs().ok_or(ConfigError::NoConfigDir)?;
    Ok(dirs.data_local_dir().to_path_buf())
}
