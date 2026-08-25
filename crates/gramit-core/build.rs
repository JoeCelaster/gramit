//! Bakes the backend address from `deploy.toml` into the crate.
//!
//! The address is deployment configuration, not a constant of the program, so it is
//! written down once — in the workspace `deploy.toml` — and read from there at build
//! time. That file is committed (it holds no secrets), which `.env` cannot be.
//! Nothing in `src/` may hardcode the address; `config::DEFAULT_BACKEND_URL` reads
//! the value this script emits.

use std::path::{Path, PathBuf};

const KEY: &str = "GRAMIT_BACKEND_URL";
const FILE: &str = "deploy.toml";

fn main() {
    // An explicit environment variable wins, so a packaging or CI job can build
    // against a different backend without editing a tracked file.
    println!("cargo::rerun-if-env-changed={KEY}");

    let value = match std::env::var(KEY) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => from_deploy_toml(),
    };

    if !(value.starts_with("http://") || value.starts_with("https://")) {
        panic!("backend url must start with http:// or https://, got {value:?}");
    }

    println!("cargo::rustc-env={KEY}={}", value.trim_end_matches('/'));
}

/// Reads `backend.url` out of the nearest `deploy.toml`, walking up from this crate.
fn from_deploy_toml() -> String {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("cargo always sets CARGO_MANIFEST_DIR"),
    );

    for dir in manifest_dir.ancestors() {
        let path = dir.join(FILE);
        // Watch every candidate, not just the one that matched: a deploy.toml added
        // closer to the crate later on should trigger a rebuild too.
        println!("cargo::rerun-if-changed={}", path.display());
        if let Some(value) = backend_url(&path) {
            return value;
        }
    }

    panic!(
        "no backend url found: set backend.url in the workspace {FILE}, or {KEY} in the \
         environment before building"
    );
}

fn backend_url(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let doc: toml::Value = toml::from_str(&text)
        .unwrap_or_else(|err| panic!("{} is not valid TOML: {err}", path.display()));

    let url = doc.get("backend").and_then(|backend| backend.get("url"));
    match url {
        Some(toml::Value::String(url)) if !url.trim().is_empty() => Some(url.trim().to_string()),
        Some(other) => panic!(
            "{}: backend.url must be a string, got {}",
            path.display(),
            other.type_str()
        ),
        None => panic!("{}: missing backend.url", path.display()),
    }
}
