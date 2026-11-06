//! Checks that only read: safe on any server.

use futures::stream::{self, StreamExt};
use reqwest::{Method, StatusCode};
use serde_json::Value;

use crate::Error;
use crate::claims::{Capability, Operation, Resource, Rest, SearchParam};
use crate::http::{Answer, Server, encode};
use crate::report::{Finding, Verdict};

const STRICT: (&str, &str) = ("Prefer", "handling=strict");

/// A parameter no server knows: how strict handling is told apart from no strict handling.
const UNKNOWN_PARAMETER: &str = "capcheck-unknown-parameter";

fn bundle_of(answer: &Answer, kind: &str) -> bool {
    answer.status == StatusCode::OK
        && answer.body["resourceType"] == "Bundle"
        && answer.body["type"] == kind
}

/// The statement itself, and what it says about formats.
pub(crate) async fn statement(server: &Server, metadata: &Answer) -> Vec<Finding> {
    let mut findings = Vec::new();
    let subject = "server";
    let content_type = metadata.header("content-type").unwrap_or_default();
    findings.push(Finding::new(
        subject,
        "format application/fhir+json",
        if content_type.starts_with("application/fhir+json") {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
        format!("Content-Type {content_type}"),
    ));
    let unknown = server.get("/capcheck-no-such-type").await;
    if let Ok(answer) = unknown {
        let outcome = answer.body["resourceType"] == "OperationOutcome";
        findings.push(Finding::new(
            subject,
            "errors are OperationOutcomes",
            if outcome && answer.status.is_client_error() {
                Verdict::Pass
            } else {
                Verdict::Fail
            },
            format!("an unknown type gave {}", answer.status),
        ));
    }
    findings
}

/// System level interactions.
pub(crate) async fn system(server: &Server, rest: &Rest) -> Result<Vec<Finding>, Error> {
    let mut findings = Vec::new();
    for (code, path, kind) in [
        ("search-system", "?_count=1", "searchset"),
        ("history-system", "/_history?_count=1", "history"),
    ] {
        let answer = server.get(path).await?;
        findings.push(judge(
            "server",
            &format!("interaction {code}"),
            rest.declares(code),
            bundle_of(&answer, kind),
            &answer,
        ));
    }
    Ok(findings)
}

/// A claim and what the server did. `worked` without the claim is undeclared; a refusal for
/// lack of rights says nothing either way.
fn judge(subject: &str, claim: &str, declared: bool, worked: bool, answer: &Answer) -> Finding {
    let verdict = match (declared, worked) {
        (true, true) => Verdict::Pass,
        (false, true) => Verdict::Undeclared,
        _ if matches!(
            answer.status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) =>
        {
            Verdict::Skip
        }
        (true, false) => Verdict::Fail,
        (false, false) => Verdict::Pass,
    };
    let detail = if verdict == Verdict::Pass {
        String::new()
    } else {
        answer.reason()
    };
    Finding::new(subject, claim, verdict, detail)
}

/// A value of the right shape for a search parameter type; `None` where one cannot be made up.
fn sample(kind: Option<&str>) -> Option<&'static str> {
    match kind? {
        "number" | "quantity" => Some("1"),
        "date" => Some("2000-01-01"),
        "string" | "token" | "reference" => Some("capcheck"),
        "uri" => Some("http://example.org/capcheck"),
        _ => None,
    }
}

async fn search_param(
    server: &Server,
    resource: &str,
    parameter: &SearchParam,
    strict: bool,
) -> Result<Finding, Error> {
    let claim = format!("searchParam {}", parameter.name);
    let Some(value) = sample(parameter.kind.as_deref()) else {
        return Ok(Finding::new(
            resource,
            claim,
            Verdict::Skip,
            format!(
                "no sample value for type {}",
                parameter.kind.as_deref().unwrap_or("(none)")
            ),
        ));
    };
    let path = format!(
        "/{resource}?{}={}&_count=1",
        encode(&parameter.name),
        encode(value)
    );
    let answer = if strict {
        server.request(Method::GET, &path, &[STRICT], None).await?
    } else {
        server.get(&path).await?
    };
    Ok(if bundle_of(&answer, "searchset") {
        if strict || used_in_self_link(&answer, &parameter.name) {
            Finding::new(resource, claim, Verdict::Pass, "")
        } else {
            Finding::new(
                resource,
                claim,
                Verdict::Fail,
                "the self link does not carry the parameter, so it was ignored",
            )
        }
    } else {
        judge(resource, &claim, true, false, &answer)
    })
}

