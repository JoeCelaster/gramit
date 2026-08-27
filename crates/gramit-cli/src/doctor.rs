//! `gramit doctor` — one command that says why the hotkey isn't working.
//!
//! Every failed check prints a remedy. A check that can't tell you what to do next
//! isn't worth running.

use std::time::Duration;

use anyhow::{anyhow, Result};
use gramit_core::client::BackendClient;
use gramit_core::ipc::{Request, Response, StatusReport};
use gramit_core::{paths, Config};

use crate::client;
use crate::ui;

#[derive(Default)]
struct Findings {
    failures: usize,
    warnings: usize,
}

impl Findings {
    fn ok(&self, label: &str, detail: &str) {
        println!("{}", ui::ok(label));
        if !detail.is_empty() {
            println!("{}", ui::detail(detail));
        }
    }

    fn warn(&mut self, label: &str, remedy: &str) {
        self.warnings += 1;
        println!("{}", ui::warn(label));
        println!("{}", ui::detail(remedy));
    }

    fn fail(&mut self, label: &str, remedy: &str) {
        self.failures += 1;
        println!("{}", ui::fail(label));
        println!("{}", ui::detail(remedy));
    }
}

pub async fn run(apply_fixes: bool) -> Result<()> {
    let mut findings = Findings::default();
    println!();

    let config = check_config(&mut findings);
    let status = check_daemon(&mut findings).await;
    check_backend(&mut findings, config.as_ref(), status.as_deref()).await;
    check_typing(&mut findings, status.as_deref());
    check_hotkey(&mut findings, config.as_ref(), status.as_deref(), apply_fixes);

    println!();
    if findings.failures > 0 {
        println!(
            "{}",
            ui::red(&format!(
                "{} problem(s) to fix{}",
                findings.failures,
                if findings.warnings > 0 {
                    format!(", {} warning(s)", findings.warnings)
                } else {
                    String::new()
                }
            ))
        );
        println!();
        std::process::exit(1);
    }

    if findings.warnings > 0 {
        println!("{}", ui::yellow(&format!("{} warning(s)", findings.warnings)));
    } else {
        println!("{}", ui::green("everything looks good"));
    }
    println!();
    Ok(())
}

fn check_config(findings: &mut Findings) -> Option<Config> {
    let path = match paths::config_path() {
        Ok(path) => path,
        Err(err) => {
            findings.fail("config", &format!("could not determine the config path: {err}"));
            return None;
        }
    };

    match Config::load() {
        Ok(config) => {
            let detail = if path.exists() {
                format!("{}", path.display())
            } else {
                format!("{} (using defaults; the file does not exist yet)", path.display())
            };
            findings.ok("config", &detail);
            Some(config)
        }
        Err(err) => {
            findings.fail("config", &format!("{err}\nfix or delete {}", path.display()));
            None
        }
    }
}

async fn check_daemon(findings: &mut Findings) -> Option<Box<StatusReport>> {
    match client::request(Request::Status).await {
        Ok(Response::Status(report)) => {
            findings.ok(
                "daemon",
                &format!("running, version {} (pid {})", report.version, report.pid),
            );
            Some(report)
        }
        Ok(Response::Error { message, .. }) => {
            findings.fail("daemon", &format!("the daemon replied with an error: {message}"));
            None
        }
        Ok(_) | Err(_) => {
            findings.fail("daemon", "not running — start it with: gramit start");
            None
        }
    }
}

/// Said the same way wherever the backend turns out to be unset, because it is the
/// one finding a brand-new install will always have.
const NOT_CONFIGURED: &str = "no backend configured — set one with: gramit setup";

async fn check_backend(
    findings: &mut Findings,
    config: Option<&Config>,
    status: Option<&StatusReport>,
) {
    // Prefer the daemon's view, since that is the connection that actually matters.
    if let Some(report) = status {
        let Some(url) = report.backend_url.as_deref() else {
            findings.fail("backend", NOT_CONFIGURED);
            return;
        };
        if !report.backend_reachable {
            findings.fail(
                "backend",
                &format!(
                    "{url} is unreachable.\n\
                     check the address with: gramit config get backend_url\n\
                     change it with: gramit setup"
                ),
            );
            return;
        }
        match report.backend_has_key {
            Some(true) => findings.ok("backend", url),
            _ => findings.fail(
                "backend",
                &format!(
                    "{url} is running but has no Azure OpenAI key.\n\
                     that is set where the backend is deployed, not on this machine"
                ),
            ),
        }
        return;
    }

    // No daemon, so ask the backend ourselves rather than reporting nothing.
    let Some(config) = config else { return };
    let Some(url) = config.backend_url() else {
        findings.fail("backend", NOT_CONFIGURED);
        return;
    };
    let client = match BackendClient::new(&url, Duration::from_secs(5)) {
        Ok(client) => client,
        Err(err) => {
            findings.fail("backend", &format!("could not build an HTTP client: {err}"));
            return;
        }
    };

    match client.health().await {
        Ok(health) if health.has_key => findings.ok("backend", &url),
        Ok(health) => findings.fail(
            "backend",
            &format!(
                "running but has no model credentials; missing: {}.\n\
                 those are set where the backend is deployed, not on this machine",
                health.missing.join(", ")
            ),
        ),
        Err(err) => findings.fail(
            "backend",
            &format!("{}\npoint gramit somewhere else with: gramit setup", err.message),
        ),
    }
}

