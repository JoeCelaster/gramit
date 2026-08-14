//! Diagnostic for the Wayland portal path.
//!
//!     cargo run -p gramit-input --example portal_check
//!
//! Prints how far each portal gets. Both may show a GNOME permission dialog the
//! first time — approve them when they appear.

use std::time::Duration;

use gramit_input::{hotkey, injector};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();

    println!("\n=== 1. RemoteDesktop portal (keystroke injection) ===");
    println!("waiting up to 60s — approve the dialog if one appears...");
    match tokio::time::timeout(Duration::from_secs(60), injector::open()).await {
        Ok(Ok(injector)) => println!("OK: {}", injector.describe()),
        Ok(Err(err)) => println!("FAILED [{}]: {err}", err.code()),
        Err(_) => println!("TIMED OUT after 60s — the portal never answered"),
    }

    println!("\n=== 2. GlobalShortcuts portal (Ctrl+Alt+F) ===");
    println!("waiting up to 60s — approve the dialog if one appears...");
    match tokio::time::timeout(Duration::from_secs(60), hotkey::register("Ctrl+Alt+F")).await {
        Ok(Ok(registration)) => println!("OK: bound as {}", registration.description),
        Ok(Err(err)) => println!("FAILED [{}]: {err}", err.code()),
        Err(_) => println!("TIMED OUT after 60s — the portal never answered"),
    }

    println!();
}
