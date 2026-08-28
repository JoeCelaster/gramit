//! `gramit version` and `gramit update` — what you are running, and getting the next one.
//!
//! The update deliberately does not reimplement installing. It asks GitHub for the
//! latest release, and when there is a newer one it runs the same install script the
//! README tells people to curl — the one that verifies a checksum, stops the daemon
//! before replacing a file it is executing, and clears macOS quarantine. A second
//! installer written in Rust would be a second installer to keep correct, and the
//! interesting failures here (a half-written binary, a locked file) are exactly the
//! ones the script already handles.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use gramit_core::ipc::{Request, Response};
use gramit_core::VERSION;

use crate::client as ipc;
use crate::lifecycle;
use crate::ui;

const REPO: &str = "JoeCelaster/gramit";
/// The same constant `gramit --version` and the daemon's status report use, so the
/// three can never disagree about what this build is.
const CURRENT: &str = VERSION;

/// Where the release list lives. The API is used rather than scraping the releases
/// page because `tag_name` is a field there rather than something to parse out of HTML.
fn latest_release_url() -> String {
    format!("https://api.github.com/repos/{REPO}/releases/latest")
}

/// The installer, taken from `main` rather than from the release tag: it is the script
/// that knows how to install *any* version, and a fix to it should reach people
/// without waiting for the next release.
fn installer_url() -> String {
    let name = if cfg!(windows) { "install.ps1" } else { "install.sh" };
    format!("https://raw.githubusercontent.com/{REPO}/main/{name}")
}

/// A release version, compared the way releases are ordered rather than as a string —
/// "1.10.0" is newer than "1.9.0", which sorting text gets backwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    /// `Some("rc1")` for `1.2.0-rc1`. A pre-release sorts *before* the release it
    /// leads to, so `gramit update` never offers an rc to someone on the final build.
    pre: Option<String>,
}

impl std::str::FromStr for Version {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let text = s.trim().trim_start_matches(['v', 'V']);
        let (core, pre) = match text.split_once(['-', '+']) {
            Some((core, pre)) => (core, Some(pre.to_string())),
            None => (text, None),
        };

        let mut parts = core.split('.');
        let mut next = |what: &str| -> Result<u64> {
            parts
                .next()
                .ok_or_else(|| anyhow!("{s:?} has no {what} number"))?
                .parse::<u64>()
                .with_context(|| format!("{s:?} has a {what} that is not a number"))
        };

        let version = Version {
            major: next("major")?,
            minor: next("minor")?,
            patch: next("patch")?,
            pre,
        };
        if parts.next().is_some() {
            bail!("{s:?} has more than three version numbers");
        }
        Ok(version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        match &self.pre {
            Some(pre) => write!(f, "-{pre}"),
            None => Ok(()),
        }
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                // 1.2.0 is newer than 1.2.0-rc1: the release comes after its own
                // candidates, so someone on the final build is never offered one.
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
                (None, None) => Ordering::Equal,
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// `gramit version` — what this binary is, and what is running.
///
/// Offline and instant on purpose: it answers "which version am I on", which is what
/// someone asks when reading a bug report. Checking the internet is `gramit update`.
pub async fn version() -> Result<()> {
    println!("{}", ui::ok(&format!("gramit {CURRENT}")));

    if let Ok(exe) = std::env::current_exe() {
        println!("{}", ui::detail(&format!("binary   {}", exe.display())));
    }

    // The daemon holds whatever version it was started from, which after an update is
    // the *old* one until it restarts. Saying so here saves a confusing bug report.
    match daemon_version().await {
        Some(version) if version != CURRENT => {
            println!("{}", ui::detail(&format!("daemon   {version} (running)")));
            println!();
            println!(
                "{}",
                ui::warn(&format!(
                    "the running daemon is {version}, not {CURRENT} — restart it with: gramit restart"
                )),
            );
        }
        Some(version) => println!("{}", ui::detail(&format!("daemon   {version} (running)"))),
        None => println!("{}", ui::detail("daemon   not running")),
    }

    println!();
    println!("{}", ui::detail("check for a newer release with: gramit update"));
    Ok(())
}

/// The version the running daemon was started from, or `None` when it is not running.
async fn daemon_version() -> Option<String> {
    match ipc::request(Request::Status).await {
        Ok(Response::Status(report)) => Some(report.version),
        _ => None,
    }
}

/// `gramit update [--check] [--yes]`.
pub async fn run(check_only: bool, assume_yes: bool) -> Result<()> {
    let current: Version = CURRENT.parse().context("this build has an unreadable version")?;

    println!("{}", ui::detail(&format!("current  {current}")));
    let latest = fetch_latest_tag().await?;
    let available: Version = latest.parse().with_context(|| {
        format!("GitHub returned a release tag this build cannot read: {latest:?}")
    })?;
    println!("{}", ui::detail(&format!("latest   {available}")));
    println!();

    if available <= current {
        println!("{}", ui::ok(&format!("gramit {current} is the latest release")));
        return Ok(());
    }

    println!("{}", ui::ok(&format!("gramit {available} is available")));
    println!(
        "{}",
        ui::detail(&format!("release notes: https://github.com/{REPO}/releases/tag/{latest}")),
    );

    if check_only {
        println!();
        println!("{}", ui::detail("install it with: gramit update"));
        return Ok(());
    }

    let install_dir = install_dir()?;
    println!();
    println!("{}", ui::detail(&format!("installing into {}", install_dir.display())));
    println!("{}", ui::detail(&format!("using {}", installer_url())));

    if !assume_yes && !confirm()? {
        println!("{}", ui::detail("nothing changed"));
        return Ok(());
    }

    // Asked before the daemon is stopped, so it can be put back exactly as it was.
    let was_running = ipc::is_running().await;

    let script = download_installer().await?;
    run_installer(&script, &install_dir, &latest)?;

    println!();
    match installed_version(&install_dir) {
        Some(version) => println!("{}", ui::ok(&format!("updated to {version}"))),
        None => println!("{}", ui::ok(&format!("updated to {available}"))),
    }

    if was_running {
        println!("{}", ui::detail("restarting gramit ..."));
        lifecycle::start(false).await?;
    } else {
        println!("{}", ui::detail("start it with: gramit start"));
    }
    Ok(())
}

/// The directory the update lands in: wherever this binary already lives, so an
/// update replaces the gramit the user is actually running rather than installing a
/// second one into the installer's default and leaving PATH to pick between them.
fn install_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("could not find this binary on disk")?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", exe.display()))?;
    Ok(dir.to_path_buf())
}

async fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        // GitHub rejects a request with no user agent outright.
        .user_agent(format!("gramit/{CURRENT}"))
        .build()
        .context("could not build an HTTP client")
}

