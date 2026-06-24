use super::types::{Request, Response};

pub fn parse_request(raw: &str) -> Result<Request, String> {
    let parsed: Request = serde_json::from_str(raw).map_err(|e| format!("parse: {e}"))?;
    if parsed.jsonrpc != "2.0" {
        return Err("jsonrpc != 2.0".into());
    }
    Ok(parsed)
}

pub fn encode_response(r: &Response) -> String {
    serde_json::to_string(r).expect("Response should always serialize")
}
