//! The gramit daemon.
//!
//! Module 2a: config, the local socket server, and the backend call. The hotkey,
//! clipboard, and paste path arrive in 2b/2c and hang off the same `DaemonState`.

mod endpoint;
mod fixloop;
mod handler;
mod hotkey_loop;
mod logging;
mod notify;
mod selection;
mod server;
mod shutdown;
mod state;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use gramit_core::client::BackendClient;
use gramit_core::{paths, Config, VERSION};
use gramit_input::{clipboard, hotkey, injector, HotkeyRegistration};
use tracing::{error, info, warn};

use crate::fixloop::Selection;
use crate::shutdown::Shutdown;
use crate::state::DaemonState;

const USAGE: &str = "\
gramitd — the gramit daemon

USAGE:
    gramitd [OPTIONS]

OPTIONS:
    -f, --foreground    Also log to stderr and stay attached to this terminal
    -h, --help          Print this help
    -V, --version       Print the version

ENVIRONMENT:
    GRAMIT_CONFIG       Override the config file path
    GRAMIT_SOCKET       Override the IPC socket path
    GRAMIT_LOG          Override the log file path
    GRAMIT_LOG_LEVEL    Log filter (default: info)
";

struct Args {
    foreground: bool,
}

fn parse_args() -> Result<Option<Args>> {
    let mut foreground = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-f" | "--foreground" => foreground = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("gramitd {VERSION}");
                return Ok(None);
            }
            other => anyhow::bail!("unknown argument {other:?}\n\n{USAGE}"),
        }
    }

    Ok(Some(Args { foreground }))
}

fn main() -> Result<()> {
    let Some(args) = parse_args()? else {
        return Ok(());
    };

    // `main` is deliberately not `#[tokio::main]`. On macOS, Carbon delivers global
    // hotkey events only while the *main thread's* run loop is being pumped, so the
    // daemon runs on the runtime's worker threads and this thread becomes the pump.
    // Elsewhere `pump_until` just idles, which keeps one shape on every platform.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the async runtime")?;

    // Windows and macOS require the hotkey manager to be created on the thread that
    // pumps the event loop — this one. Linux returns None and registers later, via
    // the portal, from the runtime. The guard keeps the manager alive on this thread.
    let config = match Config::load() {
        Ok(config) => config,
        Err(err) => return Err(anyhow::anyhow!(err)).context("could not load the gramit config"),
    };
    let (_main_thread_hotkey, prebound) = hotkey::register_on_main_thread(&config.hotkey);

    let stop = Arc::new(AtomicBool::new(false));
    let daemon = {
        let stop = Arc::clone(&stop);
        runtime.spawn(async move {
            let result = run(args, config, prebound).await;
            // Release the pump whether the daemon stopped cleanly or failed.
            stop.store(true, Ordering::Relaxed);
            result
        })
    };

    gramit_input::run_loop::pump_until(stop);

    runtime.block_on(daemon).context("the daemon task ended unexpectedly")?
}

async fn run(
    args: Args,
    config: Config,
    prebound: Option<Result<HotkeyRegistration, gramit_input::InputError>>,
) -> Result<()> {

    let log_file = logging::init(args.foreground)?;
    info!(version = VERSION, log = %log_file.display(), "gramitd starting");

    let endpoint = paths::endpoint();
    let listener = endpoint::bind(&endpoint).await?;
    // A daemon with no backend is still worth starting: it answers IPC, so `gramit
    // status` and `gramit doctor` can report the one thing that is missing.
    let client = match config.backend_url() {
        Some(url) => {
            info!(%endpoint, backend = %url, "listening");
            Some(
                BackendClient::new(&url, Duration::from_millis(config.request_timeout_ms))
                    .map_err(|err| anyhow::anyhow!("could not build the backend client: {err}"))?,
            )
        }
        None => {
            info!(%endpoint, "listening");
            warn!("no backend configured; corrections will be refused until `gramit setup` runs");
            None
        }
    };

    // The selection machinery and the hotkey are both optional: without them the
    // daemon still answers IPC, so `gramit fix` works and `gramit doctor` can say
    // exactly what is missing instead of the daemon just refusing to start.
    let selection = open_selection().await;
    let registration = match &selection {
        Some(_) => match prebound {
            // Already attempted on the main thread (Windows/macOS).
            Some(result) => report_hotkey(result, &config.hotkey),
            None => report_hotkey(hotkey::register(&config.hotkey).await, &config.hotkey),
        },
        None => {
            warn!("skipping hotkey registration: there is no way to capture a selection");
            None
        }
    };

    let shutdown = Shutdown::new();
    let state = Arc::new(
        DaemonState::new(config.clone(), client)
            .with_selection(selection)
            .with_notifier(notify::for_config(&config))
            .with_hotkey(registration.as_ref().map(|r| r.description.clone())),
    );

    let hotkey_task = registration.map(|registration| {
        tokio::spawn(hotkey_loop::run(registration, Arc::clone(&state), shutdown.clone()))
    });

    let signals = {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            wait_for_signal().await;
            info!("signal received, shutting down");
            shutdown.trigger();
        })
    };

    let result = server::serve(listener, state, shutdown.clone()).await;

    if let Some(task) = hotkey_task {
        task.abort();
    }
    signals.abort();
    endpoint::cleanup(&endpoint);

    match &result {
        Ok(()) => info!("gramitd stopped"),
        Err(err) => error!(%err, "gramitd stopped with an error"),
    }
    result
}

/// Opens the clipboard and the keystroke injector, prompting for permission if the
/// platform requires it. A failure here is degraded operation, not a fatal error.
async fn open_selection() -> Option<Selection> {
    let clipboard = match clipboard::open() {
        Ok(clipboard) => clipboard,
        Err(err) => {
            warn!(code = err.code(), %err, "no clipboard: selection fixes are unavailable");
            return None;
        }
    };

    let injector = match injector::open().await {
        Ok(injector) => injector,
        Err(err) => {
            warn!(code = err.code(), %err, "no keystroke injection: selection fixes are unavailable");
            return None;
        }
    };

    info!(injector = %injector.describe(), "selection machinery ready");
    Some(Selection { clipboard, injector })
}

// `hotkey` is only read by the Linux fallback instructions below.
#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
fn report_hotkey(
    result: Result<HotkeyRegistration, gramit_input::InputError>,
    hotkey: &str,
) -> Option<HotkeyRegistration> {
    match result {
        Ok(registration) => Some(registration),
        Err(err) => {
            // Expected on GNOME: the GlobalShortcuts portal refuses apps without a
            // sandbox app id. The custom-keybinding fallback drives the identical
            // code path, so say exactly how to set it up rather than just failing.
            warn!(
                code = err.code(),
                %err,
                "could not register the global hotkey; falling back to a desktop keybinding"
            );
            #[cfg(target_os = "linux")]
            {
                let command = std::env::current_exe()
                    .ok()
                    .and_then(|path| path.parent().map(|dir| dir.join("gramit")))
                    .map(|path| format!("{} fix --selection", path.display()))
                    .unwrap_or_else(|| "gramit fix --selection".to_string());
                warn!(
                    "bind the hotkey yourself with:\n{}",
                    gramit_input::linux_gnome::manual_instructions(hotkey, &command)
                );
            }
            None
        }
    }
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(stream) => stream,
        Err(err) => {
            error!(%err, "could not listen for SIGTERM");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
