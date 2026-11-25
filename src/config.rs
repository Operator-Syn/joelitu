//! Deployment configuration, and the only module that reads the environment.
//!
//! The split matters for testability: [`Config::from_lookup`] takes a lookup
//! closure and holds all the parsing logic, while [`Config::load`] is a three-line
//! wrapper over `std::env::var`. Tests therefore exercise every rule without
//! mutating the process environment — which is `unsafe` under edition 2024 and
//! racy under any edition.

use crate::error::ConfigError;

/// Environment variable holding the bot token.
const TOKEN_KEY: &str = "DISCORD_TOKEN";

/// Environment variable holding an optional guild id for development.
const TEST_GUILD_KEY: &str = "TEST_GUILD_ID";

/// Everything the process needs that can differ between deployments.
///
/// Deliberately small. Domain limits (message count, pacing floor) are constants
/// in [`crate::spam::plan`], not fields here, because an interval floor that an
/// environment variable can set to zero is a way to get the bot rate-limited.
#[derive(Debug, Clone)]
pub struct Config {
    /// Bot token. Never logged; no `Display` or `Serialize` impl exists for this
    /// type, so the only way to leak it is to print the field deliberately.
    pub token: String,
    /// When present, slash commands are registered against this guild instead of
    /// globally. Guild registration appears immediately; global registration can
    /// take an hour, which makes development painful.
    pub test_guild_id: Option<u64>,
}

impl Config {
    /// Read configuration from the process environment.
    ///
    /// # Errors
    ///
    /// Any [`ConfigError`] from [`Config::from_lookup`].
    pub fn load() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Build a `Config` from an arbitrary key lookup, applying validation.
    ///
    /// # Arguments
    ///
    /// * `lookup` - Returns the value for a key, or `None` when absent. An empty
    ///   string is treated as absent for the token, because an empty token is
    ///   never usable.
    ///
    /// # Returns
    ///
    /// A validated `Config`.
    ///
    /// # Errors
    ///
    /// * [`ConfigError::Missing`] when `DISCORD_TOKEN` is absent or empty.
    /// * [`ConfigError::Invalid`] when `TEST_GUILD_ID` is present but does not
    ///   parse as a `u64`.
    ///
    /// # Processing
    ///
    /// 1. Require a non-empty token.
    /// 2. Parse the guild id when present, treating an empty value as absent.
    pub fn from_lookup<F>(lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        // 1. The token has no sensible default; an empty one is rejected at
        //    startup rather than producing a confusing 401 on first use.
        let token = match lookup(TOKEN_KEY) {
            Some(token) if !token.trim().is_empty() => token,
            _ => return Err(ConfigError::Missing { name: TOKEN_KEY }),
        };

        // 2. The guild id is optional, but a malformed one is an error rather
        //    than silently ignored: a typo would otherwise register globally and
        //    look like an hour-long propagation delay.
        let test_guild_id = match lookup(TEST_GUILD_KEY) {
            None => None,
            Some(value) if value.trim().is_empty() => None,
            Some(value) => Some(
                value
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| ConfigError::Invalid {
                        name: TEST_GUILD_KEY,
                        value,
                    })?,
            ),
        };

        Ok(Self {
            token,
            test_guild_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a lookup over a fixed pair of values, so tests need no environment.
    fn lookup(token: Option<&str>, guild: Option<&str>) -> impl Fn(&str) -> Option<String> {
        let token = token.map(str::to_owned);
        let guild = guild.map(str::to_owned);

        move |key| match key {
            TOKEN_KEY => token.clone(),
            TEST_GUILD_KEY => guild.clone(),
            _ => None,
        }
    }

    #[test]
    fn a_token_alone_is_enough() {
        let config = Config::from_lookup(lookup(Some("abc"), None)).expect("token is present");

        assert_eq!(config.token, "abc");
        assert_eq!(config.test_guild_id, None);
    }

    #[test]
    fn a_missing_token_is_named_in_the_error() {
        let error = Config::from_lookup(lookup(None, None)).expect_err("token is required");

        assert!(matches!(error, ConfigError::Missing { name: TOKEN_KEY }));
        assert!(error.to_string().contains(TOKEN_KEY));
    }

    #[test]
    fn an_empty_token_counts_as_missing() {
        let error = Config::from_lookup(lookup(Some("   "), None)).expect_err("blank token");

        assert!(matches!(error, ConfigError::Missing { name: TOKEN_KEY }));
    }

    #[test]
    fn a_valid_guild_id_is_parsed() {
        let config =
            Config::from_lookup(lookup(Some("abc"), Some("123456789"))).expect("guild id parses");

        assert_eq!(config.test_guild_id, Some(123_456_789));
    }

    #[test]
    fn a_malformed_guild_id_is_rejected_rather_than_ignored() {
        let error =
            Config::from_lookup(lookup(Some("abc"), Some("not-a-number"))).expect_err("bad id");

        assert!(matches!(
            error,
            ConfigError::Invalid {
                name: TEST_GUILD_KEY,
                ..
            }
        ));
    }

    #[test]
    fn an_empty_guild_id_is_treated_as_absent() {
        let config = Config::from_lookup(lookup(Some("abc"), Some(""))).expect("empty is absent");

        assert_eq!(config.test_guild_id, None);
    }
}
