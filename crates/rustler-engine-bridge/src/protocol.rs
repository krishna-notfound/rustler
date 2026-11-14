//! JSON-RPC message types for the engine protocol (see `../PROTOCOL.md` and `engine/PROTOCOL.md`).
//!
//! Engine error kinds map: `dependency_missing` -> DependencyMissing, `media` -> Media,
//! `validation` -> Validation, other -> Internal. The standard JSON-RPC protocol errors
//! (-32700/-32600/-32601) carry `data.kind = "validation"` and map the same way; an error
//! without a usable kind falls back to the code (-32600/-32601/-32602/-32700 -> Validation).

use rustler_core::{ErrorCode, RustlerError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stage name used on every error produced by the bridge.
pub const STAGE: &str = "engine";

#[derive(Serialize, Debug)]
pub struct Request<'a> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'a str,
    pub params: &'a Value,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub data: Option<RpcErrorData>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
pub struct RpcErrorData {
    #[serde(default)]
    pub kind: Option<String>,
}

/// `params` of a `progress` notification. `pct` is 0..=100 on the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub id: Option<u64>,
    pub stage: String,
    pub pct: Option<f64>,
    pub message: Option<String>,
}

/// One parsed line from the engine's stdout.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Response {
        id: Option<u64>,
        outcome: Result<Value, RpcError>,
    },
    Progress(Progress),
    /// Valid JSON that is neither a response nor a progress notification.
    Other(Value),
}

fn id_of(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn parse_line(line: &str) -> Result<Incoming, serde_json::Error> {
    let v: Value = serde_json::from_str(line)?;
    if v.get("method").and_then(Value::as_str) == Some("progress") {
        let p = v.get("params").cloned().unwrap_or(Value::Null);
        return Ok(Incoming::Progress(Progress {
            id: id_of(p.get("id")),
            stage: p
                .get("stage")
                .and_then(Value::as_str)
                .unwrap_or("engine")
                .to_string(),
            pct: p.get("pct").and_then(Value::as_f64),
            message: p.get("message").and_then(Value::as_str).map(str::to_string),
        }));
    }
    if v.get("result").is_some() || v.get("error").is_some() {
        let id = id_of(v.get("id"));
        let outcome = match v.get("error") {
            Some(e) if !e.is_null() => Err(serde_json::from_value::<RpcError>(e.clone())
                .unwrap_or(RpcError {
                    code: -32000,
                    message: e.to_string(),
                    data: None,
                })),
            _ => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
        };
        return Ok(Incoming::Response { id, outcome });
    }
    Ok(Incoming::Other(v))
}

/// Maps an engine-reported error to the shared error type.
pub fn map_error(method: &str, e: RpcError) -> RustlerError {
    let kind = e.data.as_ref().and_then(|d| d.kind.as_deref());
    let code = match kind {
        Some("dependency_missing") => ErrorCode::DependencyMissing,
        Some("media") => ErrorCode::Media,
        Some("validation") => ErrorCode::Validation,
        Some(_) => ErrorCode::Internal,
        None => match e.code {
            -32700 | -32600 | -32601 | -32602 => ErrorCode::Validation,
            _ => ErrorCode::Internal,
        },
    };
    let mut err = RustlerError::new(code, STAGE, format!("{method}: {}", e.message));
    if code == ErrorCode::DependencyMissing {
        err = err.hint("run `rustler setup`");
    }
    err
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_success_error_progress() {
        match parse_line(r#"{"jsonrpc":"2.0","id":3,"result":{"a":1}}"#).unwrap() {
            Incoming::Response { id, outcome } => {
                assert_eq!(id, Some(3));
                assert_eq!(outcome.unwrap(), json!({"a":1}));
            }
            o => panic!("{o:?}"),
        }
        match parse_line(
            r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32601,"message":"nope","data":{"kind":"validation"}}}"#,
        )
        .unwrap()
        {
            Incoming::Response {
                outcome: Err(e), ..
            } => {
                assert_eq!(map_error("x", e).code, ErrorCode::Validation)
            }
            o => panic!("{o:?}"),
        }
        match parse_line(
            r#"{"jsonrpc":"2.0","method":"progress","params":{"id":5,"stage":"transcribe","pct":50.0,"message":"hi"}}"#,
        )
        .unwrap()
        {
            Incoming::Progress(p) => {
                assert_eq!(p.id, Some(5));
                assert_eq!(p.pct, Some(50.0));
                assert_eq!(p.message.as_deref(), Some("hi"));
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn error_kind_mapping() {
        let mk = |kind: Option<&str>, code| RpcError {
            code,
            message: "m".into(),
            data: kind.map(|k| RpcErrorData {
                kind: Some(k.into()),
            }),
        };
        assert_eq!(
            map_error("m", mk(Some("dependency_missing"), -32000)).code,
            ErrorCode::DependencyMissing
        );
        assert_eq!(
            map_error("m", mk(Some("media"), -32000)).code,
            ErrorCode::Media
        );
        assert_eq!(
            map_error("m", mk(Some("internal"), -32000)).code,
            ErrorCode::Internal
        );
        assert_eq!(map_error("m", mk(None, -32601)).code, ErrorCode::Validation);
    }
}