async fn fetch_latest_tag() -> Result<String> {
    let response = http()
        .await?
        .get(latest_release_url())
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .context("could not reach github.com to ask for the latest release")?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("{REPO} has no published releases yet");
    }
    if response.status() == reqwest::StatusCode::FORBIDDEN {
        bail!(
            "github.com refused the request — its unauthenticated rate limit is 60 an hour \
             and this address has spent it. Try again later, or download the release by hand \
             from https://github.com/{REPO}/releases/latest"
        );
    }
    if !response.status().is_success() {
        bail!("github.com answered {} when asked for the latest release", response.status());
    }

    let body: serde_json::Value = response
        .json()
        .await
        .context("github.com returned something that is not the release JSON")?;

    body.get("tag_name")
        .and_then(|tag| tag.as_str())
        .map(|tag| tag.to_string())
        .ok_or_else(|| anyhow!("the latest release has no tag_name"))
}

/// Downloads the install script to a temp file rather than piping it into a shell, so
/// what runs is a file on disk that can be read afterwards if anything goes wrong.
async fn download_installer() -> Result<PathBuf> {
    let url = installer_url();
    let response = http()
        .await?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("could not download {url}"))?;

    if !response.status().is_success() {
        bail!("{url} answered {}", response.status());
    }
    let body = response.text().await.context("the installer did not download cleanly")?;
    if body.trim().is_empty() {
        bail!("{url} returned an empty file");
    }

    let name = if cfg!(windows) { "gramit-install.ps1" } else { "gramit-install.sh" };
    let path = std::env::temp_dir().join(format!("{}-{}", std::process::id(), name));
    std::fs::write(&path, body).with_context(|| format!("could not write {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("could not make {} executable", path.display()))?;
    }
    Ok(path)
}

fn run_installer(script: &Path, install_dir: &Path, tag: &str) -> Result<()> {
    // Windows will not let anything write over a running .exe, and the running .exe
    // here is the one being replaced. Renaming it *is* allowed, and leaves the path
    // free for the installer to write. Put back if the install fails.
    let displaced = displace_self()?;

    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(script);
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.arg(script);
        c
    };

    command
        .env("GRAMIT_INSTALL_DIR", install_dir)
        .env("GRAMIT_VERSION", tag)
        // PATH already has this directory — the user just ran gramit out of it — so
        // there is nothing to add and no shell rc to touch.
        .env("GRAMIT_NO_MODIFY_PATH", "1");

    println!();
    let status = command.status().with_context(|| {
        if cfg!(windows) {
            "could not run powershell to install the update".to_string()
        } else {
            "could not run sh to install the update".to_string()
        }
    });

    match status {
        Ok(status) if status.success() => {
            std::fs::remove_file(script).ok();
            if let Some(old) = displaced {
                // Nothing holds the renamed file on Unix; on Windows the lock lasts as
                // long as this process, so it is left for the next run to sweep up.
                std::fs::remove_file(&old).ok();
            }
            Ok(())
        }
        Ok(status) => {
            restore_self(displaced);
            bail!(
                "the installer failed ({status}). It is saved at {} if you want to run it by hand.",
                script.display()
            )
        }
        Err(err) => {
            restore_self(displaced);
            Err(err)
        }
    }
}

