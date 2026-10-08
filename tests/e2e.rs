//! End-to-end tests: a real MCP client talks to the server over an in-memory
//! transport, and the server talks to a mock VinylDNS API (wiremock).

mod common;

use common::*;
use serde_json::{Value, json};
use vinyldns_mcp::{config::ConfirmationMode, server::WRITE_TOOLS};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, header_exists, method, path, query_param},
};

/// Mocks the zone lookup and empty record set search used by plan_create_record_set.
async fn mock_zone(api: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/zones/z1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "zone": { "id": "z1", "name": "example.com.", "shared": false } })),
        )
        .mount(api)
        .await;
    Mock::given(method("GET"))
        .and(path("/zones/z1/recordsets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "recordSets": [] })))
        .mount(api)
        .await;
}

fn create_args() -> Value {
    json!({
        "zone_id": "z1",
        "name": "www",
        "record_type": "a",
        "ttl": 300,
        "records": [{ "address": "10.0.0.5" }]
    })
}

#[tokio::test]
async fn read_only_mode_hides_write_tools() {
    let api = MockServer::start().await;
    let client = connect(
        config(&api, false, ConfirmationMode::Auto),
        TestClient { elicit_answer: None },
    )
    .await;
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert!(names.contains(&"list_zones".to_string()));
    for w in WRITE_TOOLS {
        assert!(
            !names.contains(&w.to_string()),
            "{w} should be hidden in read-only mode"
        );
    }
}

#[tokio::test]
async fn write_mode_exposes_write_tools() {
    let api = MockServer::start().await;
    let client = connect(
        config(&api, true, ConfirmationMode::Auto),
        TestClient { elicit_answer: None },
    )
    .await;
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for w in WRITE_TOOLS {
        assert!(names.contains(&w.to_string()), "{w} should be available");
    }
}

#[tokio::test]
async fn read_requests_are_signed_and_pass_query_parameters() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zones"))
        .and(query_param("nameFilter", "ex ample*"))
        .and(query_param("maxItems", "5"))
        .and(header_exists("x-amz-date"))
        .and(signed)
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "zones": [], "maxItems": 5 })))
        .expect(1)
        .mount(&api)
        .await;

    let client = connect(
        config(&api, false, ConfirmationMode::Auto),
        TestClient { elicit_answer: None },
    )
    .await;
    let out = json_of(
        &call(
            &client,
            "list_zones",
            json!({ "name_filter": "ex ample*", "max_items": 5 }),
        )
        .await,
    );
    assert_eq!(out["maxItems"], 5);
}

#[tokio::test]
async fn api_errors_are_reported_as_tool_errors_with_hints() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zones/name/missing.com."))
        .respond_with(ResponseTemplate::new(404).set_body_string("Zone with name missing.com. does not exist"))
        .mount(&api)
        .await;

    let client = connect(
        config(&api, false, ConfirmationMode::Auto),
        TestClient { elicit_answer: None },
    )
    .await;
    let result = call(&client, "get_zone", json!({ "zone_name": "missing.com." })).await;
    assert_eq!(result.is_error, Some(true));
    let msg = text(&result);
    assert!(
        msg.contains("404") && msg.contains("does not exist") && msg.contains("Hint:"),
        "{msg}"
    );
}

#[tokio::test]
async fn create_is_applied_only_after_elicitation_confirms() {
    let api = MockServer::start().await;
    mock_zone(&api).await;
    Mock::given(method("POST"))
        .and(path("/zones/z1/recordsets"))
        .and(body_partial_json(
            json!({ "zoneId": "z1", "name": "www", "type": "A", "ttl": 300 }),
        ))
        .and(signed)
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "chg-1", "status": "Pending" })))
        .expect(1)
        .mount(&api)
        .await;

    let client = connect(
        config(&api, true, ConfirmationMode::Auto),
        TestClient {
            elicit_answer: Some(true),
        },
    )
    .await;
    let plan = json_of(&call(&client, "plan_create_record_set", create_args()).await);
    assert_eq!(plan["status"], "pending_confirmation");
    assert_eq!(plan["summary"], "Create A www.example.com. (ttl 300) with 1 record(s)");
    let token = plan["token"].as_str().unwrap().to_string();

    let applied = json_of(&call(&client, "confirm_change", json!({ "token": token })).await);
    assert_eq!(applied["result"]["id"], "chg-1");

    // Tokens are single-use.
    let again = call(&client, "confirm_change", json!({ "token": token })).await;
    assert_eq!(again.is_error, Some(true));
}

