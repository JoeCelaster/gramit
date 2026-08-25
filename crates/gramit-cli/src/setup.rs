//! `gramit setup` — asks for the backend address and saves it.
//!
//! Nothing in these binaries knows a backend address. This repository is public, so
//! an address compiled in here would aim every install on earth at whoever built the
//! binary and spend their model credits. The address is the user's to supply, and
//! this is where they supply it: once, into their own `config.toml`.

use std::io::{IsTerminal, Write};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use gramit_core::client::BackendClient;
use gramit_core::{config, Config};

use crate::client as ipc;
use crate::ui;

/// Generous: a hosted backend that has scaled to zero can take several seconds to
/// answer the first request, and calling that "unreachable" would be wrong.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// `gramit setup [url] [--force]`.
pub async fn run(url: Option<String>, force: bool) -> Result<()> {
    configure(url, force, true).await
}

/// Guards the commands that cannot work without a backend.
///
/// On a terminal this is the first-run prompt, so `gramit start` on a fresh install
/// simply asks. Without a terminal there is nobody to ask, so it says what to run.
pub async fn ensure_configured() -> Result<()> {
    let config = Config::load().map_err(|err| anyhow!(err))?;
    if config.has_backend() {
        return Ok(());
    }

    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return Err(anyhow!(
            "no backend is configured.\nSet one with: gramit setup <url>"
        ));
    }

    configure(None, false, false).await
}

/// `announce_next_steps` is false when setup runs as a step inside another command,
/// which is about to print its own "started" output.
async fn configure(url: Option<String>, force: bool, announce_next_steps: bool) -> Result<()> {
    let mut config = Config::load().map_err(|err| anyhow!(err))?;

    let raw = match url {
        Some(value) => value,
        None => prompt_for_url(&config.backend_url)?,
    };

    let normalized = config::normalize_backend_url(&raw);
    if normalized.is_empty() {
        return Err(anyhow!("no backend address given"));
    }

    println!("{}", ui::detail(&format!("checking {normalized} ...")));
    match probe(&normalized).await {
        Probe::Ready { model } => {
            let detail = model.map(|model| format!(" ({model})")).unwrap_or_default();
            println!("{}", ui::ok(&format!("{normalized} answered{detail}")));
        }
        Probe::NoKey { missing } => {
            println!("{}", ui::warn(&format!("{normalized} is reachable but has no model credentials")));
            println!("{}", ui::detail(&format!("missing there: {}", missing.join(", "))));
            confirm_anyway(force)?;
        }
        Probe::Unreachable { detail } => {
            println!("{}", ui::warn(&format!("could not reach {normalized}")));
            println!("{}", ui::detail(&detail));
            confirm_anyway(force)?;
        }
    }

    config.backend_url = normalized.clone();
    let written = config.save().map_err(|err| anyhow!(err))?;

    println!("{}", ui::ok(&format!("backend_url = {normalized}")));
    println!("{}", ui::detail(&format!("saved to {}", written.display())));

    if announce_next_steps {
        if ipc::is_running().await {
            println!("{}", ui::detail("restart the daemon to apply: gramit restart"));
        } else {
            println!("{}", ui::detail("start gramit with: gramit start"));
        }
    }
    Ok(())
}

fn prompt_for_url(current: &str) -> Result<String> {
    println!("gramit sends text to a backend that does the correcting.");
    println!(
        "{}",
        ui::detail(
            "No address ships with gramit and no API key is stored on this machine.\n\
             Point it at your own deployment, or at a backend somebody has given you access to.\n\
             To run one yourself: clone the repo and see backend/README, then use\n\
             http://127.0.0.1:8787"
        )
    );
    println!();

    if !current.trim().is_empty() {
        println!("{}", ui::detail(&format!("current: {current}")));
    }

    print!("  Backend URL: ");
    std::io::stdout().flush().ok();

    let mut line = String::new();
    let read = std::io::stdin()
        .read_line(&mut line)
        .context("could not read the backend address")?;

    // Zero bytes means the terminal closed (Ctrl-D) rather than an empty answer.
    if read == 0 {
        return Err(anyhow!("no backend address given"));
    }

    let answer = line.trim().to_string();
    if answer.is_empty() {
        return Err(anyhow!("no backend address given"));
    }
    Ok(answer)
}

/// A backend that is down right now may just be cold-starting, so saving anyway is
/// usually what the user wants — but never without being asked.
fn confirm_anyway(force: bool) -> Result<()> {
    if force {
        return Ok(());
    }
    // A scripted `gramit setup <url>` has nobody to ask; take the address as given.
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return Ok(());
    }

    print!("  Save it anyway? [y/N]: ");
    std::io::stdout().flush().ok();

    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok();

    if is_yes(&line) {
        Ok(())
    } else {
        Err(anyhow!("nothing saved"))
    }
}

fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

enum Probe {
    Ready { model: Option<String> },
    NoKey { missing: Vec<String> },
    Unreachable { detail: String },
}

async fn probe(url: &str) -> Probe {
    let client = match BackendClient::new(url, PROBE_TIMEOUT) {
        Ok(client) => client,
        Err(err) => return Probe::Unreachable { detail: err.message },
    };

    match client.health().await {
        Ok(health) if health.has_key => Probe::Ready { model: health.model },
        Ok(health) => Probe::NoKey { missing: health.missing },
        Err(err) => Probe::Unreachable { detail: err.message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_explicit_yes_confirms() {
        assert!(is_yes("y"));
        assert!(is_yes(" YES \n"));
        assert!(!is_yes(""));
        assert!(!is_yes("\n"));
        assert!(!is_yes("n"));
        // "yeah" is not a yes: anything but the two accepted words means no.
        assert!(!is_yes("yeah"));
    }

    #[test]
    fn a_bare_host_becomes_an_https_url() {
        // The whole point of the prompt: users paste a host, not a URL.
        assert_eq!(
            config::normalize_backend_url("gramit.example.app"),
            "https://gramit.example.app"
        );
    }
}
