//! `gramit config get/set/path`.
//!
//! Get and set round-trip through TOML rather than matching on field names, so the
//! config struct stays the single source of truth: a new setting works here the day
//! it is added, and `deny_unknown_fields` rejects typos with a real message.

use anyhow::{anyhow, Context, Result};
use gramit_core::{paths, Config};

use crate::ui;

pub fn path() -> Result<()> {
    println!("{}", paths::config_path().map_err(|err| anyhow!(err))?.display());
    Ok(())
}

pub fn get(key: Option<String>) -> Result<()> {
    let config = Config::load().map_err(|err| anyhow!(err))?;
    let table = to_table(&config)?;

    match key {
        None => {
            for (name, value) in &table {
                println!("{name} = {}", render(value));
            }
        }
        Some(name) => {
            let value = table
                .get(&name)
                .ok_or_else(|| anyhow!("unknown setting {name:?}\n{}", known_keys(&table)))?;
            println!("{}", render(value));
        }
    }
    Ok(())
}

pub fn set(key: String, value: String) -> Result<()> {
    // `backend_url` is the one setting people paste a bare host into, so accept that
    // here the same way `gramit setup` does instead of failing validation on it.
    let value = if key == "backend_url" {
        gramit_core::config::normalize_backend_url(&value)
    } else {
        value
    };

    let config = Config::load().map_err(|err| anyhow!(err))?;
    let mut table = to_table(&config)?;

    if !table.contains_key(&key) {
        return Err(anyhow!("unknown setting {key:?}\n{}", known_keys(&table)));
    }

    table.insert(key.clone(), parse_scalar(&value));

    let updated: Config = toml::Value::Table(table)
        .try_into()
        .with_context(|| format!("{value:?} is not valid for {key}"))?;
    updated.validate().map_err(|err| anyhow!(err))?;

    let written = updated.save().map_err(|err| anyhow!(err))?;
    println!("{}", ui::ok(&format!("{key} = {value}")));
    println!("{}", ui::detail(&format!("saved to {}", written.display())));

    // `gramit mode` saves and restarts in one step, so point at it rather than leave
    // someone to discover that their new mode did nothing until the next restart.
    if key == "mode" {
        println!("{}", ui::detail("apply it now with: gramit mode <name>"));
    } else {
        println!("{}", ui::detail("restart the daemon to apply: gramit restart"));
    }
    Ok(())
}

fn to_table(config: &Config) -> Result<toml::map::Map<String, toml::Value>> {
    match toml::Value::try_from(config).context("could not serialize the config")? {
        toml::Value::Table(table) => Ok(table),
        other => Err(anyhow!("config did not serialize to a table, got {other:?}")),
    }
}

/// Interprets the raw argument as a bool or integer where possible, so
/// `notifications false` and `max_chars 500` do the obvious thing.
fn parse_scalar(raw: &str) -> toml::Value {
    let trimmed = raw.trim();

    if let Ok(value) = trimmed.parse::<bool>() {
        return toml::Value::Boolean(value);
    }
    if let Ok(value) = trimmed.parse::<i64>() {
        return toml::Value::Integer(value);
    }
    toml::Value::String(trimmed.to_string())
}

fn render(value: &toml::Value) -> String {
    match value {
        toml::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn known_keys(table: &toml::map::Map<String, toml::Value>) -> String {
    let names: Vec<&str> = table.keys().map(String::as_str).collect();
    format!("known settings: {}", names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_booleans_and_integers() {
        assert_eq!(parse_scalar("true"), toml::Value::Boolean(true));
        assert_eq!(parse_scalar(" false "), toml::Value::Boolean(false));
        assert_eq!(parse_scalar("8000"), toml::Value::Integer(8000));
    }

    #[test]
    fn leaves_everything_else_as_a_string() {
        assert_eq!(parse_scalar("Ctrl+Alt+F"), toml::Value::String("Ctrl+Alt+F".into()));
        // A URL must not be mistaken for a number even though it contains digits.
        assert_eq!(
            parse_scalar("http://127.0.0.1:8787"),
            toml::Value::String("http://127.0.0.1:8787".into())
        );
    }

    #[test]
    fn every_config_field_is_reachable() {
        let table = to_table(&Config::default()).unwrap();
        for expected in [
            "hotkey",
            "backend_url",
            "mode",
            "notifications",
            "max_chars",
            "request_timeout_ms",
            "modifier_release_ms",
            "copy_settle_ms",
            "paste_delay_ms",
            "restore_delay_ms",
        ] {
            assert!(table.contains_key(expected), "{expected} is not settable");
        }
    }

    #[test]
    fn a_bad_value_is_rejected_by_the_type() {
        let mut table = to_table(&Config::default()).unwrap();
        table.insert("max_chars".into(), parse_scalar("not-a-number"));
        assert!(toml::Value::Table(table).try_into::<Config>().is_err());
    }

    #[test]
    fn a_valid_change_round_trips() {
        let mut table = to_table(&Config::default()).unwrap();
        table.insert("max_chars".into(), parse_scalar("500"));
        let config: Config = toml::Value::Table(table).try_into().unwrap();
        assert_eq!(config.max_chars, 500);
    }

    #[test]
    fn validation_still_applies_after_a_set() {
        let mut table = to_table(&Config::default()).unwrap();
        table.insert("max_chars".into(), parse_scalar("0"));
        let config: Config = toml::Value::Table(table).try_into().unwrap();
        assert!(config.validate().is_err(), "max_chars = 0 must be refused");
    }

    #[test]
    fn a_bare_host_is_normalized_before_it_is_stored() {
        // What `set` does to the value before it reaches the table.
        assert_eq!(
            gramit_core::config::normalize_backend_url("example.vercel.app"),
            "https://example.vercel.app"
        );
    }

    #[test]
    fn renders_strings_without_quotes() {
        assert_eq!(render(&toml::Value::String("code".into())), "code");
        assert_eq!(render(&toml::Value::Integer(42)), "42");
    }
}
