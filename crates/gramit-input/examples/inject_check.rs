//! Isolates keystroke injection from the hotkey.
//!
//!     cargo run -p gramit-input --example inject_check
//!
//! Gives you time to select text with no keys held, then injects a single Ctrl+C and
//! reports whether — and how long after — anything reached the clipboard. It never
//! pastes, so the worst it can do is copy something.

use std::time::{Duration, Instant};

use gramit_input::clipboard::{ArboardClipboard, Clipboard};
use gramit_input::injector;

const COUNTDOWN: u64 = 6;
const POLL_LIMIT: Duration = Duration::from_secs(3);

#[tokio::main]
async fn main() {
    let clipboard = match ArboardClipboard::new() {
        Ok(clipboard) => clipboard,
        Err(err) => {
            println!("FAILED to open the clipboard: {err}");
            return;
        }
    };

    let injector = match injector::open().await {
        Ok(injector) => injector,
        Err(err) => {
            println!("FAILED to open the injector [{}]: {err}", err.code());
            return;
        }
    };
    println!("injector: {}\n", injector.describe());

    let before = clipboard.get_text().await.unwrap_or(None);
    println!("clipboard right now: {before:?}\n");

    println!("Do this in the next {COUNTDOWN} seconds:");
    println!("  1. click into a text field (browser, editor, chat)");
    println!("  2. type a few words");
    println!("  3. select them with Ctrl+A");
    println!("  4. LET GO OF EVERY KEY and leave that window focused\n");

    for remaining in (1..=COUNTDOWN).rev() {
        println!("  ...{remaining}");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    println!("\nclearing the clipboard");
    if let Err(err) = clipboard.clear().await {
        println!("FAILED to clear the clipboard: {err}");
        return;
    }

    println!("injecting Ctrl+C");
    let sent_at = Instant::now();
    if let Err(err) = injector.copy().await {
        println!("FAILED to inject [{}]: {err}", err.code());
        return;
    }

    // Poll well past the daemon's 400ms window, so a slow-but-working clipboard shows
    // up as a timing problem rather than a failure.
    loop {
        match clipboard.get_text().await {
            Ok(Some(text)) if !text.is_empty() => {
                let elapsed = sent_at.elapsed();
                println!("\nCAPTURED after {}ms: {text:?}", elapsed.as_millis());
                println!("\n=> Injection works.");
                if elapsed > Duration::from_millis(400) {
                    println!(
                        "   But it took longer than the daemon's 400ms copy_settle_ms.\n\
                            Fix with: gramit config set copy_settle_ms {} && gramit restart",
                        (elapsed.as_millis() as u64 / 100 + 3) * 100
                    );
                }
                return;
            }
            Ok(_) => {}
            Err(err) => {
                println!("FAILED to read the clipboard: {err}");
                return;
            }
        }

        if sent_at.elapsed() >= POLL_LIMIT {
            println!("\nNOTHING CAPTURED after {}s.", POLL_LIMIT.as_secs());
            println!(
                "\n=> The injected Ctrl+C did not reach the focused window, or the app\n\
                    did not put anything on the clipboard.\n\n\
                 Check, in order:\n\
                 - Did you actually have text selected in a *focused* window?\n\
                 - Does Ctrl+C copy in that app when you press it yourself?\n\
                 - Try a different app (GNOME Text Editor vs Chrome vs a terminal)."
            );
            return;
        }

        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
