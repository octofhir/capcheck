//! Checks that write. They run only when asked for, on one resource type, and clean up after
//! themselves as far as the server allows.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::Error;
use crate::claims::{Capability, Resource};
use crate::http::{Answer, Server};
use crate::report::{Finding, Verdict};

/// Marks what capcheck created, and makes conditional requests match only that.
const MARK: &str = "urn:capcheck";

fn patient(id: Option<&str>, value: &str) -> Value {
    let mut body = json!({
        "resourceType": "Patient",
        "identifier": [{"system": MARK, "value": value}],
        "name": [{"family": "Capcheck"}],
    });
    if let Some(id) = id {
        body["id"] = json!(id);
    }
    body
}

fn check(name: &str, claim: &str, worked: bool, answer: &Answer) -> Finding {
    let verdict = if worked { Verdict::Pass } else { Verdict::Fail };
    Finding::new(
        name,
        claim,
        verdict,
        if worked {
            String::new()
        } else {
            answer.reason()
        },
    )
}

/// The write side of `Patient`, the one type every server has and whose minimal form is known.
pub(crate) async fn patient_writes(
    server: &Server,
    capability: &Capability,
    run: &str,
) -> Result<Vec<Finding>, Error> {
    let Some(claim) = capability
        .rest
        .iter()
        .flat_map(|rest| &rest.resource)
        .find(|resource| resource.name == "Patient")
    else {
        return Ok(vec![Finding::new(
            "Patient",
            "write checks",
            Verdict::Skip,
            "the statement does not list Patient",
        )]);
    };
    let mut findings = Vec::new();
    let name = "Patient";
    if !claim.declares("create") {
        findings.push(Finding::new(
            name,
            "write checks",
            Verdict::Skip,
            "the statement does not declare create",
        ));
        return Ok(findings);
    }

    let value = format!("{run}-1");
    let created = server
        .request(Method::POST, "/Patient", &[], Some(&patient(None, &value)))
        .await?;
    let id = created.body["id"].as_str().map(str::to_owned);
    findings.push(check(
        name,
        "interaction create",
        created.status == StatusCode::CREATED
            && id.is_some()
            && created.header("location").is_some(),
        &created,
    ));
    let Some(id) = id else {
        return Ok(findings);
    };

    findings.extend(update_checks(server, claim, &id, &value).await?);
    findings.extend(conditional_checks(server, claim, run).await?);

    if claim.declares("delete") {
        let deleted = server
            .request(Method::DELETE, &format!("/Patient/{id}"), &[], None)
            .await?;
        findings.push(check(
            name,
            "interaction delete",
            matches!(
                deleted.status,
                StatusCode::OK | StatusCode::ACCEPTED | StatusCode::NO_CONTENT
            ),
            &deleted,
        ));
        let gone = server.get(&format!("/Patient/{id}")).await?;
        findings.push(check(
            name,
            "a deleted resource is gone",
            matches!(gone.status, StatusCode::GONE | StatusCode::NOT_FOUND),
            &gone,
        ));
    }
    Ok(findings)
}

async fn update_checks(
    server: &Server,
    claim: &Resource,
    id: &str,
    value: &str,
) -> Result<Vec<Finding>, Error> {
    let name = "Patient";
    let path = format!("/Patient/{id}");
    let mut findings = Vec::new();
    let body = patient(Some(id), value);
    if claim.declares("update") {
        let updated = server.request(Method::PUT, &path, &[], Some(&body)).await?;
        findings.push(check(
            name,
            "interaction update",
            updated.status == StatusCode::OK,
            &updated,
        ));
    }
    if claim.versioning.as_deref() == Some("versioned-update") {
        let stale = server
            .request(
                Method::PUT,
                &path,
                &[("If-Match", "W/\"capcheck-stale\"")],
                Some(&body),
            )
            .await?;
        findings.push(check(
            name,
            "versioning versioned-update: a stale If-Match is refused",
            matches!(
                stale.status,
                StatusCode::PRECONDITION_FAILED | StatusCode::CONFLICT
            ),
            &stale,
        ));
    }
    if claim.declares("patch") {
        let patch = json!([{"op": "replace", "path": "/name/0/family", "value": "Patched"}]);
        let patched = server
            .request(
                Method::PATCH,
                &path,
                &[("Content-Type", "application/json-patch+json")],
                Some(&patch),
            )
            .await?;
        findings.push(check(
            name,
            "interaction patch",
            patched.status == StatusCode::OK && patched.body["name"][0]["family"] == "Patched",
            &patched,
        ));
    }
    Ok(findings)
}

async fn conditional_checks(
    server: &Server,
    claim: &Resource,
    run: &str,
) -> Result<Vec<Finding>, Error> {
    let name = "Patient";
    let mut findings = Vec::new();
    let value = format!("{run}-2");
    let criteria = format!("identifier={MARK}|{value}");
    let mut created = None;

    if claim.conditional_create == Some(true) {
        let header = [("If-None-Exist", criteria.as_str())];
        let first = server
            .request(
                Method::POST,
                "/Patient",
                &header,
                Some(&patient(None, &value)),
            )
            .await?;
        created = first.body["id"].as_str().map(str::to_owned);
        let second = server
            .request(
                Method::POST,
                "/Patient",
                &header,
                Some(&patient(None, &value)),
            )
            .await?;
        findings.push(check(
            name,
            "conditionalCreate: a match creates nothing",
            first.status == StatusCode::CREATED && second.status == StatusCode::OK,
            &second,
        ));
    }
    if claim.conditional_update == Some(true) && created.is_some() {
        let answer = server
            .request(
                Method::PUT,
                &format!("/Patient?{criteria}"),
                &[],
                Some(&patient(None, &value)),
            )
            .await?;
        findings.push(check(
            name,
            "conditionalUpdate",
            answer.status == StatusCode::OK,
            &answer,
        ));
    }
    if matches!(
        claim.conditional_delete.as_deref(),
        Some("single" | "multiple")
    ) && created.is_some()
    {
        let answer = server
            .request(Method::DELETE, &format!("/Patient?{criteria}"), &[], None)
            .await?;
        findings.push(check(
            name,
            "conditionalDelete",
            answer.status.is_success(),
            &answer,
        ));
    } else if let Some(id) = created {
        server
            .request(Method::DELETE, &format!("/Patient/{id}"), &[], None)
            .await?;
    }
    Ok(findings)
}
