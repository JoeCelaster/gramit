//! `gramit fix` — correct text from an argument, stdin, the clipboard, or the
//! current selection.

use std::io::{IsTerminal, Read, Write};

use anyhow::{anyhow, Result};
use gramit_core::config::Mode;
use gramit_core::ipc::{Request, Response};

use crate::client;
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Correct the given text (or stdin) and print the result.
    Text,
    /// Correct the clipboard in place.
    Clipboard,
    /// Capture the selection, correct it, and paste it back.
    Selection,
}

pub async fn run(target: Target, text: Option<String>, mode: Option<Mode>) -> Result<()> {
    match target {
        Target::Text => fix_text(text, mode).await,
        Target::Clipboard => fix_clipboard().await,
        Target::Selection => fix_selection().await,
    }
}

async fn fix_text(text: Option<String>, mode: Option<Mode>) -> Result<()> {
    let input = match text {
        // `-` is the conventional "read stdin" argument.
        Some(value) if value != "-" => value,
        _ => read_stdin()?,
    };

    if input.trim().is_empty() {
        return Err(anyhow!("no text to correct (pass text, or pipe it in)"));
    }

    let request = Request::Fix { text: input, mode: mode.unwrap_or_default() };

    match client::request(request).await? {
        Response::Fixed { text, changed, changes, .. } => {
            // The corrected text is the only thing on stdout, so `gramit fix` composes
            // in a pipeline. Everything else goes to stderr.
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
            std::io::stdout().flush().ok();

            if changed {
                eprintln!("{}", ui::dim(&format!("{changes} correction(s)")));
            } else {
                eprintln!("{}", ui::dim("already correct"));
            }
            Ok(())
        }
        Response::Error { code, message, .. } => Err(anyhow!("{message} [{code}]")),
        other => Err(anyhow!("unexpected reply: {other:?}")),
    }
}

fn read_stdin() -> Result<String> {
    if std::io::stdin().is_terminal() {
        return Err(anyhow!(
            "no text given.\nUsage: gramit fix \"he go to the store\"  |  echo ... | gramit fix -"
        ));
    }
    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;
    Ok(buffer)
}

async fn fix_clipboard() -> Result<()> {
    // The daemon owns the clipboard: on X11 whoever sets it must stay alive to serve
    // it, and this process is about to exit.
    match client::request(Request::FixClipboard).await? {
        Response::Fixed { changed, changes, text, .. } => {
            if changed {
                println!("{}", ui::ok(&format!("clipboard fixed ({changes} correction(s))")));
                eprintln!("{}", ui::dim(&preview(&text)));
            } else {
                println!("{}", ui::ok("clipboard was already correct"));
            }
            Ok(())
        }
        Response::Error { code, message, .. } => Err(anyhow!("{message} [{code}]")),
        other => Err(anyhow!("unexpected reply: {other:?}")),
    }
}

async fn fix_selection() -> Result<()> {
    let result = client::request(Request::FixSelection).await;

    // This is what the desktop keybinding runs, so there is usually no terminal to
    // report to. The daemon notifies for outcomes it produces itself, but it cannot
    // notify when it is not running — so do that here.
    let response = match result {
        Ok(response) => response,
        Err(err) => {
            notify_if_headless("gramit is not running", &err.to_string());
            return Err(err);
        }
    };

    match response {
        Response::Fixed { changed, changes, .. } => {
            if changed {
                println!("{}", ui::ok(&format!("selection fixed ({changes} correction(s))")));
            } else {
                println!("{}", ui::ok("selection was already correct"));
            }
            Ok(())
        }
        Response::Error { code, message, .. } => Err(anyhow!("{message} [{code}]")),
        other => Err(anyhow!("unexpected reply: {other:?}")),
    }
}

/// Raises a desktop notification when there is no terminal to print to.
fn notify_if_headless(summary: &str, body: &str) {
    if std::io::stdout().is_terminal() {
        return;
    }
    let _ = notify_rust::Notification::new()
        .appname("gramit")
        .summary(summary)
        .body(body)
        .show();
}

fn preview(text: &str) -> String {
    const LIMIT: usize = 72;
    let single_line = text.replace('\n', " ");
    if single_line.chars().count() <= LIMIT {
        return single_line;
    }
    let truncated: String = single_line.chars().take(LIMIT).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_collapses_newlines() {
        assert_eq!(preview("one\ntwo"), "one two");
    }

    #[test]
    fn preview_truncates_long_text() {
        let long = "a".repeat(100);
        let result = preview(&long);
        assert!(result.ends_with('…'));
        assert_eq!(result.chars().count(), 73);
    }

    #[test]
    fn preview_counts_characters_not_bytes() {
        let text = "é".repeat(80);
        assert_eq!(preview(&text).chars().count(), 73);
    }
}
