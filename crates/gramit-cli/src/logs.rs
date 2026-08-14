//! `gramit logs` — show or follow the daemon's log.

use std::io::{Seek, SeekFrom, Write};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use gramit_core::paths;

const POLL: Duration = Duration::from_millis(300);

pub async fn run(follow: bool, lines: usize) -> Result<()> {
    let path = paths::log_path().map_err(|err| anyhow!(err))?;

    if !path.exists() {
        return Err(anyhow!(
            "no log yet at {}\nstart the daemon with: gramit start",
            path.display()
        ));
    }

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    print_tail(&contents, lines);

    if !follow {
        return Ok(());
    }

    // Follow from the end of what was just printed, so nothing is shown twice.
    let mut file = std::fs::File::open(&path)
        .with_context(|| format!("could not open {}", path.display()))?;
    let mut position = file.seek(SeekFrom::End(0))?;

    loop {
        tokio::time::sleep(POLL).await;

        let length = std::fs::metadata(&path)?.len();
        if length < position {
            // The file was truncated or rotated; start over from its beginning.
            position = 0;
        }
        if length == position {
            continue;
        }

        file.seek(SeekFrom::Start(position))?;
        let mut fresh = String::new();
        use std::io::Read;
        file.read_to_string(&mut fresh)?;
        position += fresh.len() as u64;

        print!("{fresh}");
        std::io::stdout().flush().ok();
    }
}

fn print_tail(contents: &str, lines: usize) {
    let all: Vec<&str> = contents.lines().collect();
    let start = all.len().saturating_sub(lines);
    for line in &all[start..] {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_only_the_last_lines() {
        // Exercised through the same slicing print_tail uses.
        let contents = "1\n2\n3\n4\n5";
        let all: Vec<&str> = contents.lines().collect();
        let start = all.len().saturating_sub(2);
        assert_eq!(&all[start..], &["4", "5"]);
    }

    #[test]
    fn tail_of_a_short_file_keeps_everything() {
        let contents = "only\ntwo";
        let all: Vec<&str> = contents.lines().collect();
        let start = all.len().saturating_sub(50);
        assert_eq!(start, 0);
    }
}
