//! Checks a FHIR server's CapabilityStatement against what the server really does.
//!
//! Each claim of the statement is put to the test: a declared interaction has to work, a
//! declared search parameter has to be known to the search, a declared operation has to be
//! there. What works without being declared is reported too. A run only reads, unless asked
//! to write.
//!
//! ```no_run
//! # async fn run() -> Result<(), capcheck::Error> {
//! let report = capcheck::check(&capcheck::Options::new("https://fhir.example.org/fhir")).await?;
//! println!("{}", report.text(false));
//! # Ok(())
//! # }
//! ```

mod claims;
mod http;
mod read;
mod report;
mod write;

pub use claims::Capability;
pub use report::{Finding, Report, Verdict};

use futures::stream::{self, StreamExt};
use reqwest::StatusCode;

use http::Server;

/// Why a run could not be done.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    Statement(String),
}

/// What to check, and how.
#[derive(Debug, Clone)]
pub struct Options {
    /// The FHIR base URL.
    pub base: String,
    /// A bearer token.
    pub token: Option<String>,
    /// Only these resource types; all when empty.
    pub types: Vec<String>,
    /// Requests under way at the same time.
    pub concurrency: usize,
    /// Also check writes, on Patient. They create and delete resources.
    pub write: bool,
}

impl Options {
    #[must_use]
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            token: None,
            types: Vec::new(),
            concurrency: 8,
            write: false,
        }
    }
}

/// Runs the checks.
///
/// # Errors
///
/// Fails if the server cannot be reached or has no CapabilityStatement to check.
pub async fn check(options: &Options) -> Result<Report, Error> {
    let server = Server::new(&options.base, options.token.clone())?;
    let metadata = server.get("/metadata").await?;
    if metadata.status != StatusCode::OK || metadata.body["resourceType"] != "CapabilityStatement" {
        return Err(Error::Statement(format!(
            "{}/metadata gave no CapabilityStatement: {}",
            server.base(),
            metadata.status
        )));
    }
    let capability: Capability = serde_json::from_value(metadata.body.clone())
        .map_err(|error| Error::Statement(format!("Unreadable CapabilityStatement: {error}")))?;

    let mut findings = read::statement(&server, &metadata).await;
    for rest in capability
        .rest
        .iter()
        .filter(|rest| rest.mode.as_deref() != Some("client"))
    {
        findings.extend(read::system(&server, rest).await?);
        let wanted = rest
            .resource
            .iter()
            .filter(|resource| options.types.is_empty() || options.types.contains(&resource.name));
        let results = stream::iter(wanted)
            .map(|resource| read::resource(&server, resource, options.concurrency))
            .buffer_unordered(options.concurrency)
            .collect::<Vec<_>>()
            .await;
        for result in results {
            findings.extend(result?);
        }
    }
    findings.extend(read::operations(&server, &capability).await?);
    if options.write {
        let run = format!("{:x}", std::process::id());
        findings.extend(write::patient_writes(&server, &capability, &run).await?);
    }
    findings
        .sort_by(|a, b| (a.verdict, &a.subject, &a.claim).cmp(&(b.verdict, &b.subject, &b.claim)));
    Ok(Report {
        base: server.base().to_owned(),
        fhir_version: capability.fhir_version,
        findings,
    })
}
