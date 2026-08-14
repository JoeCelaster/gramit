//! `gramit start` / `stop` / `restart` / `status`.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use gramit_core::ipc::{Request, Response, StatusReport};

use crate::client;
use crate::ui;

const READY_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// Finds `gramitd`, preferring the copy sitting beside this binary so a build tree and
/// an installed copy never get mixed up.
fn daemon_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("could not locate the gramit binary")?;
    let name = if cfg!(windows) { "gramitd.exe" } else { "gramitd" };

    if let Some(sibling) = exe.parent().map(|dir| dir.join(name)) {
        if sibling.is_file() {
            return Ok(sibling);
        }
    }

    // Fall back to PATH resolution by the OS.
    Ok(PathBuf::from(name))
}

pub async fn start(foreground: bool) -> Result<()> {
    if client::is_running().await {
        println!("{}", ui::ok("gramit is already running"));
        return print_status().await;
    }

    let binary = daemon_binary()?;

    if foreground {
        println!("{}", ui::dim(&format!("running {} in the foreground", binary.display())));
        let status = Command::new(&binary)
            .arg("--foreground")
            .status()
            .with_context(|| format!("could not run {}", binary.display()))?;
        return if status.success() {
            Ok(())
        } else {
            Err(anyhow!("gramitd exited with {status}"))
        };
    }

    Command::new(&binary)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| {
            format!(
                "could not start {}.\nBuild it with `cargo build --release`, or put it on your PATH.",
                binary.display()
            )
        })?;

    if !client::wait_until_ready(READY_TIMEOUT).await {
        return Err(anyhow!(
            "gramitd did not start listening within {}s.\nCheck the log: gramit logs",
            READY_TIMEOUT.as_secs()
        ));
    }

    println!("{}", ui::ok("gramit started"));
    print_status().await
}

pub async fn stop() -> Result<()> {
    if !client::is_running().await {
        println!("{}", ui::dim("gramit is not running"));
        return Ok(());
    }

    match client::request(Request::Shutdown).await {
        Ok(Response::Ok) => {}
        Ok(Response::Error { message, .. }) => return Err(anyhow!(message)),
        Ok(other) => return Err(anyhow!("unexpected reply to shutdown: {other:?}")),
        // The daemon can close the socket before the reply lands, which is a
        // successful shutdown, not a failure.
        Err(_) => {}
    }

    if !client::wait_until_stopped(STOP_TIMEOUT).await {
        return Err(anyhow!("gramitd is still running after {}s", STOP_TIMEOUT.as_secs()));
    }

    println!("{}", ui::ok("gramit stopped"));
    Ok(())
}

pub async fn restart() -> Result<()> {
    stop().await?;
    start(false).await
}

pub async fn status() -> Result<()> {
    if !client::is_running().await {
        println!("{}", ui::fail("gramit is not running"));
        println!("{}", ui::detail("start it with: gramit start"));
        std::process::exit(1);
    }
    print_status().await
}

async fn print_status() -> Result<()> {
    let report = match client::request(Request::Status).await? {
        Response::Status(report) => report,
        Response::Error { message, .. } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected reply to status: {other:?}")),
    };
    print_report(&report);
    Ok(())
}

pub fn print_report(report: &StatusReport) {
    println!();
    println!("  {}  {} (pid {})", ui::bold("daemon"), report.version, report.pid);
    println!("  {}  {}", ui::bold("uptime"), format_uptime(report.uptime_s));

    let hotkey = if report.hotkey_registered {
        ui::green(report.hotkey_detail.as_deref().unwrap_or(&report.hotkey))
    } else {
        // Not an error on Linux: the GNOME keybinding drives the same path.
        ui::yellow(&format!("{} (not bound by the daemon)", report.hotkey))
    };
    println!("  {}  {hotkey}", ui::bold("hotkey"));

    let selection = match (&report.selection_ready, &report.injector) {
        (true, Some(injector)) => ui::green(injector),
        _ => ui::red("unavailable"),
    };
    println!("  {}  {selection}", ui::bold("typing"));

    let backend = if report.backend_reachable {
        match report.backend_has_key {
            Some(true) => ui::green(&report.backend_url),
            _ => ui::yellow(&format!("{} (no API key)", report.backend_url)),
        }
    } else {
        ui::red(&format!("{} (unreachable)", report.backend_url))
    };
    println!("  {}  {backend}", ui::bold("backend"));

    if let Some(detail) = &report.backend_detail {
        println!("{}", ui::detail(detail));
    }

    println!("  {}  {}", ui::bold("fixes "), report.fixes_total);

    if let Some(error) = &report.last_error {
        println!("  {}  {}", ui::bold("last error"), ui::red(error));
    }
    println!();
}

fn format_uptime(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {}s", s / 60, s % 60),
        s => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_uptime_at_each_scale() {
        assert_eq!(format_uptime(5), "5s");
        assert_eq!(format_uptime(59), "59s");
        assert_eq!(format_uptime(60), "1m 0s");
        assert_eq!(format_uptime(3599), "59m 59s");
        assert_eq!(format_uptime(3600), "1h 0m");
        assert_eq!(format_uptime(7860), "2h 11m");
    }
}
