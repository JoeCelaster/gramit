//! `gramit mode` — reads or changes what pressing the hotkey means.
//!
//! The hotkey carries no argument, so the mode is a setting rather than a per-fix
//! choice. That makes switching it a two-step job: write the config, then get the
//! running daemon to read it. Doing only the first is the trap this module exists to
//! avoid — the setting would look changed while every fix kept using the old mode.

use std::io::{IsTerminal, Write};

use anyhow::{anyhow, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, execute, terminal};
use gramit_core::config::Mode;
use gramit_core::Config;

use crate::client as ipc;
use crate::lifecycle;
use crate::ui;

/// `gramit mode [name]`.
pub async fn run(requested: Option<Mode>) -> Result<()> {
    let config = Config::load().map_err(|err| anyhow!(err))?;

    match requested {
        None => {
            show(config.mode);
            Ok(())
        }
        Some(mode) => apply(mode, config).await,
    }
}

/// Offers the choice on `gramit start`, defaulting to whatever is saved.
///
/// Runs before the daemon is spawned, so the answer is on disk by the time it reads
/// the config — no restart needed for this one path. Silent when there is nobody to
/// ask, which keeps autostart entries and scripted starts working.
pub async fn prompt_on_start() -> Result<()> {
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return Ok(());
    }
    // `gramit start` on a running daemon only prints status, so a mode saved here
    // would not take effect. Say nothing rather than appear to change something.
    if ipc::is_running().await {
        return Ok(());
    }

    let mut config = Config::load().map_err(|err| anyhow!(err))?;
    let current = config.mode;

    let Some(chosen) = ask(current)? else {
        return Ok(());
    };
    if chosen == current {
        println!("{}", ui::detail(&format!("mode: {current}")));
        return Ok(());
    }

    config.mode = chosen;
    config.save().map_err(|err| anyhow!(err))?;
    println!("{}", ui::ok(&format!("mode = {chosen}")));
    Ok(())
}

fn show(current: Mode) {
    println!("{}", ui::ok(&format!("mode = {current}")));
    println!("{}", ui::detail(current.summary()));
    println!();
    for mode in Mode::ALL {
        if mode != current {
            println!("{}", ui::detail(&format!("switch with: gramit mode {mode}")));
        }
    }
}

/// Saves the mode, then makes it real. A daemon that is already up holds the old mode
/// in memory, so it is restarted; there is nothing to restart if it is down.
async fn apply(mode: Mode, mut config: Config) -> Result<()> {
    if config.mode == mode {
        println!("{}", ui::ok(&format!("already in {mode} mode")));
        println!("{}", ui::detail(mode.summary()));
        return Ok(());
    }

    config.mode = mode;
    let written = config.save().map_err(|err| anyhow!(err))?;
    println!("{}", ui::ok(&format!("mode = {mode}")));
    println!("{}", ui::detail(mode.summary()));
    println!("{}", ui::detail(&format!("saved to {}", written.display())));

    if ipc::is_running().await {
        println!("{}", ui::detail("restarting gramit to apply it ..."));
        lifecycle::restart().await?;
    }
    Ok(())
}

/// Returns `None` when the user declined to choose, which means "keep what I have".
fn ask(current: Mode) -> Result<Option<Mode>> {
    // Raw mode is what makes arrow keys readable at all. Where it cannot be turned on
    // — a dumb terminal, an odd pty, a shell that has stdin on something exotic —
    // fall back to typing the answer rather than losing the prompt entirely.
    if terminal::enable_raw_mode().is_err() {
        return ask_typed(current);
    }
    let _restore = RawMode;
    ask_with_arrows(current)
}

/// Puts the terminal back however the picker exits: a normal return, an error, or a
/// panic unwinding through it. A shell left in raw mode is unusable.
struct RawMode;

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(std::io::stdout(), cursor::Show);
    }
}