/// Renames this binary out of the way so the installer can write over its path.
/// Returns where it went, or `None` where nothing had to move.
fn displace_self() -> Result<Option<PathBuf>> {
    if !cfg!(windows) {
        return Ok(None);
    }
    let exe = std::env::current_exe().context("could not find this binary on disk")?;
    let old = exe.with_extension("exe.old");
    // A leftover from a previous update, now unlocked. Failing to remove it is not
    // fatal; the rename below is what matters.
    std::fs::remove_file(&old).ok();
    std::fs::rename(&exe, &old)
        .with_context(|| format!("could not move {} aside for the update", exe.display()))?;
    Ok(Some(old))
}

fn restore_self(displaced: Option<PathBuf>) {
    let Some(old) = displaced else { return };
    let Ok(exe) = std::env::current_exe() else { return };
    if !exe.exists() {
        // The installer never got as far as writing one, so put the old binary back
        // rather than leave the user with no gramit at all.
        std::fs::rename(&old, &exe).ok();
    }
}

/// Asks the freshly installed binary what it is, which is the only answer that proves
/// the update landed rather than merely that the installer exited zero.
fn installed_version(install_dir: &Path) -> Option<String> {
    let exe = install_dir.join(if cfg!(windows) { "gramit.exe" } else { "gramit" });
    let output = std::process::Command::new(exe).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    text.split_whitespace().last().map(|version| version.to_string())
}

fn confirm() -> Result<bool> {
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() {
        bail!("not a terminal, so there is nobody to ask. Re-run with: gramit update --yes");
    }

    print!("  Install it now? [y/N]: ");
    std::io::stdout().flush().ok();

    let mut line = String::new();
    std::io::stdin().read_line(&mut line).context("could not read the answer")?;
    Ok(matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        text.parse().expect(text)
    }

    #[test]
    fn parses_a_release_tag_with_or_without_its_v() {
        assert_eq!(v("v1.2.3"), v("1.2.3"));
        assert_eq!(v(" 1.2.3 \n"), v("1.2.3"));
    }

    #[test]
    fn orders_by_number_not_by_text() {
        // The bug this exists to prevent: "1.10.0" < "1.9.0" as strings, which would
        // tell someone on 1.9.0 that they are already up to date forever.
        assert!(v("1.10.0") > v("1.9.0"));
        assert!(v("2.0.0") > v("1.99.99"));
        assert!(v("1.0.1") > v("1.0.0"));
    }

    #[test]
    fn a_release_is_newer_than_its_own_candidates() {
        // Otherwise `gramit update` would offer an rc to someone on the final build.
        assert!(v("1.2.0") > v("1.2.0-rc1"));
        assert!(v("1.2.0-rc2") > v("1.2.0-rc1"));
        assert!(v("1.2.0-rc1") < v("1.2.1"));
    }

    #[test]
    fn the_current_build_is_a_version_update_can_compare() {
        // A version this cannot parse would make `gramit update` fail on every run,
        // and the workspace manifest is the only place it comes from.
        let current: Version = CURRENT.parse().expect("the crate version must parse");
        assert!(current >= v("0.0.1"));
    }

    #[test]
    fn rejects_something_that_is_not_a_version() {
        assert!("latest".parse::<Version>().is_err());
        assert!("1.2".parse::<Version>().is_err());
        assert!("1.2.3.4".parse::<Version>().is_err());
        assert!("1.2.x".parse::<Version>().is_err());
    }

    #[test]
    fn the_installer_matches_the_platform_it_will_run_on() {
        let url = installer_url();
        if cfg!(windows) {
            assert!(url.ends_with("install.ps1"), "{url}");
        } else {
            assert!(url.ends_with("install.sh"), "{url}");
        }
        assert!(url.starts_with("https://"), "{url}");
    }

    #[test]
    fn every_release_url_points_at_this_repository() {
        // A typo here would send someone else's binaries to a user's machine.
        assert!(latest_release_url().contains(REPO));
        assert!(installer_url().contains(REPO));
    }
}
