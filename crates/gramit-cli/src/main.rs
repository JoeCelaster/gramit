//! `gramit` — the command line interface.
//!
//! Thin by design: the daemon does the work, and this speaks to it over the local
//! socket. The one exception is `doctor`, which deliberately still says something
//! useful when the daemon is down.

mod client;
mod config_cmd;
mod doctor;
mod fix;
mod lifecycle;
mod logs;
mod ui;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use gramit_core::config::Mode;
use gramit_core::VERSION;

#[derive(Parser)]
#[command(
    name = "gramit",
    version = VERSION,
    about = "Fix the grammar of the text you have selected, anywhere",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the daemon
    Start {
        /// Run in this terminal instead of the background
        #[arg(short, long)]
        foreground: bool,
    },
    /// Stop the daemon
    Stop,
    /// Restart the daemon
    Restart,
    /// Show what the daemon is doing
    Status,
    /// Correct text, the clipboard, or the current selection
    Fix(FixArgs),
    /// Read or change settings
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Show the daemon log
    Logs {
        /// Keep printing new lines as they arrive
        #[arg(short, long)]
        follow: bool,
        /// How many lines of history to show
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
    },
    /// Check the setup and say how to fix whatever is broken
    Doctor {
        /// Apply the fixes that can be applied automatically
        #[arg(long)]
        fix: bool,
    },
}

#[derive(Args)]
struct FixArgs {
    /// Text to correct. Use `-`, or pipe input, to read stdin.
    text: Option<String>,

    /// Correct the clipboard in place
    #[arg(long, conflicts_with_all = ["selection", "text"])]
    clipboard: bool,

    /// Correct the current selection and paste it back
    #[arg(long, conflicts_with_all = ["clipboard", "text"])]
    selection: bool,

    /// Correction mode
    #[arg(long)]
    mode: Option<Mode>,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        // `{err:#}` prints the whole anyhow chain, which is where the remedies live.
        eprintln!("{} {err:#}", ui::red("error:"));
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Start { foreground } => lifecycle::start(foreground).await,
        Command::Stop => lifecycle::stop().await,
        Command::Restart => lifecycle::restart().await,
        Command::Status => lifecycle::status().await,
        Command::Fix(args) => {
            let target = if args.clipboard {
                fix::Target::Clipboard
            } else if args.selection {
                fix::Target::Selection
            } else {
                fix::Target::Text
            };
            fix::run(target, args.text, args.mode).await
        }
        Command::Config { command } => match command {
            ConfigCommand::Path => config_cmd::path(),
            ConfigCommand::Get { key } => config_cmd::get(key),
            ConfigCommand::Set { key, value } => config_cmd::set(key, value),
        },
        Command::Logs { follow, lines } => logs::run(follow, lines).await,
        Command::Doctor { fix } => {
            doctor::ensure_supported()?;
            doctor::run(fix).await
        }
    }
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print where the config file lives
    Path,
    /// Print one setting, or all of them
    Get { key: Option<String> },
    /// Change a setting
    Set { key: String, value: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn fix_accepts_bare_text() {
        let cli = Cli::try_parse_from(["gramit", "fix", "he go"]).unwrap();
        match cli.command {
            Command::Fix(args) => {
                assert_eq!(args.text.as_deref(), Some("he go"));
                assert!(!args.clipboard && !args.selection);
            }
            _ => panic!("expected fix"),
        }
    }

    #[test]
    fn fix_targets_are_mutually_exclusive() {
        assert!(Cli::try_parse_from(["gramit", "fix", "--clipboard", "--selection"]).is_err());
        assert!(Cli::try_parse_from(["gramit", "fix", "--clipboard", "some text"]).is_err());
    }

    #[test]
    fn fix_selection_takes_no_text() {
        let cli = Cli::try_parse_from(["gramit", "fix", "--selection"]).unwrap();
        match cli.command {
            Command::Fix(args) => assert!(args.selection && args.text.is_none()),
            _ => panic!("expected fix"),
        }
    }

    #[test]
    fn an_unknown_mode_is_rejected() {
        assert!(Cli::try_parse_from(["gramit", "fix", "hi", "--mode", "sarcastic"]).is_err());
        assert!(Cli::try_parse_from(["gramit", "fix", "hi", "--mode", "grammar"]).is_ok());
    }

    #[test]
    fn logs_defaults_to_fifty_lines() {
        let cli = Cli::try_parse_from(["gramit", "logs"]).unwrap();
        match cli.command {
            Command::Logs { follow, lines } => {
                assert!(!follow);
                assert_eq!(lines, 50);
            }
            _ => panic!("expected logs"),
        }
    }

    #[test]
    fn config_set_requires_both_arguments() {
        assert!(Cli::try_parse_from(["gramit", "config", "set", "max_chars"]).is_err());
        assert!(Cli::try_parse_from(["gramit", "config", "set", "max_chars", "500"]).is_ok());
    }

    #[test]
    fn config_get_allows_no_key() {
        assert!(Cli::try_parse_from(["gramit", "config", "get"]).is_ok());
    }
}
