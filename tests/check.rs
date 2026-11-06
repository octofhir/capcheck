//! Runs against a small server that says one thing and does another.

use capcheck::{Options, Report, Verdict, check};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fhir(status: u16, body: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.to_string(), "application/fhir+json")
}

fn bundle(kind: &str, entries: Value) -> Value {
    json!({"resourceType": "Bundle", "type": kind, "entry": entries})
}

fn outcome(text: &str) -> Value {
    json!({"resourceType": "OperationOutcome", "issue": [{"severity": "error", "diagnostics": text}]})
}

fn statement() -> Value {
    json!({
        "resourceType": "CapabilityStatement",
        "fhirVersion": "4.0.1",
        "format": ["application/fhir+json"],
        "rest": [{
            "mode": "server",
            "interaction": [{"code": "search-system"}],
            "resource": [{
                "type": "Patient",
                "versioning": "versioned",
                "interaction": [
                    {"code": "read"}, {"code": "vread"}, {"code": "search-type"},
                    {"code": "history-instance"}
                ],
                "searchParam": [
                    {"name": "name", "type": "string"},
                    {"name": "invented", "type": "token"},
                    {"name": "both", "type": "composite"}
                ],
                "operation": [{"name": "everything"}, {"name": "lastn"}]
            }, {
                "type": "Basic",
                "interaction": [{"code": "read"}, {"code": "search-type"}]
            }]
        }]
    })
}

