use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// Sets up logging and returns the log file path.
///
/// The file layer is always on — `gramit logs` tails it, and a daemon started in the
/// background has nowhere else to write. `--foreground` adds a stderr layer on top.
pub fn init(foreground: bool) -> Result<PathBuf> {
    let path = gramit_core::paths::log_path().context("could not determine the log path")?;
    let dir = path
        .parent()
        .map(PathBuf::from)
        .context("log path has no parent directory")?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("could not create log directory {}", dir.display()))?;

    let file_name = path
        .file_name()
        .context("log path has no file name")?
        .to_owned();

    let filter = EnvFilter::try_from_env("GRAMIT_LOG_LEVEL").unwrap_or_else(|_| EnvFilter::new("info"));
    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(tracing_appender::rolling::never(&dir, &file_name));

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(foreground.then(|| fmt::layer().with_writer(std::io::stderr)))
        .init();

    Ok(path)
}