#[tokio::test]
async fn declining_the_elicitation_discards_the_plan() {
    let api = MockServer::start().await;
    mock_zone(&api).await;
    Mock::given(method("POST"))
        .and(path("/zones/z1/recordsets"))
        .respond_with(ResponseTemplate::new(202))
        .expect(0)
        .mount(&api)
        .await;

    let client = connect(
        config(&api, true, ConfirmationMode::Auto),
        TestClient {
            elicit_answer: Some(false),
        },
    )
    .await;
    let plan = json_of(&call(&client, "plan_create_record_set", create_args()).await);
    let token = plan["token"].as_str().unwrap();

    let result = call(&client, "confirm_change", json!({ "token": token })).await;
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("declined"));
    let pending = json_of(&call(&client, "list_pending_changes", json!({})).await);
    assert_eq!(pending["pending"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn elicit_mode_refuses_clients_without_elicitation() {
    let api = MockServer::start().await;
    mock_zone(&api).await;
    Mock::given(method("POST"))
        .and(path("/zones/z1/recordsets"))
        .respond_with(ResponseTemplate::new(202))
        .expect(0)
        .mount(&api)
        .await;

    let client = connect(
        config(&api, true, ConfirmationMode::Elicit),
        TestClient { elicit_answer: None },
    )
    .await;
    let plan = json_of(&call(&client, "plan_create_record_set", create_args()).await);
    let result = call(&client, "confirm_change", json!({ "token": plan["token"] })).await;
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("does not support elicitation"));
}

#[tokio::test]
async fn auto_mode_falls_back_to_token_confirmation() {
    let api = MockServer::start().await;
    mock_zone(&api).await;
    Mock::given(method("POST"))
        .and(path("/zones/z1/recordsets"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "chg-2" })))
        .expect(1)
        .mount(&api)
        .await;

    let client = connect(
        config(&api, true, ConfirmationMode::Auto),
        TestClient { elicit_answer: None },
    )
    .await;
    let plan = json_of(&call(&client, "plan_create_record_set", create_args()).await);
    let applied = json_of(&call(&client, "confirm_change", json!({ "token": plan["token"] })).await);
    assert_eq!(applied["result"]["id"], "chg-2");
}

#[tokio::test]
async fn create_warns_about_existing_record_set() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zones/z1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "zone": { "name": "example.com." } })))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/zones/z1/recordsets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "recordSets": [{ "id": "rs-9", "name": "www", "type": "A" }] })),
        )
        .mount(&api)
        .await;

    let client = connect(
        config(&api, true, ConfirmationMode::Token),
        TestClient { elicit_answer: None },
    )
    .await;
    let plan = json_of(&call(&client, "plan_create_record_set", create_args()).await);
    let warning = plan["preview"]["warnings"][0].as_str().unwrap();
    assert!(
        warning.contains("rs-9") && warning.contains("plan_update_record_set"),
        "{warning}"
    );
}

#[tokio::test]
async fn update_preserves_owner_group_and_shows_diff() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zones/z1/recordsets/rs1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "recordSet": {
            "id": "rs1", "zoneId": "z1", "name": "www", "type": "A", "ttl": 300,
            "records": [{ "address": "10.0.0.5" }], "ownerGroupId": "g1", "fqdn": "www.example.com."
        }})))
        .mount(&api)
        .await;
    Mock::given(method("PUT"))
        .and(path("/zones/z1/recordsets/rs1"))
        .and(body_partial_json(
            json!({ "id": "rs1", "ttl": 600, "ownerGroupId": "g1", "records": [{ "address": "10.0.0.5" }] }),
        ))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "chg-3" })))
        .expect(1)
        .mount(&api)
        .await;

    let client = connect(
        config(&api, true, ConfirmationMode::Token),
        TestClient { elicit_answer: None },
    )
    .await;
    let plan = json_of(
        &call(
            &client,
            "plan_update_record_set",
            json!({ "zone_id": "z1", "record_set_id": "rs1", "ttl": 600 }),
        )
        .await,
    );
    assert_eq!(plan["preview"]["before"]["ttl"], 300);
    assert_eq!(plan["preview"]["after"]["ttl"], 600);
    assert_eq!(plan["preview"]["after"]["ownerGroupId"], "g1");

    let applied = json_of(&call(&client, "confirm_change", json!({ "token": plan["token"] })).await);
    assert_eq!(applied["result"]["id"], "chg-3");
}

#[tokio::test]
async fn batch_change_is_submitted_with_manual_review_flag() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/zones/batchrecordchanges"))
        .and(query_param("allowManualReview", "false"))
        .and(body_partial_json(json!({
            "comments": "migrate",
            "changes": [
                { "changeType": "DeleteRecordSet", "inputName": "old.example.com.", "type": "CNAME" },
                { "changeType": "Add", "inputName": "new.example.com.", "type": "A", "ttl": 300, "record": { "address": "10.1.1.1" } }
            ]
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "bc-1", "status": "PendingProcessing" })))
        .expect(1)
        .mount(&api)
        .await;

    let client = connect(
        config(&api, true, ConfirmationMode::Token),
        TestClient { elicit_answer: None },
    )
    .await;
    let plan = json_of(
        &call(
            &client,
            "plan_batch_change",
            json!({
                "comments": "migrate",
                "allow_manual_review": false,
                "changes": [
                    { "change_type": "DeleteRecordSet", "input_name": "old.example.com.", "record_type": "CNAME" },
                    { "change_type": "Add", "input_name": "new.example.com.", "record_type": "A", "ttl": 300,
                      "record": { "address": "10.1.1.1" } }
                ]
            }),
        )
        .await,
    );
    assert_eq!(plan["preview"]["changes"].as_array().unwrap().len(), 2);
    let applied = json_of(&call(&client, "confirm_change", json!({ "token": plan["token"] })).await);
    assert_eq!(applied["result"]["id"], "bc-1");
}

#[tokio::test]
async fn invalid_input_is_rejected_before_any_api_call() {
    let api = MockServer::start().await;
    let client = connect(
        config(&api, true, ConfirmationMode::Token),
        TestClient { elicit_answer: None },
    )
    .await;
    let mut args = create_args();
    args["ttl"] = json!(5);
    let result = call(&client, "plan_create_record_set", args).await;
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("ttl"));
    assert!(api.received_requests().await.unwrap().is_empty());
}
