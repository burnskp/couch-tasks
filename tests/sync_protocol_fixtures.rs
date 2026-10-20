use couch_tasks::sync_protocol::{SyncRequest, SyncResponse};

#[test]
fn sync_request_fixture_matches_rust_protocol() {
    let fixture = include_str!("fixtures/sync/request.json");
    let request: SyncRequest = serde_json::from_str(fixture).expect("request fixture parses");

    assert_eq!(request.checkpoint, 7);
    assert_eq!(request.projects.len(), 2);
    assert_eq!(request.tasks.len(), 2);
    assert_eq!(request.tombstones.len(), 1);

    let expected: serde_json::Value = serde_json::from_str(fixture).expect("fixture is json");
    let encoded = serde_json::to_value(request).expect("request serializes");
    assert_eq!(encoded, expected);
}

#[test]
fn sync_response_fixture_matches_rust_protocol() {
    let fixture = include_str!("fixtures/sync/response.json");
    let response: SyncResponse = serde_json::from_str(fixture).expect("response fixture parses");

    assert_eq!(response.checkpoint, 9);
    assert_eq!(response.projects.len(), 2);
    assert_eq!(response.tasks.len(), 1);
    assert_eq!(response.tombstones.len(), 2);

    let expected: serde_json::Value = serde_json::from_str(fixture).expect("fixture is json");
    let encoded = serde_json::to_value(response).expect("response serializes");
    assert_eq!(encoded, expected);
}
