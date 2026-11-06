//! What came of the checks.

use std::fmt::Write as _;

use serde::Serialize;

/// The outcome of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// The statement claims something the server does not do.
    Fail,
    /// The server does something the statement does not claim.
    Undeclared,
    /// The claim could not be put to the test.
    Skip,
    /// The server does what the statement claims.
    Pass,
}

/// One claim and what the server did about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// What the claim is about, such as `Patient` or `server`.
    pub subject: String,
    /// The claim, such as `interaction read`.
    pub claim: String,
    pub verdict: Verdict,
    /// What was seen. Empty when there is nothing more to say.
    pub detail: String,
}

impl Finding {
    pub(crate) fn new(
        subject: impl Into<String>,
        claim: impl Into<String>,
        verdict: Verdict,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            subject: subject.into(),
            claim: claim.into(),
            verdict,
            detail: detail.into(),
        }
    }
}

/// Everything a run found.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub base: String,
    pub fhir_version: Option<String>,
    pub findings: Vec<Finding>,
}

impl Report {
    #[must_use]
    pub fn count(&self, verdict: Verdict) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.verdict == verdict)
            .count()
    }

    fn summary(&self) -> String {
        format!(
            "{} passed, {} failed, {} undeclared, {} skipped",
            self.count(Verdict::Pass),
            self.count(Verdict::Fail),
            self.count(Verdict::Undeclared),
            self.count(Verdict::Skip)
        )
    }

    fn notable(&self, skipped: bool) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(move |finding| {
            matches!(finding.verdict, Verdict::Fail | Verdict::Undeclared)
                || (skipped && finding.verdict == Verdict::Skip)
        })
    }

    /// The findings as lines of text; `verbose` adds what was skipped.
    #[must_use]
    pub fn text(&self, verbose: bool) -> String {
        let mut out = format!(
            "{} (FHIR {})\n",
            self.base,
            self.fhir_version.as_deref().unwrap_or("unknown")
        );
        for finding in self.notable(verbose) {
            let label = match finding.verdict {
                Verdict::Fail => "FAIL",
                Verdict::Undeclared => "UNDECLARED",
                Verdict::Skip => "SKIP",
                Verdict::Pass => "PASS",
            };
            let _ = write!(out, "{label:<11}{}: {}", finding.subject, finding.claim);
            if !finding.detail.is_empty() {
                let _ = write!(out, ": {}", finding.detail);
            }
            out.push('\n');
        }
        out.push_str(&self.summary());
        out.push('\n');
        out
    }

    /// The findings as a Markdown report.
    #[must_use]
    pub fn markdown(&self) -> String {
        let mut out = format!(
            "# capcheck: {}\n\nFHIR {}. {}.\n",
            self.base,
            self.fhir_version.as_deref().unwrap_or("unknown"),
            self.summary()
        );
        let mut rows = self.notable(false).peekable();
        if rows.peek().is_some() {
            out.push_str("\n| Verdict | Subject | Claim | Seen |\n|---|---|---|---|\n");
            for finding in rows {
                let verdict = match finding.verdict {
                    Verdict::Fail => "fail",
                    _ => "undeclared",
                };
                let _ = writeln!(
                    out,
                    "| {verdict} | {} | {} | {} |",
                    finding.subject,
                    finding.claim,
                    finding.detail.replace('|', "\\|")
                );
            }
        }
        out
    }
}