async fn server() -> MockServer {
    let server = MockServer::start().await;
    let patient = json!({"resourceType": "Patient", "id": "1", "meta": {"versionId": "2"}});
    let entry = json!([{"resource": patient}]);

    Mock::given(method("GET"))
        .and(path("/metadata"))
        .respond_with(fhir(200, statement()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Patient"))
        .and(query_param("capcheck-unknown-parameter", "1"))
        .respond_with(fhir(400, outcome("Unknown parameter")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Patient"))
        .and(query_param("invented", "capcheck"))
        .and(header("Prefer", "handling=strict"))
        .respond_with(fhir(400, outcome("Unknown parameter invented")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Patient"))
        .respond_with(fhir(200, bundle("searchset", entry)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Patient/1"))
        .respond_with(fhir(200, patient.clone()).insert_header("ETag", "W/\"2\""))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Patient/1/_history/2"))
        .respond_with(fhir(404, outcome("No versions here")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Patient/1/_history"))
        .respond_with(fhir(200, bundle("history", json!([]))))
        .mount(&server)
        .await;
    // History of the type works, and the statement does not say so.
    Mock::given(method("GET"))
        .and(path("/Patient/_history"))
        .respond_with(fhir(200, bundle("history", json!([]))))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Patient/$everything"))
        .respond_with(fhir(404, outcome("Not at this level")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Patient/1/$everything"))
        .respond_with(fhir(200, bundle("searchset", json!([]))))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Patient/$lastn"))
        .respond_with(fhir(501, outcome("Not implemented")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Patient/1/$lastn"))
        .respond_with(fhir(501, outcome("Not implemented")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Basic"))
        .respond_with(fhir(200, bundle("searchset", json!([]))))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(fhir(404, outcome("Not found")))
        .mount(&server)
        .await;
    server
}

fn verdict(report: &Report, subject: &str, claim: &str) -> Verdict {
    report
        .findings
        .iter()
        .find(|finding| finding.subject == subject && finding.claim == claim)
        .unwrap_or_else(|| panic!("no finding for {subject}: {claim}"))
        .verdict
}

#[tokio::test]
async fn each_claim_gets_its_verdict() {
    let mock = server().await;
    let report = check(&Options::new(mock.uri())).await.unwrap();

    assert_eq!(report.fhir_version.as_deref(), Some("4.0.1"));
    assert_eq!(
        verdict(&report, "Patient", "interaction read"),
        Verdict::Pass
    );
    assert_eq!(
        verdict(&report, "Patient", "interaction vread"),
        Verdict::Fail
    );
    assert_eq!(
        verdict(&report, "Patient", "versioning: ETag on read"),
        Verdict::Pass
    );
    assert_eq!(
        verdict(&report, "Patient", "interaction history-type"),
        Verdict::Undeclared
    );
    assert_eq!(
        verdict(&report, "Patient", "searchParam name"),
        Verdict::Pass
    );
    assert_eq!(
        verdict(&report, "Patient", "searchParam invented"),
        Verdict::Fail
    );
    assert_eq!(
        verdict(&report, "Patient", "searchParam both"),
        Verdict::Skip
    );
    assert_eq!(
        verdict(&report, "Patient", "operation $everything"),
        Verdict::Pass
    );
    assert_eq!(
        verdict(&report, "Patient", "operation $lastn"),
        Verdict::Fail
    );
    assert_eq!(verdict(&report, "Basic", "interaction read"), Verdict::Skip);
    assert_eq!(
        verdict(&report, "server", "interaction search-system"),
        Verdict::Fail
    );
    assert_eq!(
        verdict(&report, "server", "errors are OperationOutcomes"),
        Verdict::Pass
    );
}

#[tokio::test]
async fn findings_come_worst_first_and_reports_say_why() {
    let mock = server().await;
    let report = check(&Options::new(mock.uri())).await.unwrap();
    assert_eq!(report.findings[0].verdict, Verdict::Fail);

    let text = report.text(false);
    assert!(
        text.contains("FAIL       Patient: interaction vread: 404 Not Found: No versions here")
    );
    assert!(!text.contains("SKIP"));
    assert!(report.text(true).contains("SKIP"));
    let markdown = report.markdown();
    assert!(markdown.contains("| fail | Patient | searchParam invented |"));
}

#[tokio::test]
async fn only_the_asked_types_are_checked() {
    let mock = server().await;
    let mut options = Options::new(mock.uri());
    options.types = vec!["Basic".to_owned()];
    let report = check(&options).await.unwrap();
    assert!(
        report
            .findings
            .iter()
            .filter(|finding| finding.subject == "Patient")
            .all(|finding| finding.claim.starts_with("operation"))
    );
}

#[tokio::test]
async fn an_operation_at_rest_level_may_belong_to_resource_types() {
    let mock = MockServer::start().await;
    let statement = json!({
        "resourceType": "CapabilityStatement",
        "rest": [{
            "mode": "server",
            "operation": [
                {"name": "lookup"},
                {"name": "meta", "definition": "http://example.org/OperationDefinition/meta"}
            ]
        }]
    });
    let definition = json!({"resourceType": "OperationDefinition", "system": true});
    Mock::given(method("GET"))
        .and(path("/metadata"))
        .respond_with(fhir(200, statement))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/OperationDefinition"))
        .respond_with(fhir(
            200,
            bundle("searchset", json!([{"resource": definition}])),
        ))
        .mount(&mock)
        .await;
    let report = check(&Options::new(mock.uri())).await.unwrap();

    assert_eq!(
        verdict(&report, "server", "operation $lookup"),
        Verdict::Skip
    );
    assert_eq!(verdict(&report, "server", "operation $meta"), Verdict::Fail);
}

#[tokio::test]
async fn a_search_without_a_self_link_proves_nothing_either_way() {
    let mock = MockServer::start().await;
    let parameter = json!([{"name": "name", "type": "string"}]);
    let statement = json!({
        "resourceType": "CapabilityStatement",
        "rest": [{
            "mode": "server",
            "resource": [
                {"type": "Library", "searchParam": parameter},
                {"type": "Group", "searchParam": parameter}
            ]
        }]
    });
    Mock::given(method("GET"))
        .and(path("/metadata"))
        .respond_with(fhir(200, statement))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Library"))
        .and(query_param("capcheck-unknown-parameter", "1"))
        .respond_with(fhir(500, outcome("Unsupported search parameter")))
        .mount(&mock)
        .await;
    for resource in ["/Library", "/Group"] {
        Mock::given(method("GET"))
            .and(path(resource))
            .respond_with(fhir(200, bundle("searchset", json!([]))))
            .mount(&mock)
            .await;
    }
    let report = check(&Options::new(mock.uri())).await.unwrap();

    // Library refuses what it does not know, so it knows this one.
    assert_eq!(
        verdict(&report, "Library", "searchParam name"),
        Verdict::Pass
    );
    assert_eq!(verdict(&report, "Group", "searchParam name"), Verdict::Skip);
}

#[tokio::test]
async fn a_server_with_no_statement_is_an_error() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(fhir(404, outcome("Not found")))
        .mount(&mock)
        .await;
    let error = check(&Options::new(mock.uri())).await.unwrap_err();
    assert!(error.to_string().contains("no CapabilityStatement"));
}