fn check_typing(findings: &mut Findings, status: Option<&StatusReport>) {
    let Some(report) = status else {
        findings.warn("typing", "cannot check without a running daemon");
        return;
    };

    if report.selection_ready {
        findings.ok("typing", report.injector.as_deref().unwrap_or("available"));
        return;
    }

    let remedy = if cfg!(target_os = "macos") {
        "no keystroke injection.\n\
         grant Accessibility in System Settings → Privacy & Security → Accessibility,\n\
         then run: gramit restart"
    } else if cfg!(target_os = "linux") {
        "no keystroke injection.\n\
         the RemoteDesktop portal was refused or is stuck. Try:\n\
         systemctl --user restart xdg-desktop-portal-gnome xdg-desktop-portal && gramit restart"
    } else {
        "no keystroke injection — see `gramit logs` for the reason"
    };
    findings.fail("typing", remedy);
}

fn check_hotkey(
    findings: &mut Findings,
    config: Option<&Config>,
    status: Option<&StatusReport>,
    apply_fixes: bool,
) {
    let hotkey = config
        .map(|config| config.hotkey.clone())
        .or_else(|| status.map(|report| report.hotkey.clone()))
        .unwrap_or_else(|| "Ctrl+Alt+F".to_string());

    // `hotkey` stays the canonical config spelling — the GNOME fallback below binds
    // against it. Only what the user reads goes through `display`.
    let shown = gramit_input::hotkey_spec::display(&hotkey);

    if let Some(report) = status {
        if report.hotkey_registered {
            let detail = report.hotkey_detail.as_deref().unwrap_or(&hotkey);
            findings.ok("hotkey", &gramit_input::hotkey_spec::display(detail));
            return;
        }
    }

    #[cfg(target_os = "linux")]
    {
        let _ = shown;
        check_gnome_keybinding(findings, &hotkey, apply_fixes);
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = apply_fixes;
        findings.fail(
            "hotkey",
            &format!("{shown} is not registered — see `gramit logs` for the reason"),
        );
    }
}

/// The command a desktop keybinding should run: an absolute path, since the desktop
/// environment does not inherit the user's PATH.
///
/// Linux-only, like its one caller: nothing else installs a desktop keybinding, and an
/// ungated helper here is dead code on macOS and Windows, which CI builds with
/// `-D warnings`.
#[cfg(target_os = "linux")]
fn selection_command() -> String {
    match std::env::current_exe() {
        Ok(path) => format!("{} fix --selection", path.display()),
        Err(_) => "gramit fix --selection".to_string(),
    }
}

#[cfg(target_os = "linux")]
fn check_gnome_keybinding(findings: &mut Findings, hotkey: &str, apply_fixes: bool) {
    use gramit_input::linux_gnome;

    let command = selection_command();

    match linux_gnome::status() {
        Ok(Some(binding)) => {
            let accelerator = linux_gnome::to_gtk_accelerator(hotkey).unwrap_or_default();

            if binding.binding != accelerator {
                findings.warn(
                    "hotkey",
                    &format!(
                        "the desktop shortcut is bound to {} but the config says {hotkey} ({accelerator}).\n\
                         re-run with: gramit doctor --fix",
                        binding.binding
                    ),
                );
            } else if binding.command != command {
                findings.warn(
                    "hotkey",
                    &format!(
                        "the desktop shortcut runs a different binary:\n  {}\nexpected:\n  {command}\n\
                         re-run with: gramit doctor --fix",
                        binding.command
                    ),
                );
            } else {
                findings.ok("hotkey", &format!("{hotkey} via a GNOME keybinding"));
            }
        }
        Ok(None) if apply_fixes => match linux_gnome::install(hotkey, &command) {
            Ok(()) => findings.ok("hotkey", &format!("{hotkey} installed as a GNOME keybinding")),
            Err(err) => findings.fail("hotkey", &format!("could not install the keybinding: {err}")),
        },
        Ok(None) => findings.fail(
            "hotkey",
            &format!(
                "{hotkey} is not bound.\n\
                 The GlobalShortcuts portal refuses apps without a sandbox app id, so gramit\n\
                 uses a GNOME keybinding instead. Install it with:\n\
                 gramit doctor --fix"
            ),
        ),
        Err(err) => findings.fail("hotkey", &format!("could not read GNOME keybindings: {err}")),
    }
}

/// `gramit doctor --fix` with nothing else to do still needs a sane exit.
pub fn ensure_supported() -> Result<()> {
    if cfg!(any(target_os = "linux", target_os = "macos", target_os = "windows")) {
        Ok(())
    } else {
        Err(anyhow!("gramit does not support this platform"))
    }
}
