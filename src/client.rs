//! Minimal signed HTTP client for the VinylDNS REST API.

use chrono::Utc;
use reqwest::{Method, StatusCode};
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::config::Config;
use crate::signer::{Signer, encode_component};

/// Query parameters; `None` values are omitted.
pub type Query<'a> = [(&'a str, Option<String>)];

#[derive(Debug, Error)]
pub enum ApiError {
    /// VinylDNS answered with a non-success status.
    #[error("VinylDNS returned HTTP {status}: {message}")]
    Status { status: StatusCode, message: String },
    /// The request never got a response (DNS, TLS, timeout, ...).
    #[error("could not reach VinylDNS: {0}")]
    Transport(#[from] reqwest::Error),
}

impl ApiError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Self::Status { status, .. } => Some(*status),
            Self::Transport(_) => None,
        }
    }

    /// A short hint for the model on how to recover from common failures.
    pub fn hint(&self) -> Option<&'static str> {
        match self.status()?.as_u16() {
            401 => {
                Some("The access key / secret key were rejected. Check VINYLDNS_ACCESS_KEY and VINYLDNS_SECRET_KEY.")
            }
            403 => Some("The VinylDNS user lacks permission for this action (zone admin group or ACL rule required)."),
            404 => Some(
                "The referenced zone, record set, group or batch change does not exist or is not visible to this user.",
            ),
            409 => Some(
                "A conflicting resource exists (e.g. a record set with the same name and type) or a change is already in progress.",
            ),
            422 => Some("VinylDNS rejected the input as invalid; read the message and fix the request."),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VinylDnsClient {
    http: reqwest::Client,
    base: Url,
    signer: Signer,
}

impl VinylDnsClient {
    pub fn new(config: &Config) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .timeout(config.http_timeout)
            .user_agent(concat!("vinyldns-mcp/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http,
            base: config.api_url.clone(),
            signer: Signer::new(
                &config.access_key,
                &config.secret_key,
                &config.signing_region,
                &config.signing_service,
            ),
        })
    }

    pub async fn get(&self, path: &str, query: &Query<'_>) -> Result<Value, ApiError> {
        self.send(Method::GET, path, query, None).await
    }

    pub async fn post(&self, path: &str, query: &Query<'_>, body: Option<&Value>) -> Result<Value, ApiError> {
        self.send(Method::POST, path, query, body).await
    }

    pub async fn put(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        self.send(Method::PUT, path, &[], Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> Result<Value, ApiError> {
        self.send(Method::DELETE, path, &[], None).await
    }

    /// Builds the full request URL. Path segments supplied by callers must
    /// already be encoded with [`segment`].
    pub fn url(&self, path: &str, query: &Query<'_>) -> Url {
        let mut url = self.base.clone();
        let joined = format!(
            "{}/{}",
            self.base.path().trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        url.set_path(&joined);
        let qs = query
            .iter()
            .filter_map(|(k, v)| {
                v.as_ref()
                    .map(|v| format!("{}={}", encode_component(k), encode_component(v)))
            })
            .collect::<Vec<_>>()
            .join("&");
        url.set_query(if qs.is_empty() { None } else { Some(&qs) });
        url
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        query: &Query<'_>,
        body: Option<&Value>,
    ) -> Result<Value, ApiError> {
        let url = self.url(path, query);
        let body_bytes = match body {
            Some(b) => serde_json::to_vec(b).expect("serde_json::Value always serializes"),
            None => Vec::new(),
        };
        let signed = self.signer.sign(method.as_str(), &url, &body_bytes, Utc::now());

        tracing::debug!(%method, %url, "calling VinylDNS");
        let mut req = self
            .http
            .request(method, url)
            .header("Authorization", signed.authorization)
            .header("X-Amz-Date", signed.x_amz_date)
            .header("Accept", "application/json");
        if body.is_some() {
            req = req.header("Content-Type", "application/json").body(body_bytes);
        }

        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            return Err(ApiError::Status {
                status,
                message: error_message(&text),
            });
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
}

/// Percent-encodes a value for use as one URL path segment.
pub fn segment(s: &str) -> String {
    encode_component(s)
}

/// VinylDNS errors are either plain text or `{"errors": [...]}`.
fn error_message(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "(empty response body)".into();
    }
    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(trimmed)
        && let Some(Value::Array(errors)) = obj.get("errors")
    {
        let msgs: Vec<String> = errors
            .iter()
            .map(|e| e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string()))
            .collect();
        if !msgs.is_empty() {
            return msgs.join("; ");
        }
    }
    trimmed.chars().take(4000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_error_lists() {
        assert_eq!(error_message(r#"{"errors":["a","b"]}"#), "a; b");
        assert_eq!(error_message("Zone not found"), "Zone not found");
        assert_eq!(error_message("  "), "(empty response body)");
    }

    #[test]
    fn segment_encodes_reserved_characters() {
        assert_eq!(segment("ok.zone."), "ok.zone.");
        assert_eq!(segment("a/b c"), "a%2Fb%20c");
    }
}
