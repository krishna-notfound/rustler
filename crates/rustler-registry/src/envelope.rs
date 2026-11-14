//! The single JSON envelope printed on stdout per invocation.

use rustler_core::RustlerError;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub enum Envelope {
    Ok {
        schema: String,
        data: Value,
        warnings: Vec<String>,
    },
    Err {
        error: RustlerError,
    },
}

impl Envelope {
    /// `schema = "rustler.<command>@1"`. A top-level `"warnings": [..]` array in `data`
    /// is moved to the envelope level.
    pub fn ok(command: &str, mut data: Value) -> Self {
        let mut warnings = Vec::new();
        if let Value::Object(map) = &mut data {
            if matches!(map.get("warnings"), Some(Value::Array(_))) {
                if let Some(Value::Array(items)) = map.remove("warnings") {
                    warnings = items
                        .into_iter()
                        .map(|w| match w {
                            Value::String(s) => s,
                            other => other.to_string(),
                        })
                        .collect();
                }
            }
        }
        Envelope::Ok {
            schema: format!("rustler.{command}@1"),
            data,
            warnings,
        }
    }

    pub fn err(error: RustlerError) -> Self {
        Envelope::Err { error }
    }

    /// `{"ok":true,"schema":…,"data":…,"warnings":[…]}` or `{"ok":false,"error":{…}}`.
    pub fn to_value(&self) -> Value {
        match self {
            Envelope::Ok {
                schema,
                data,
                warnings,
            } => {
                json!({ "ok": true, "schema": schema, "data": data, "warnings": warnings })
            }
            Envelope::Err { error } => json!({ "ok": false, "error": error }),
        }
    }

    /// Compact single-line JSON.
    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }

    /// 0 for Ok, else the error code's exit code.
    pub fn exit_code(&self) -> i32 {
        match self {
            Envelope::Ok { .. } => 0,
            Envelope::Err { error } => error.code.exit_code(),
        }
    }
}
