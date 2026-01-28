//! Agent protocol types
//!
//! JSON-based protocol for communication with the remote agent.
//! All binary data is base64 encoded.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};

/// Protocol version
pub const PROTOCOL_VERSION: u32 = 1;

/// Request sent to the agent
#[derive(Debug, Clone, Serialize)]
pub struct AgentRequest {
    pub id: u64,
    #[serde(rename = "m")]
    pub method: String,
    #[serde(rename = "p")]
    pub params: serde_json::Value,
}

impl AgentRequest {
    pub fn new(id: u64, method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            id,
            method: method.into(),
            params,
        }
    }

    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).unwrap() + "\n"
    }
}

/// Response from the agent - can be one of three types
#[derive(Debug, Clone, Deserialize)]
pub struct AgentResponse {
    pub id: u64,
    /// Streaming data (intermediate)
    #[serde(rename = "d")]
    pub data: Option<serde_json::Value>,
    /// Final result (success)
    #[serde(rename = "r")]
    pub result: Option<serde_json::Value>,
    /// Error message (failure)
    #[serde(rename = "e")]
    pub error: Option<String>,
    /// Ready message fields
    pub ok: Option<bool>,
    #[serde(rename = "v")]
    pub version: Option<u32>,
}

impl AgentResponse {
    /// Check if this is the ready message
    pub fn is_ready(&self) -> bool {
        self.ok == Some(true)
    }

    /// Check if this is a streaming data message
    pub fn is_data(&self) -> bool {
        self.data.is_some()
    }

    /// Check if this is a final message (result or error)
    pub fn is_final(&self) -> bool {
        self.result.is_some() || self.error.is_some()
    }
}

/// Directory entry returned by `ls` command
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct RemoteDirEntry {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub dir: bool,
    #[serde(default)]
    pub file: bool,
    #[serde(default)]
    pub link: bool,
    #[serde(default)]
    pub link_dir: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mtime: i64,
    #[serde(default)]
    pub mode: u32,
}

/// File metadata returned by `stat` command
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct RemoteMetadata {
    pub size: u64,
    pub mtime: i64,
    pub mode: u32,
    #[serde(default)]
    pub uid: u32,
    #[serde(default)]
    pub gid: u32,
    #[serde(default)]
    pub dir: bool,
    #[serde(default)]
    pub file: bool,
    #[serde(default)]
    pub link: bool,
}

/// Process execution result
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ExecResult {
    pub code: i32,
}

/// Streaming output from exec
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ExecOutput {
    #[serde(default)]
    pub out: Option<String>,
    #[serde(default)]
    pub err: Option<String>,
}

/// Helper to encode bytes to base64
pub fn encode_base64(data: &[u8]) -> String {
    BASE64.encode(data)
}

/// Helper to decode base64 to bytes
pub fn decode_base64(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    BASE64.decode(s)
}

/// Build params for read request
pub fn read_params(path: &str, offset: Option<u64>, len: Option<usize>) -> serde_json::Value {
    let mut params = serde_json::json!({"path": path});
    if let Some(off) = offset {
        params["off"] = serde_json::json!(off);
    }
    if let Some(l) = len {
        params["len"] = serde_json::json!(l);
    }
    params
}

/// Build params for write request
pub fn write_params(path: &str, data: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "data": encode_base64(data)
    })
}

/// Build params for sudo_write request (write file as root)
pub fn sudo_write_params(
    path: &str,
    data: &[u8],
    mode: u32,
    uid: u32,
    gid: u32,
) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "data": encode_base64(data),
        "mode": mode,
        "uid": uid,
        "gid": gid
    })
}

/// Build params for stat request
pub fn stat_params(path: &str, follow_symlinks: bool) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "link": follow_symlinks
    })
}

/// Build params for ls request
pub fn ls_params(path: &str) -> serde_json::Value {
    serde_json::json!({"path": path})
}

/// Build params for exec request
///
/// NOTE: Used by RemoteProcessSpawner, appears unused until spawner integration.
pub fn exec_params(cmd: &str, args: &[String], cwd: Option<&str>) -> serde_json::Value {
    let mut params = serde_json::json!({
        "cmd": cmd,
        "args": args
    });
    if let Some(dir) = cwd {
        params["cwd"] = serde_json::json!(dir);
    }
    params
}

/// Build params for cancel request
pub fn cancel_params(request_id: u64) -> serde_json::Value {
    serde_json::json!({"id": request_id})
}

/// Build params for append request
pub fn append_params(path: &str, data: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "data": encode_base64(data)
    })
}

/// Build params for truncate request
pub fn truncate_params(path: &str, len: u64) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "len": len
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let req = AgentRequest::new(1, "read", serde_json::json!({"path": "/test.txt"}));
        let json = req.to_json_line();
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"m\":\"read\""));
        assert!(json.contains("\"p\":{\"path\":\"/test.txt\"}"));
    }

    #[test]
    fn test_response_parsing() {
        let ready = r#"{"id":0,"ok":true,"v":1}"#;
        let resp: AgentResponse = serde_json::from_str(ready).unwrap();
        assert!(resp.is_ready());
        assert_eq!(resp.version, Some(1));

        let data = r#"{"id":1,"d":{"data":"SGVsbG8="}}"#;
        let resp: AgentResponse = serde_json::from_str(data).unwrap();
        assert!(resp.is_data());
        assert!(!resp.is_final());

        let result = r#"{"id":1,"r":{"size":5}}"#;
        let resp: AgentResponse = serde_json::from_str(result).unwrap();
        assert!(resp.is_final());
        assert!(resp.result.is_some());

        let error = r#"{"id":1,"e":"not found"}"#;
        let resp: AgentResponse = serde_json::from_str(error).unwrap();
        assert!(resp.is_final());
        assert_eq!(resp.error, Some("not found".to_string()));
    }

    #[test]
    fn test_base64_roundtrip() {
        let data = b"Hello, World!";
        let encoded = encode_base64(data);
        let decoded = decode_base64(&encoded).unwrap();
        assert_eq!(data.as_slice(), decoded.as_slice());
    }
}
