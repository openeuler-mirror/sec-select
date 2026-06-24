use super::codec::{encode_response, parse_request};
use super::methods::{dispatch, State};
use super::types::*;
use serde_json::json;

#[test]
fn parses_mount_request() {
    let raw = r#"{"jsonrpc":"2.0","method":"secafs.v1.mount","params":{"conversationId":"a1","hostPath":"/tmp/x"},"id":1}"#;
    let req = parse_request(raw).unwrap();
    assert_eq!(req.method, "secafs.v1.mount");
    assert_eq!(req.id, Some(json!(1)));
    assert_eq!(req.params["conversationId"], json!("a1"));
}

#[test]
fn encodes_success_response() {
    let r = Response::success(json!(1), json!({"hostPath":"/tmp/x","mounted":true}));
    let s = encode_response(&r);
    assert!(s.contains("\"result\""));
    assert!(s.contains("\"mounted\":true"));
    assert!(!s.contains("\"error\""));
}

#[test]
fn encodes_typed_error() {
    let r = Response::error(json!(2), ALREADY_MOUNTED, "ALREADY_MOUNTED", None);
    let s = encode_response(&r);
    assert!(s.contains("-32000"));
    assert!(s.contains("ALREADY_MOUNTED"));
    assert!(!s.contains("\"result\""));
}

#[test]
fn rejects_missing_jsonrpc_field() {
    let raw = r#"{"method":"secafs.v1.ping","id":1}"#;
    let err = parse_request(raw).unwrap_err();
    assert!(err.contains("parse"));
}

#[test]
fn rejects_wrong_jsonrpc_version() {
    let raw = r#"{"jsonrpc":"1.0","method":"x","id":1}"#;
    let err = parse_request(raw).unwrap_err();
    assert!(err.contains("jsonrpc"));
}

#[test]
fn request_with_null_id_parses() {
    let raw = r#"{"jsonrpc":"2.0","method":"x","id":null}"#;
    let req = parse_request(raw).unwrap();
    assert_eq!(req.id, Some(json!(null)));
}

#[tokio::test]
async fn ping_probes_pg_connectivity_live() {
    let state = State::with_fake_mount();
    let out = dispatch(&state, "secafs.v1.ping", json!({})).await.unwrap();
    assert!(out["version"].is_string());
    // pgConnected is a live probe via backend.get_connection(); the fake
    // backend has no database, so it must report false (the old behavior —
    // a startup-time flag frozen to true — masked dead connections).
    assert_eq!(out["pgConnected"], false);
    assert_eq!(out["mountCount"], 0);
}

#[tokio::test]
async fn mount_is_idempotent() {
    let state = State::with_fake_mount();
    let p1 = dispatch(&state, "secafs.v1.mount", json!({"conversationId":"a"})).await.unwrap();
    let p2 = dispatch(&state, "secafs.v1.mount", json!({"conversationId":"a"})).await.unwrap();
    assert_eq!(p1["hostPath"], p2["hostPath"]);
    assert_eq!(p2["mounted"], true);
}

#[tokio::test]
async fn unmount_unmounted_is_noop() {
    let state = State::with_fake_mount();
    let out = dispatch(&state, "secafs.v1.unmount", json!({"conversationId":"ghost"})).await.unwrap();
    assert_eq!(out["unmounted"], false);
}

#[tokio::test]
async fn list_returns_mounts_after_mount() {
    let state = State::with_fake_mount();
    dispatch(&state, "secafs.v1.mount", json!({"conversationId":"a"})).await.unwrap();
    dispatch(&state, "secafs.v1.mount", json!({"conversationId":"b"})).await.unwrap();
    let out = dispatch(&state, "secafs.v1.list", json!({})).await.unwrap();
    assert_eq!(out["mounts"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn destroy_removes_mount_and_calls_backend() {
    let state = State::with_fake_mount();
    dispatch(&state, "secafs.v1.mount", json!({"conversationId":"a"})).await.unwrap();
    let out = dispatch(&state, "secafs.v1.destroy", json!({"conversationId":"a"})).await.unwrap();
    assert_eq!(out["destroyed"], true);
    let list = dispatch(&state, "secafs.v1.list", json!({})).await.unwrap();
    assert_eq!(list["mounts"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn unknown_method_returns_minus_32601() {
    let state = State::with_fake_mount();
    let err = dispatch(&state, "secafs.v1.bogus", json!({})).await.unwrap_err();
    assert_eq!(err.0, -32601);
}

#[tokio::test]
async fn mount_without_conversation_id_returns_invalid_params() {
    let state = State::with_fake_mount();
    let err = dispatch(&state, "secafs.v1.mount", json!({})).await.unwrap_err();
    assert_eq!(err.0, -32602);
}
