//! End-to-end tests for the zone management and batch review tools.

mod common;

use common::*;
use serde_json::{Value, json};
use vinyldns_mcp::{
    config::ConfirmationMode,
    server::{ADMIN_TOOLS, WRITE_TOOLS},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, body_partial_json, method, path},
};

fn zone() -> Value {
    json!({
        "id": "z1",
        "name": "example.com.",
        "email": "old@example.com",
        "status": "Active",
        "adminGroupId": "g1",
        "adminGroupName": "dns-admins",
        "shared": false,
        "latestSync": "2026-10-01T00:00:00Z",
        "connection": {
            "name": "example.com.", "keyName": "vinyldns.", "key": "1:ENCRYPTED==", "primaryServer": "10.0.0.53"
        },
        "acl": { "rules": [
            { "accessLevel": "Read", "groupId": "g2", "recordTypes": ["A"], "description": "ops", "displayName": "ops-team" }
        ]},
        "accessLevel": "Delete"
    })
}

async fn mock_zone(api: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/zones/z1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "zone": zone() })))
        .mount(api)
        .await;
}

async fn tool_names(cfg: vinyldns_mcp::config::Config) -> Vec<String> {
    let client = connect(cfg, TestClient { elicit_answer: None }).await;
    client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
}

#[tokio::test]
async fn admin_tools_are_registered_only_when_enabled() {
    let api = MockServer::start().await;

    let writes_only = tool_names(config(&api, true, ConfirmationMode::Auto)).await;
    for t in ADMIN_TOOLS {
        assert!(
            !writes_only.contains(&t.to_string()),
            "{t} must be hidden without admin"
        );
    }
    // Read-only admin helpers are always present.
    assert!(writes_only.contains(&"list_backend_ids".to_string()));
    assert!(writes_only.contains(&"list_deleted_zones".to_string()));

    let admin = tool_names(admin_config(&api)).await;
    for t in ADMIN_TOOLS.iter().chain(WRITE_TOOLS) {
        assert!(admin.contains(&t.to_string()), "{t} should be available");
    }
}

#[tokio::test]
async fn update_zone_carries_over_connection_and_acl() {
    let api = MockServer::start().await;
    mock_zone(&api).await;
    // The PUT must keep the (already encrypted) connection key and the ACL rules,
    // without the display-only `displayName`.
    Mock::given(method("PUT"))
        .and(path("/zones/z1"))
        .and(body_json(json!({
            "id": "z1",
            "name": "example.com.",
            "email": "new@example.com",
            "adminGroupId": "g1",
            "shared": false,
            "connection": {
                "name": "example.com.", "keyName": "vinyldns.", "key": "1:ENCRYPTED==", "primaryServer": "10.0.0.53"
            },
            "acl": { "rules": [
                { "accessLevel": "Read", "groupId": "g2", "recordTypes": ["A"], "description": "ops" }
            ]}
        })))
        .and(signed)
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "zc-1", "changeType": "Update" })))
        .expect(1)
        .mount(&api)
        .await;

    let client = admin_client(&api).await;
    let plan = json_of(
        &call(
            &client,
            "plan_update_zone",
            json!({ "zone_id": "z1", "email": "new@example.com" }),
        )
        .await,
    );
    assert_eq!(plan["summary"], "Update zone example.com. (email)");
    assert_eq!(plan["preview"]["before"]["email"], "old@example.com");
    assert_eq!(plan["preview"]["after"]["email"], "new@example.com");
    assert_eq!(plan["preview"]["request"]["connection"]["key"], "<redacted>");
    assert!(
        !plan.to_string().contains("ENCRYPTED"),
        "secrets must not appear in previews"
    );

    let applied = json_of(&call(&client, "confirm_change", json!({ "token": plan["token"] })).await);
    assert_eq!(applied["result"]["id"], "zc-1");
}

#[tokio::test]
async fn update_zone_refuses_no_op() {
    let api = MockServer::start().await;
    mock_zone(&api).await;
    let client = admin_client(&api).await;
    let result = call(
        &client,
        "plan_update_zone",
        json!({ "zone_id": "z1", "email": "old@example.com" }),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("identical"));
}

#[tokio::test]
async fn delete_zone_requires_matching_name() {
    let api = MockServer::start().await;
    mock_zone(&api).await;
    Mock::given(method("GET"))
        .and(path("/zones/z1/recordsetcount"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "count": 12 })))
        .mount(&api)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/zones/z1"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "zc-del" })))
        .expect(1)
        .mount(&api)
        .await;

    let client = admin_client(&api).await;
    let wrong = call(
        &client,
        "plan_delete_zone",
        json!({ "zone_id": "z1", "confirm_zone_name": "example.org." }),
    )
    .await;
    assert_eq!(wrong.is_error, Some(true));
    assert!(text(&wrong).contains("does not match"));

    let plan = json_of(
        &call(
            &client,
            "plan_delete_zone",
            json!({ "zone_id": "z1", "confirm_zone_name": "Example.com" }),
        )
        .await,
    );
    assert_eq!(plan["summary"], "DELETE (abandon) zone example.com.");
    assert_eq!(plan["preview"]["record_set_count"], 12);
    let applied = json_of(&call(&client, "confirm_change", json!({ "token": plan["token"] })).await);
    assert_eq!(applied["result"]["id"], "zc-del");
}

