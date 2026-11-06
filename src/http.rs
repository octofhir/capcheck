//! Requests to the server under test.

use reqwest::{Client, Method, StatusCode};
use serde_json::Value;

use crate::Error;

const FHIR_JSON: &str = "application/fhir+json";

/// What a request gave.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: StatusCode,
    pub headers: reqwest::header::HeaderMap,
    pub body: Value,
}

impl Answer {
    /// The first diagnostics of an OperationOutcome, to say why a request failed.
    #[must_use]
    pub fn reason(&self) -> String {
        let issue = &self.body["issue"][0];
        let text = issue["diagnostics"]
            .as_str()
            .or_else(|| issue["details"]["text"].as_str())
            .unwrap_or_default();
        let text: String = text.chars().take(200).collect();
        if text.is_empty() {
            self.status.to_string()
        } else {
            format!("{}: {text}", self.status)
        }
    }

    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)?.to_str().ok()
    }
}

/// The server under test.
#[derive(Debug, Clone)]
pub struct Server {
    client: Client,
    base: String,
    token: Option<String>,
}

impl Server {
    /// # Errors
    ///
    /// Fails if the HTTP client cannot be set up.
    pub fn new(base: &str, token: Option<String>) -> Result<Self, Error> {
        Ok(Self {
            client: Client::builder()
                .user_agent(concat!("capcheck/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(60))
                .build()?,
            base: base.trim_end_matches('/').to_owned(),
            token,
        })
    }

    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    /// A request at `path` under the base, with these headers, and a FHIR body if there is one.
    ///
    /// # Errors
    ///
    /// Fails if the server cannot be reached.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Answer, Error> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header("Accept", FHIR_JSON);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            if !headers.iter().any(|(name, _)| *name == "Content-Type") {
                request = request.header("Content-Type", FHIR_JSON);
            }
            request = request.body(body.to_string());
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().await?;
        let status = response.status();
        let headers = response.headers().clone();
        let text = response.text().await?;
        Ok(Answer {
            status,
            headers,
            body: serde_json::from_str(&text).unwrap_or(Value::Null),
        })
    }

    /// A GET at `path`.
    ///
    /// # Errors
    ///
    /// Fails if the server cannot be reached.
    pub async fn get(&self, path: &str) -> Result<Answer, Error> {
        self.request(Method::GET, path, &[], None).await
    }
}

/// Encodes a query value.
#[must_use]
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b':' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