fn ask_with_arrows(current: Mode) -> Result<Option<Mode>> {
    let mut out = std::io::stdout();
    let mut selected = Mode::ALL.iter().position(|mode| *mode == current).unwrap_or(0);

    // Raw mode means a bare newline moves down without returning to column 0, so every
    // line ends "\r\n" from here until the guard restores the terminal.
    write!(out, "What should gramit do with the text you select?\r\n")?;
    write!(out, "{}\r\n\r\n", ui::dim("  ↑/↓ to move, Enter to choose, Esc to keep the current one"))?;
    execute!(out, cursor::Hide)?;
    draw(&mut out, selected, false)?;

    loop {
        let Event::Key(key) = event::read().context("could not read a key")? else {
            continue;
        };
        // Windows reports press and release; without this every key counts twice.
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            // Checked before the plain characters below, so Ctrl-C never reads as a
            // letter. It aborts the whole command, the way it would anywhere else.
            KeyCode::Char('c' | 'd') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                write!(out, "\r\n")?;
                return Err(anyhow!("cancelled"));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                selected = (selected + Mode::ALL.len() - 1) % Mode::ALL.len();
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                selected = (selected + 1) % Mode::ALL.len();
            }
            // The list is short and numbered nowhere, but a digit is a natural reach.
            KeyCode::Char(digit @ '1'..='9') => {
                let index = digit as usize - '1' as usize;
                if index >= Mode::ALL.len() {
                    continue;
                }
                selected = index;
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                draw(&mut out, selected, true)?;
                write!(out, "\r\n")?;
                return Ok(Some(Mode::ALL[selected]));
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                draw(&mut out, Mode::ALL.iter().position(|m| *m == current).unwrap_or(0), true)?;
                write!(out, "\r\n")?;
                return Ok(None);
            }
            _ => continue,
        }
        draw(&mut out, selected, true)?;
    }
}

/// Paints the option list. After the first pass the cursor sits below the list, so a
/// repaint steps back over exactly the lines it wrote and overwrites them in place.
fn draw(out: &mut impl Write, selected: usize, repaint: bool) -> Result<()> {
    if repaint {
        execute!(out, cursor::MoveToPreviousLine(Mode::ALL.len() as u16))?;
    }
    for (index, mode) in Mode::ALL.iter().enumerate() {
        execute!(out, Clear(ClearType::CurrentLine))?;
        let line = format!("{mode:<8} {}", mode.summary());
        if index == selected {
            write!(out, "  {} {}\r\n", ui::green("›"), ui::bold(&line))?;
        } else {
            write!(out, "    {}\r\n", ui::dim(&line))?;
        }
    }
    out.flush()?;
    Ok(())
}

/// The prompt for terminals that cannot go into raw mode. Same question, typed answer.
fn ask_typed(current: Mode) -> Result<Option<Mode>> {
    println!("What should gramit do with the text you select?");
    println!();
    for (index, mode) in Mode::ALL.iter().enumerate() {
        let marker = if *mode == current { "*" } else { " " };
        println!(
            "{}",
            ui::detail(&format!("{marker} {}) {mode:<8} {}", index + 1, mode.summary())),
        );
    }
    println!();

    print!("  Mode [{current}]: ");
    std::io::stdout().flush().ok();

    let mut line = String::new();
    let read = std::io::stdin()
        .read_line(&mut line)
        .context("could not read the mode")?;

    // Zero bytes is Ctrl-D, not an answer. Keep the saved mode and get on with it.
    if read == 0 {
        println!();
        return Ok(None);
    }
    parse_answer(&line)
}

/// Accepts what the prompt showed: a menu number, a mode name, its first letter, or
/// nothing at all.
fn parse_answer(line: &str) -> Result<Option<Mode>> {
    let answer = line.trim();
    if answer.is_empty() {
        return Ok(None);
    }
    if let Ok(index) = answer.parse::<usize>() {
        return Mode::ALL
            .get(index.wrapping_sub(1))
            .copied()
            .map(Some)
            .ok_or_else(|| anyhow!("there is no mode {index}"));
    }
    answer.parse::<Mode>().map(Some).map_err(|err| anyhow!(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_enter_keeps_the_current_mode() {
        assert_eq!(parse_answer("\n").unwrap(), None);
        assert_eq!(parse_answer("   ").unwrap(), None);
    }

    #[test]
    fn the_menu_numbers_match_what_was_printed() {
        assert_eq!(parse_answer("1").unwrap(), Some(Mode::Grammar));
        assert_eq!(parse_answer("2").unwrap(), Some(Mode::Write));
        assert_eq!(parse_answer("3").unwrap(), Some(Mode::Code));
    }

    #[test]
    fn names_and_initials_both_work() {
        assert_eq!(parse_answer("code").unwrap(), Some(Mode::Code));
        assert_eq!(parse_answer(" GRAMMAR \n").unwrap(), Some(Mode::Grammar));
        assert_eq!(parse_answer("g").unwrap(), Some(Mode::Grammar));
        assert_eq!(parse_answer("write").unwrap(), Some(Mode::Write));
        assert_eq!(parse_answer("w").unwrap(), Some(Mode::Write));
    }

    #[test]
    fn an_answer_that_is_not_a_mode_is_an_error() {
        // Silently falling back would start the daemon in a mode the user did not pick.
        assert!(parse_answer("0").is_err());
        assert!(parse_answer("4").is_err());
        assert!(parse_answer("sarcastic").is_err());
    }
}