/// Parameters a server ignored are left out of the `self` link of the result.
fn used_in_self_link(answer: &Answer, name: &str) -> bool {
    answer.body["link"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|link| link["relation"] == "self")
        .and_then(|link| link["url"].as_str())
        .is_some_and(|url| url.contains(&format!("{name}=")) || url.contains(&encode(name)))
}

async fn control(
    server: &Server,
    resource: &str,
    parameter: &str,
    value: &str,
    strict: bool,
) -> Result<Finding, Error> {
    let claim = format!("{parameter} {value}");
    let path = format!("/{resource}?{parameter}={}&_count=1", encode(value));
    let headers: &[(&str, &str)] = if strict { &[STRICT] } else { &[] };
    let answer = server.request(Method::GET, &path, headers, None).await?;
    Ok(judge(
        resource,
        &claim,
        true,
        bundle_of(&answer, "searchset"),
        &answer,
    ))
}

/// Whether unknown parameters are refused here, so that a pass means something.
async fn strict_handling(server: &Server, resource: &str) -> Result<bool, Error> {
    let answer = server
        .request(
            Method::GET,
            &format!("/{resource}?{UNKNOWN_PARAMETER}=1&_count=1"),
            &[STRICT],
            None,
        )
        .await?;
    Ok(answer.status.is_client_error())
}

fn entry_of(search: &Answer) -> Option<&Value> {
    search.body["entry"]
        .as_array()?
        .iter()
        .find(|entry| entry["search"]["mode"] != "include" && entry["search"]["mode"] != "outcome")
        .map(|entry| &entry["resource"])
}

/// Interactions on one stored resource.
async fn instance(
    server: &Server,
    claim: &Resource,
    id: &str,
    version: Option<&str>,
) -> Result<Vec<Finding>, Error> {
    let name = &claim.name;
    let mut findings = Vec::new();
    let path = format!("/{name}/{id}");
    let read = server.get(&path).await?;
    let worked = read.status == StatusCode::OK
        && read.body["resourceType"] == name.as_str()
        && read.body["id"] == id;
    findings.push(judge(
        name,
        "interaction read",
        claim.declares("read"),
        worked,
        &read,
    ));

    if let Some(version) = version {
        let vread = server.get(&format!("{path}/_history/{version}")).await?;
        findings.push(judge(
            name,
            "interaction vread",
            claim.declares("vread"),
            vread.status == StatusCode::OK && vread.body["meta"]["versionId"] == version,
            &vread,
        ));
        if matches!(
            claim.versioning.as_deref(),
            Some("versioned" | "versioned-update")
        ) {
            findings.push(Finding::new(
                name.as_str(),
                "versioning: ETag on read",
                if read
                    .header("etag")
                    .is_some_and(|etag| etag.contains(version))
                {
                    Verdict::Pass
                } else {
                    Verdict::Fail
                },
                read.header("etag").unwrap_or("no ETag").to_owned(),
            ));
        }
        if matches!(
            claim.conditional_read.as_deref(),
            Some("not-match" | "full-support")
        ) {
            let tag = format!("W/\"{version}\"");
            let answer = server
                .request(Method::GET, &path, &[("If-None-Match", &tag)], None)
                .await?;
            findings.push(judge(
                name,
                "conditionalRead If-None-Match",
                true,
                answer.status == StatusCode::NOT_MODIFIED,
                &answer,
            ));
        }
        if matches!(
            claim.conditional_read.as_deref(),
            Some("modified-since" | "full-support")
        ) && let Some(modified) = read.header("last-modified")
        {
            let answer = server
                .request(Method::GET, &path, &[("If-Modified-Since", modified)], None)
                .await?;
            findings.push(judge(
                name,
                "conditionalRead If-Modified-Since",
                true,
                answer.status == StatusCode::NOT_MODIFIED,
                &answer,
            ));
        }
    }

    let history = server.get(&format!("{path}/_history?_count=1")).await?;
    findings.push(judge(
        name,
        "interaction history-instance",
        claim.declares("history-instance"),
        bundle_of(&history, "history"),
        &history,
    ));
    Ok(findings)
}