#[tokio::test]
async fn connect_zone_validates_and_redacts_keys() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zones/name/new.example.com."))
        .respond_with(ResponseTemplate::new(404).set_body_string("Zone not found"))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/groups/g1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "g1", "name": "dns-admins" })))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/zones"))
        .and(body_partial_json(json!({
            "name": "new.example.com.",
            "adminGroupId": "g1",
            "connection": { "name": "new.example.com.", "keyName": "tsig.", "key": "s3cret", "algorithm": "HMAC-SHA256" }
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "zc-new" })))
        .expect(1)
        .mount(&api)
        .await;

    let client = admin_client(&api).await;
    let plan = json_of(
        &call(
            &client,
            "plan_connect_zone",
            json!({
                "name": "new.example.com",
                "email": "dns@example.com",
                "admin_group_id": "g1",
                "connection": {
                    "key_name": "tsig.", "key": "s3cret", "primary_server": "10.0.0.53", "algorithm": "HMAC-SHA256"
                }
            }),
        )
        .await,
    );
    assert_eq!(
        plan["summary"],
        "Connect zone new.example.com., administered by group dns-admins"
    );
    assert!(!plan.to_string().contains("s3cret"), "TSIG secret leaked into preview");

    let applied = json_of(&call(&client, "confirm_change", json!({ "token": plan["token"] })).await);
    assert_eq!(applied["result"]["id"], "zc-new");
}

#[tokio::test]
async fn connect_zone_rejects_already_connected_zone() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zones/name/example.com."))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "zone": zone() })))
        .mount(&api)
        .await;
    let client = admin_client(&api).await;
    let result = call(
        &client,
        "plan_connect_zone",
        json!({ "name": "example.com.", "email": "dns@example.com", "admin_group_id": "g1" }),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("already connected"));
}

#[tokio::test]
async fn approve_batch_change_sends_review_comment() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zones/batchrecordchanges/bc1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "bc1", "userName": "alice", "approvalStatus": "PendingReview", "status": "PendingReview",
            "changes": [{ "changeType": "Add", "inputName": "a.example.com.", "type": "A",
                          "record": { "address": "10.0.0.1" }, "status": "NeedsReview",
                          "validationErrors": [{ "errorType": "ZoneDiscoveryError" }] }]
        })))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/zones/batchrecordchanges/bc1/approve"))
        .and(body_json(json!({ "reviewComment": "looks good" })))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({ "id": "bc1", "approvalStatus": "ManuallyApproved" })),
        )
        .expect(1)
        .mount(&api)
        .await;

    let client = admin_client(&api).await;
    let plan = json_of(
        &call(
            &client,
            "plan_approve_batch_change",
            json!({ "id": "bc1", "review_comment": "looks good" }),
        )
        .await,
    );
    assert!(
        plan["summary"]
            .as_str()
            .unwrap()
            .starts_with("APPROVE batch change bc1 from alice")
    );
    assert_eq!(plan["preview"]["batch_change"]["changes"][0]["status"], "NeedsReview");
    let applied = json_of(&call(&client, "confirm_change", json!({ "token": plan["token"] })).await);
    assert_eq!(applied["result"]["approvalStatus"], "ManuallyApproved");
}

#[tokio::test]
async fn reject_refuses_batch_not_in_review() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zones/batchrecordchanges/bc2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": "bc2", "approvalStatus": "AutoApproved" })),
        )
        .mount(&api)
        .await;
    let client = admin_client(&api).await;
    let result = call(&client, "plan_reject_batch_change", json!({ "id": "bc2" })).await;
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("AutoApproved"));
}

#[tokio::test]
async fn delete_acl_rule_sends_the_stored_rule() {
    let api = MockServer::start().await;
    mock_zone(&api).await;
    // The description was not given, but the stored rule (with it) is sent,
    // because VinylDNS removes rules by exact equality.
    Mock::given(method("DELETE"))
        .and(path("/zones/z1/acl/rules"))
        .and(body_json(
            json!({ "accessLevel": "Read", "groupId": "g2", "recordTypes": ["A"], "description": "ops" }),
        ))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "zc-acl" })))
        .expect(1)
        .mount(&api)
        .await;

    let client = admin_client(&api).await;
    let missing = call(
        &client,
        "plan_delete_zone_acl_rule",
        json!({ "zone_id": "z1", "access_level": "Write", "group_id": "g2" }),
    )
    .await;
    assert_eq!(missing.is_error, Some(true));
    assert!(text(&missing).contains("Read for group g2 on A matching all records"));

    let plan = json_of(
        &call(
            &client,
            "plan_delete_zone_acl_rule",
            json!({ "zone_id": "z1", "access_level": "Read", "group_id": "g2", "record_types": ["a"] }),
        )
        .await,
    );
    let applied = json_of(&call(&client, "confirm_change", json!({ "token": plan["token"] })).await);
    assert_eq!(applied["result"]["id"], "zc-acl");
}

#[tokio::test]
async fn add_acl_rule_warns_when_rule_applies_to_everyone() {
    let api = MockServer::start().await;
    mock_zone(&api).await;
    Mock::given(method("PUT"))
        .and(path("/zones/z1/acl/rules"))
        .and(body_json(
            json!({ "accessLevel": "Read", "recordTypes": [], "recordMask": "^www.*" }),
        ))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "zc-add" })))
        .expect(1)
        .mount(&api)
        .await;

    let client = admin_client(&api).await;
    let plan = json_of(
        &call(
            &client,
            "plan_add_zone_acl_rule",
            json!({ "zone_id": "z1", "access_level": "Read", "record_mask": "^www.*" }),
        )
        .await,
    );
    assert!(
        plan["preview"]["warnings"][0]
            .as_str()
            .unwrap()
            .contains("ALL VinylDNS users")
    );
    let applied = json_of(&call(&client, "confirm_change", json!({ "token": plan["token"] })).await);
    assert_eq!(applied["result"]["id"], "zc-add");
}
