//! Runtime configuration, read from environment variables.
//!
//! See `docs/CONFIGURATION.md` for the full reference.

use std::time::Duration;

use thiserror::Error;
use url::Url;

/// How write operations are confirmed before they are sent to VinylDNS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationMode {
    /// Ask the human through MCP elicitation when the client supports it;
    /// otherwise rely on the client's own tool-call approval for `confirm_change`.
    Auto,
    /// Always require an elicitation answer. Clients without elicitation
    /// support cannot apply changes.
    Elicit,
    /// Never elicit; the `confirm_change` call itself is the confirmation.
    Token,
}

impl std::str::FromStr for ConfirmationMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "elicit" => Ok(Self::Elicit),
            "token" => Ok(Self::Token),
            other => Err(format!("expected one of auto, elicit, token; got '{other}'")),
        }
    }
}

#[derive(Clone)]
pub struct Config {
    /// Base URL of the VinylDNS API, e.g. `https://vinyldns.example.com:9443`.
    pub api_url: Url,
    pub access_key: String,
    pub secret_key: String,
    /// Credential-scope region; VinylDNS does not check it.
    pub signing_region: String,
    /// Credential-scope service name; VinylDNS does not check it.
    pub signing_service: String,
    pub http_timeout: Duration,
    /// Write tools are only registered when this is true.
    pub enable_writes: bool,
    pub confirmation: ConfirmationMode,
    /// How long a planned change stays confirmable.
    pub pending_ttl: Duration,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("api_url", &self.api_url.as_str())
            .field("access_key", &self.access_key)
            .field("secret_key", &"<redacted>")
            .field("signing_region", &self.signing_region)
            .field("signing_service", &self.signing_service)
            .field("http_timeout", &self.http_timeout)
            .field("enable_writes", &self.enable_writes)
            .field("confirmation", &self.confirmation)
            .field("pending_ttl", &self.pending_ttl)
            .finish()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("required environment variable {0} is not set")]
    Missing(&'static str),
    #[error("environment variable {name} is invalid: {reason}")]
    Invalid { name: &'static str, reason: String },
}

impl Config {
    /// Reads configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads configuration through `lookup`, which makes it testable without
    /// touching the process environment.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let get = |name: &str| lookup(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let required = |name: &'static str| get(name).ok_or(ConfigError::Missing(name));

        let raw_url = required("VINYLDNS_API_URL")?;
        let api_url = Url::parse(&raw_url).map_err(|e| ConfigError::Invalid {
            name: "VINYLDNS_API_URL",
            reason: e.to_string(),
        })?;
        if !matches!(api_url.scheme(), "http" | "https") {
            return Err(ConfigError::Invalid {
                name: "VINYLDNS_API_URL",
                reason: "scheme must be http or https".into(),
            });
        }

        let parse_bool = |name: &'static str, default: bool| -> Result<bool, ConfigError> {
            match get(name) {
                None => Ok(default),
                Some(v) => match v.to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" | "on" => Ok(true),
                    "0" | "false" | "no" | "off" => Ok(false),
                    _ => Err(ConfigError::Invalid {
                        name,
                        reason: format!("expected true/false, got '{v}'"),
                    }),
                },
            }
        };
        let parse_secs = |name: &'static str, default: u64| -> Result<Duration, ConfigError> {
            match get(name) {
                None => Ok(Duration::from_secs(default)),
                Some(v) => match v.parse::<u64>() {
                    Ok(n) if n > 0 => Ok(Duration::from_secs(n)),
                    _ => Err(ConfigError::Invalid {
                        name,
                        reason: format!("expected a positive integer, got '{v}'"),
                    }),
                },
            }
        };

        let confirmation = match get("VINYLDNS_MCP_CONFIRMATION") {
            None => ConfirmationMode::Auto,
            Some(v) => v.parse().map_err(|reason| ConfigError::Invalid {
                name: "VINYLDNS_MCP_CONFIRMATION",
                reason,
            })?,
        };

        Ok(Self {
            api_url,
            access_key: required("VINYLDNS_ACCESS_KEY")?,
            secret_key: required("VINYLDNS_SECRET_KEY")?,
            signing_region: get("VINYLDNS_SIGNING_REGION").unwrap_or_else(|| "us-east-1".into()),
            signing_service: get("VINYLDNS_SIGNING_SERVICE").unwrap_or_else(|| "VinylDNS".into()),
            http_timeout: parse_secs("VINYLDNS_HTTP_TIMEOUT_SECS", 30)?,
            enable_writes: parse_bool("VINYLDNS_MCP_ENABLE_WRITES", false)?,
            confirmation,
            pending_ttl: parse_secs("VINYLDNS_MCP_PENDING_TTL_SECS", 600)?,
        })
    }

    /// True when credentials would cross the network unencrypted.
    pub fn is_insecure_remote(&self) -> bool {
        self.api_url.scheme() == "http"
            && !matches!(
                self.api_url.host_str(),
                Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| map.get(k).cloned()
    }

    const BASE: &[(&str, &str)] = &[
        ("VINYLDNS_API_URL", "http://localhost:9000"),
        ("VINYLDNS_ACCESS_KEY", "ak"),
        ("VINYLDNS_SECRET_KEY", "sk"),
    ];

    #[test]
    fn defaults_are_read_only_and_auto_confirmation() {
        let cfg = Config::from_lookup(env(BASE)).unwrap();
        assert!(!cfg.enable_writes);
        assert_eq!(cfg.confirmation, ConfirmationMode::Auto);
        assert_eq!(cfg.pending_ttl, Duration::from_secs(600));
        assert_eq!(cfg.signing_service, "VinylDNS");
        assert!(!cfg.is_insecure_remote());
    }

    #[test]
    fn missing_credentials_are_reported() {
        let err = Config::from_lookup(env(&[("VINYLDNS_API_URL", "http://x")])).unwrap_err();
        assert_eq!(err, ConfigError::Missing("VINYLDNS_ACCESS_KEY"));
    }

    #[test]
    fn parses_overrides_and_rejects_bad_values() {
        let mut pairs = BASE.to_vec();
        pairs.extend([
            ("VINYLDNS_MCP_ENABLE_WRITES", "true"),
            ("VINYLDNS_MCP_CONFIRMATION", "elicit"),
            ("VINYLDNS_API_URL", "http://dns.internal:9000"),
        ]);
        let cfg = Config::from_lookup(env(&pairs)).unwrap();
        assert!(cfg.enable_writes);
        assert_eq!(cfg.confirmation, ConfirmationMode::Elicit);
        assert!(cfg.is_insecure_remote());

        pairs.push(("VINYLDNS_MCP_PENDING_TTL_SECS", "0"));
        assert!(matches!(
            Config::from_lookup(env(&pairs)),
            Err(ConfigError::Invalid {
                name: "VINYLDNS_MCP_PENDING_TTL_SECS",
                ..
            })
        ));
    }

    #[test]
    fn debug_redacts_secret() {
        let cfg = Config::from_lookup(env(BASE)).unwrap();
        assert!(!format!("{cfg:?}").contains("\"sk\""));
    }
}