/// Everything readable about one resource type.
pub(crate) async fn resource(
    server: &Server,
    claim: &Resource,
    concurrency: usize,
) -> Result<Vec<Finding>, Error> {
    let name = &claim.name;
    let mut findings = Vec::new();

    let search = server.get(&format!("/{name}?_count=1")).await?;
    findings.push(judge(
        name,
        "interaction search-type",
        claim.declares("search-type"),
        bundle_of(&search, "searchset"),
        &search,
    ));
    let history = server.get(&format!("/{name}/_history?_count=1")).await?;
    findings.push(judge(
        name,
        "interaction history-type",
        claim.declares("history-type"),
        bundle_of(&history, "history"),
        &history,
    ));

    match entry_of(&search)
        .and_then(|stored| Some((stored["id"].as_str()?, stored["meta"]["versionId"].as_str())))
    {
        Some((id, version)) => findings.extend(instance(server, claim, id, version).await?),
        None if claim.declares("read") => findings.push(Finding::new(
            name.as_str(),
            "interaction read",
            Verdict::Skip,
            "the server holds no such resource to read",
        )),
        None => {}
    }

    if !claim.search_param.is_empty() || !claim.search_include.is_empty() {
        let strict = strict_handling(server, name).await?;
        let parameters = stream::iter(&claim.search_param)
            .map(|parameter| search_param(server, name, parameter, strict))
            .buffer_unordered(concurrency)
            .collect::<Vec<_>>()
            .await;
        for finding in parameters {
            findings.push(finding?);
        }
        for value in claim.search_include.iter().filter(|value| *value != "*") {
            findings.push(control(server, name, "_include", value, strict).await?);
        }
        for value in claim
            .search_rev_include
            .iter()
            .filter(|value| *value != "*")
        {
            findings.push(control(server, name, "_revinclude", value, strict).await?);
        }
    }
    Ok(findings)
}

/// Operations of the FHIR specification that change nothing and can be called at type or
/// system level: a server need not publish these definitions as resources.
const READ_ONLY_OPERATIONS: [&str; 15] = [
    "care-gaps",
    "data-requirements",
    "evaluate-measure",
    "expand",
    "everything",
    "lastn",
    "lookup",
    "meta",
    "stats",
    "subsumes",
    "summary",
    "translate",
    "validate",
    "validate-code",
    "versions",
];

/// Whether an operation changes nothing, by its definition or the specification: only those
/// may be called with a GET. `None` when that cannot be told.
async fn safe_to_call(server: &Server, operation: &Operation) -> Result<Option<bool>, Error> {
    if READ_ONLY_OPERATIONS.contains(&operation.name.as_str()) {
        return Ok(Some(true));
    }
    let Some(canonical) = &operation.definition else {
        return Ok(None);
    };
    let url = canonical.split('|').next().unwrap_or(canonical);
    let found = server
        .get(&format!(
            "/OperationDefinition?url={}&_count=1",
            encode(url)
        ))
        .await?;
    Ok(entry_of(&found)
        .map(|definition| definition["affectsState"] == false && definition["type"] == true))
}

fn missing(answer: &Answer) -> bool {
    matches!(
        answer.status,
        StatusCode::NOT_FOUND | StatusCode::NOT_IMPLEMENTED
    )
}

/// Operations without side effects answer, whatever they say to no parameters. The statement
/// does not say at which level one works, so one the type does not know is tried on an instance.
pub(crate) async fn operation(
    server: &Server,
    subject: &str,
    path: &str,
    operation: &Operation,
) -> Result<Finding, Error> {
    let claim = format!("operation ${}", operation.name);
    if safe_to_call(server, operation).await? != Some(true) {
        return Ok(Finding::new(
            subject,
            claim,
            Verdict::Skip,
            "nothing says it can be called with a GET",
        ));
    }
    let mut answer = server.get(&format!("{path}/${}", operation.name)).await?;
    if missing(&answer) && !path.is_empty() {
        let search = server.get(&format!("{path}?_count=1")).await?;
        let Some(id) = entry_of(&search).and_then(|stored| stored["id"].as_str()) else {
            return Ok(Finding::new(
                subject,
                claim,
                Verdict::Skip,
                "not at type level, and there is no resource to try it on",
            ));
        };
        answer = server
            .get(&format!("{path}/{id}/${}", operation.name))
            .await?;
    }
    let gone = missing(&answer);
    Ok(Finding::new(
        subject,
        claim,
        if gone { Verdict::Fail } else { Verdict::Pass },
        if gone { answer.reason() } else { String::new() },
    ))
}

/// All type level operations a statement declares.
pub(crate) async fn operations(
    server: &Server,
    capability: &Capability,
) -> Result<Vec<Finding>, Error> {
    let mut findings = Vec::new();
    for rest in &capability.rest {
        for declared in &rest.operation {
            findings.push(operation(server, "server", "", declared).await?);
        }
        for resource in &rest.resource {
            let path = format!("/{}", resource.name);
            for declared in &resource.operation {
                findings.push(operation(server, &resource.name, &path, declared).await?);
            }
        }
    }
    Ok(findings)
}
