use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Deserialize `id` field: absent => None, explicit null => Some(Value::Null), number/string => Some(v).
fn deserialize_id<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    // Use Option<Value> with a custom visitor so that a present-null maps to Some(Null).
    let v: Value = Deserialize::deserialize(deserializer)?;
    Ok(Some(v))
}

#[derive(Debug, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default, deserialize_with = "deserialize_id")]
    pub id: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Response {
    pub fn success(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0", id, result: Some(result), error: None }
    }
    pub fn error(id: Value, code: i32, message: &str, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError { code, message: message.into(), data }),
        }
    }
}

// Reserved SecAFS-specific error codes (-32000..-32099 range).
pub const ALREADY_MOUNTED: i32 = -32000;
pub const NOT_MOUNTED: i32 = -32001;
pub const PG_UNAVAILABLE: i32 = -32002;
pub const FUSE_MOUNT_FAILED: i32 = -32003;
pub const CONVERSATION_NOT_FOUND: i32 = -32004;
pub const ROLLBACK_NOT_ENABLED: i32 = -32010;
pub const SNAPSHOT_NOT_FOUND: i32 = -32011;
